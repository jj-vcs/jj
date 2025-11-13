// Copyright 2025 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#![expect(missing_docs)]

use std::collections::HashMap;
use std::fs::File;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use gix_attributes::Search;
use gix_attributes::State;
use gix_attributes::glob::pattern::Case;
use gix_attributes::search::MetadataCollection;
use gix_attributes::search::Outcome;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt as _;
use tokio::sync::OnceCell;

use crate::backend::TreeValue;
use crate::file_util::BlockingAsyncReader;
use crate::merge::SameChange;
use crate::merged_tree::MergedTree;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::repo_path::RepoPathComponent;

/// Git Attributes instance
///
/// This struct handles accessing .gitattributes files
/// and maintains a cache to prevent loading the same file twice.
///
/// It's lazy loaded so .gitattributes files are only accessed
/// if you search for attributes for a given path.
pub(crate) struct GitAttributes {
    disk_file_loader: Arc<dyn FileLoader>,
    store_file_loader: Arc<dyn FileLoader>,
    node_cache: Mutex<HashMap<RepoPathBuf, Arc<GitAttributesNode>>>,
}

impl std::fmt::Debug for GitAttributes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitAttributes").finish()
    }
}

#[async_trait::async_trait]
pub(crate) trait FileLoader: Send + Sync {
    /// Loads a file in a given `path`
    ///
    /// Returns Some(..) if the file was found and None if not
    async fn load(
        &self,
        path: &RepoPath,
    ) -> Result<Option<Box<dyn AsyncRead + Send + Unpin>>, GitAttributesError>;
}

pub(crate) struct TreeFileLoader {
    tree: MergedTree,
}
impl TreeFileLoader {
    pub fn new(tree: MergedTree) -> Self {
        Self { tree }
    }
}

#[async_trait::async_trait]
impl FileLoader for TreeFileLoader {
    async fn load(
        &self,
        path: &RepoPath,
    ) -> Result<Option<Box<dyn AsyncRead + Send + Unpin>>, GitAttributesError> {
        let merged_tree_value =
            self.tree
                .path_value_async(path)
                .await
                .map_err(|err| GitAttributesError {
                    message: "Could not retrieve the value from path".to_string(),
                    source: err.into(),
                })?;
        let maybe_file_merge = merged_tree_value.to_file_merge();
        // try to resolve the file
        let id = match maybe_file_merge
            .as_ref()
            .and_then(|files| files.resolve_trivial(SameChange::Accept))
        {
            Some(Some(id)) => id,
            Some(None) => return Ok(None),
            None => {
                // conflict path
                let Some(id) = merged_tree_value.iter().find_map(|tree_value| {
                    let Some(TreeValue::File { id, .. }) = tree_value else {
                        return None;
                    };
                    Some(id)
                }) else {
                    return Ok(None);
                };
                id
            }
        };

        let result =
            self.tree
                .store()
                .read_file(path, id)
                .await
                .map_err(|err| GitAttributesError {
                    message: "Could not retrieve the value from path".to_string(),
                    source: err.into(),
                })?;
        Ok(Some(Box::new(result)))
    }
}

pub(crate) struct DiskFileLoader {
    repo_root: PathBuf,
}

impl DiskFileLoader {
    pub fn new(repo_root: PathBuf) -> Self {
        Self { repo_root }
    }
}

#[async_trait::async_trait]
impl FileLoader for DiskFileLoader {
    async fn load(
        &self,
        path: &RepoPath,
    ) -> Result<Option<Box<dyn AsyncRead + Send + Unpin>>, GitAttributesError> {
        let path = path
            .to_fs_path(&self.repo_root)
            .map_err(|err| GitAttributesError {
                message: "Could not convert path into fs path".to_string(),
                source: err.into(),
            })?;

        // we use std::fs::symlink_metadata to not follow symlinks to follow Git's
        // behavior.
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(GitAttributesError {
                    message: format!("Failed to obtain the file metadata of {}", path.display()),
                    source: err.into(),
                });
            }
        };
        if !metadata.is_file() {
            return Ok(None);
        }

        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(GitAttributesError {
                    message: format!("Failed to open the file at {}", path.display()),
                    source: err.into(),
                });
            }
        };
        Ok(Some(Box::new(BlockingAsyncReader::new(file))))
    }
}

struct SearchAndCollection {
    search: Search,
    collection: MetadataCollection,
}

struct GitAttributesNode {
    disk_first: OnceCell<Arc<SearchAndCollection>>,
    store_first: OnceCell<Arc<SearchAndCollection>>,
    disk_file_loader: Arc<dyn FileLoader>,
    store_file_loader: Arc<dyn FileLoader>,
    parent: Option<Arc<GitAttributesNode>>,
    path: RepoPathBuf,
}

impl GitAttributesNode {
    async fn get_disk_first(&self) -> Result<Arc<SearchAndCollection>, GitAttributesError> {
        self.primary_then_secondary(SearchPriority::Disk).await
    }
    async fn get_store_first(&self) -> Result<Arc<SearchAndCollection>, GitAttributesError> {
        self.primary_then_secondary(SearchPriority::Store).await
    }

    fn store(&self, priority: SearchPriority) -> &OnceCell<Arc<SearchAndCollection>> {
        match priority {
            SearchPriority::Store => &self.store_first,
            SearchPriority::Disk => &self.disk_first,
        }
    }

    fn primary_loader(&self, priority: SearchPriority) -> &Arc<dyn FileLoader> {
        match priority {
            SearchPriority::Store => &self.store_file_loader,
            SearchPriority::Disk => &self.disk_file_loader,
        }
    }

    fn secondary_loader(&self, priority: SearchPriority) -> &Arc<dyn FileLoader> {
        match priority {
            SearchPriority::Store => &self.disk_file_loader,
            SearchPriority::Disk => &self.store_file_loader,
        }
    }

    async fn primary_then_secondary(
        &self,
        priority: SearchPriority,
    ) -> Result<Arc<SearchAndCollection>, GitAttributesError> {
        // we use pin because this is a recursive call
        self.store(priority)
            .get_or_try_init(async || {
                let parent_metadata = match &self.parent {
                    Some(parent) => Box::pin(parent.primary_then_secondary(priority)).await?,
                    None => {
                        let mut search = Search::default();
                        let mut collection = MetadataCollection::default();
                        // initialize search
                        search.add_patterns_buffer(
                            b"[attr]binary -diff -merge -text",
                            "[builtin]".into(),
                            None,
                            &mut collection,
                            true, /* allow macros */
                        );
                        Arc::new(SearchAndCollection { search, collection })
                    }
                };
                let git_attributes_path =
                    self.path
                        .join(RepoPathComponent::new(".gitattributes").map_err(|err| {
                            GitAttributesError {
                                message: "Could not join path with .gitattributes".to_string(),
                                source: err.into(),
                            }
                        })?);
                let mut async_reader = match self
                    .primary_loader(priority)
                    .load(&git_attributes_path)
                    .await?
                {
                    Some(reader) => reader,
                    None => {
                        // fallback to the secondary loader
                        match self
                            .secondary_loader(priority)
                            .load(&git_attributes_path)
                            .await?
                        {
                            Some(reader) => reader,
                            None => return Ok(parent_metadata),
                        }
                    }
                };
                let mut bytes = Vec::new();
                async_reader
                    .read_to_end(&mut bytes)
                    .await
                    .map_err(|err| GitAttributesError {
                        source: err.into(),
                        message: "Could not read .gitattributes file".into(),
                    })?;
                let mut search = parent_metadata.search.clone();
                let mut collection = parent_metadata.collection.clone();

                search.add_patterns_buffer(
                    &bytes,
                    git_attributes_path
                        .to_fs_path(&PathBuf::new())
                        .map_err(|err| GitAttributesError {
                            message: "Could not convert gitattributes path into PathBuf"
                                .to_string(),
                            source: err.into(),
                        })?,
                    Some(&PathBuf::new()),
                    &mut collection,
                    // Macros can only be defined in top-level gitattributes:
                    // https://git-scm.com/docs/gitattributes#_defining_macro_attributes
                    self.parent.is_none(), /* allow macros */
                );

                Ok(Arc::new(SearchAndCollection { search, collection }))
            })
            .await
            .cloned()
    }
}

#[derive(Clone, Copy)]
pub(crate) enum SearchPriority {
    Store,
    Disk,
}

impl GitAttributes {
    /// Creates a new instance by receiving a Store File Loader, a Disk File
    /// Loader and a list of filters to be ignored.
    pub fn new(
        store_file_loader: impl FileLoader + 'static,
        disk_file_loader: impl FileLoader + 'static,
    ) -> Self {
        Self {
            store_file_loader: Arc::new(store_file_loader),
            disk_file_loader: Arc::new(disk_file_loader),
            node_cache: Default::default(),
        }
    }

    pub(crate) async fn search(
        &self,
        path: &RepoPath,
        attribute_names: impl AsRef<[&str]>,
        priority: SearchPriority,
    ) -> Result<HashMap<String, State>, GitAttributesError> {
        let attributes = self.get_git_attributes_node(path);
        let store = match priority {
            SearchPriority::Store => attributes.get_store_first().await?,
            SearchPriority::Disk => attributes.get_disk_first().await?,
        };
        let SearchAndCollection { search, collection } = &*store;

        let mut out = Outcome::default();
        out.initialize_with_selection(
            collection,
            // From<&str> is implemented for KStringRef
            attribute_names.as_ref().iter().copied(),
        );
        search.pattern_matching_relative_path(
            path.as_internal_file_string().into(),
            Case::Sensitive,
            None,
            &mut out,
        );

        let mut map = out
            .iter_selected()
            .map(|attr| {
                let val = attr.assignment.to_owned();
                (val.name.as_str().to_string(), val.state)
            })
            .collect::<HashMap<String, State>>();

        // go over attributes and mark as unspecified if not set
        for attribute_name in attribute_names.as_ref() {
            map.entry(attribute_name.to_string())
                .or_insert(State::Unspecified);
        }

        Ok(map)
    }

    fn get_git_attributes_node(&self, path: &RepoPath) -> Arc<GitAttributesNode> {
        let path = path.parent().unwrap_or(RepoPath::root());
        let mut val = self.node_cache.lock().expect("Not be poisoned");
        self.inner(&mut val, path)
    }

    fn inner(
        &self,
        map: &mut HashMap<RepoPathBuf, Arc<GitAttributesNode>>,
        path: &RepoPath,
    ) -> Arc<GitAttributesNode> {
        if let Some(node) = map.get(path) {
            return node.clone();
        }
        let parent = path.parent().map(|parent| self.inner(map, parent));
        let new_node = Arc::new(GitAttributesNode {
            disk_first: OnceCell::new(),
            store_first: OnceCell::new(),
            disk_file_loader: self.disk_file_loader.clone(),
            store_file_loader: self.store_file_loader.clone(),
            parent,
            path: path.to_owned(),
        });
        map.insert(path.to_owned(), new_node.clone());
        new_node
    }
}

/// Extension trait to access filter_matches in git attributes file
pub(crate) trait GitAttributesFilter {
    /// Matches if the given `path` contains the `filter` value
    /// in .gitattributes and if the value is contained in
    /// the provided list of ignored filters
    fn filter_matches(
        &self,
        path: &RepoPath,
        ignore_filters: &[String],
        priority: SearchPriority,
    ) -> impl Future<Output = bool>;
}

impl GitAttributesFilter for GitAttributes {
    async fn filter_matches(
        &self,
        path: &RepoPath,
        ignore_filters: &[String],
        priority: SearchPriority,
    ) -> bool {
        let Ok(result) = self.search(path, ["filter"], priority).await else {
            return false;
        };

        let Some(State::Value(value)) = result.get("filter") else {
            return false;
        };
        let value = value.as_ref().as_bstr();
        ignore_filters.iter().any(|state| value == state)
    }
}

/// Errors for GitAttributes
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct GitAttributesError {
    message: String,
    #[source]
    source: Box<dyn std::error::Error + Send + Sync>,
}

#[cfg(test)]
mod tests {

    use std::io::Cursor;

    use gix_attributes::state::Value;
    use indoc::indoc;
    use pollster::FutureExt as _;

    use super::*;

    type TestStore = HashMap<RepoPathBuf, Result<String, String>>;

    #[async_trait::async_trait]
    impl FileLoader for TestStore {
        async fn load(
            &self,
            path: &RepoPath,
        ) -> Result<Option<Box<dyn AsyncRead + Send + Unpin>>, GitAttributesError> {
            let Some(mocked_result) = self.get(path) else {
                return Ok(None);
            };
            let data = mocked_result.clone();
            match data {
                Ok(data) => Ok(Some(Box::new(Cursor::new(Vec::from(data))))),
                Err(message) => Err(GitAttributesError {
                    message: message.clone(),
                    source: message.into(),
                }),
            }
        }
    }

    fn repo_path(path: &str) -> &RepoPath {
        RepoPath::from_internal_string(path).unwrap()
    }

    fn create_git_attributes(files: &[(&'static str, &'static str)]) -> GitAttributes {
        let data: TestStore = files
            .iter()
            .map(|(key, value)| (repo_path(key).to_owned(), Ok(value.to_string())))
            .collect();

        GitAttributes::new(HashMap::new(), data)
    }

    fn assert_search_output(
        git_attributes: &GitAttributes,
        file: &str,
        attribute: &str,
        expected: State,
    ) {
        assert_search_output_with_priority(
            git_attributes,
            file,
            attribute,
            expected,
            SearchPriority::Disk,
        );
    }

    fn assert_search_output_with_priority(
        git_attributes: &GitAttributes,
        file: &str,
        attribute: &str,
        expected: State,
        search: SearchPriority,
    ) {
        let map = git_attributes
            .search(repo_path(file), &[attribute], search)
            .block_on()
            .unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(*map.get(attribute).unwrap(), expected);
    }

    fn assert_attribute_output(
        git_attributes_content: &'static str,
        file: &str,
        attribute: &str,
        expected: State,
    ) {
        let git_attributes = create_git_attributes(&[(".gitattributes", git_attributes_content)]);
        assert_search_output(&git_attributes, file, attribute, expected);
    }

    #[test]
    fn test_search_state_set() {
        assert_attribute_output("abc foo", "abc", "foo", State::Set);
    }

    #[test]
    fn test_search_state_eol_lf() {
        assert_attribute_output("foo eol=lf", "foo", "eol", State::Value(Value::from("lf")));
    }

    #[test]
    fn test_search_unset() {
        assert_attribute_output("foo -text", "foo", "text", State::Unset);
    }

    #[test]
    fn test_search_unspecified() {
        assert_attribute_output("foo !text", "foo", "text", State::Unspecified);
    }

    #[test]
    fn test_search_unspecified_no_pattern() {
        assert_attribute_output("foo elo=lf", "bar", "text", State::Unspecified);
        assert_attribute_output("foo elo=lf", "bar", "elo", State::Unspecified);
    }

    #[test]
    fn test_path_and_pattern_matching() {
        // Using https://git-scm.com/docs/gitattributes#_examples as example
        // It's not 1:1 because we don't support $GIT_DIR/info/.gitattributes
        // that would override in-tree settings.
        let git_attributes = create_git_attributes(&[
            (".gitattributes", "abc	foo bar baz"),
            (
                "t/.gitattributes",
                indoc! {"
                    ab*	merge=filfre
                    abc	-foo -bar
                    *.c	frotz
                "},
            ),
        ]);

        assert_search_output(&git_attributes, "t/abc", "foo", State::Unset);
        assert_search_output(&git_attributes, "t/abc", "bar", State::Unset);
        assert_search_output(&git_attributes, "t/abc", "baz", State::Set);
        assert_search_output(
            &git_attributes,
            "t/abc",
            "merge",
            State::Value(Value::from("filfre")),
        );
        assert_search_output(&git_attributes, "t/abc", "frotz", State::Unspecified);
    }

    #[test]
    fn test_glob_matching() {
        assert_attribute_output("*.txt text", "bar.txt", "text", State::Set);
        assert_attribute_output("foo/ text", "foo/bar.txt", "text", State::Unspecified);
        assert_attribute_output("**/bar.rs text", "baz/bar.rs", "text", State::Set);
    }

    #[test]
    fn test_case_sensitive_attr() {
        assert_attribute_output(
            indoc! {"
                foo text
                FOO -text
            "},
            "foo",
            "text",
            State::Set,
        );
        assert_attribute_output(
            indoc! {"
                foo text
                FOO -text
            "},
            "FOO",
            "text",
            State::Unset,
        );
    }

    #[test]
    fn test_subdirectory_override() {
        let git_attributes = create_git_attributes(&[
            (".gitattributes", "abc	!baz"),
            ("t/.gitattributes", "abc baz"),
        ]);
        assert_search_output(&git_attributes, "t/abc", "baz", State::Set);
    }

    #[test]
    fn test_subdirectory_root_match() {
        assert_attribute_output("baz/foo text", "baz/foo", "text", State::Set);
    }

    #[test]
    fn test_directory_shouldnt_match() {
        assert_attribute_output("foo text", "foo/bar.txt", "text", State::Unspecified);
    }

    #[test]
    fn test_subdirectory_attribute() {
        let git_attributes = create_git_attributes(&[("foo/.gitattributes", "bar.txt text")]);
        assert_search_output(&git_attributes, "foo/bar.txt", "text", State::Set);
    }

    #[test]
    fn test_macro_definition() {
        let git_attributes = create_git_attributes(&[
            (
                ".gitattributes",
                indoc! {"
                    [attr]base_macro a
                    [attr]override_macro b
                "},
            ),
            (
                "foo1/.gitattributes",
                indoc! {"
                    # this macro definition shouldn't take effect.
                    [attr]override_macro c
                    # this macro definition shouldn't take effect.
                    [attr]new_macro e
                    bar base_macro override_macro new_macro 
                "},
            ),
            (
                "foo2/.gitattributes",
                indoc! {"
                    # this macro definition shouldn't take effect.
                    [attr]override_macro d
                    # this macro definition shouldn't take effect.
                    [attr]new_macro e
                    bar base_macro override_macro new_macro
                "},
            ),
        ]);
        assert_search_output(&git_attributes, "foo1/bar", "a", State::Set);
        assert_search_output(&git_attributes, "foo1/bar", "b", State::Set);
        assert_search_output(&git_attributes, "foo1/bar", "c", State::Unspecified);
        assert_search_output(&git_attributes, "foo1/bar", "d", State::Unspecified);
        assert_search_output(&git_attributes, "foo1/bar", "e", State::Unspecified);
        assert_search_output(&git_attributes, "foo1/bar", "new_macro", State::Set);

        assert_search_output(&git_attributes, "foo2/bar", "a", State::Set);
        assert_search_output(&git_attributes, "foo2/bar", "b", State::Set);
        assert_search_output(&git_attributes, "foo2/bar", "c", State::Unspecified);
        assert_search_output(&git_attributes, "foo2/bar", "d", State::Unspecified);
        assert_search_output(&git_attributes, "foo2/bar", "e", State::Unspecified);
        assert_search_output(&git_attributes, "foo2/bar", "new_macro", State::Set);
    }

    #[test]
    fn test_search_priority_and_fallback() {
        let disk = TestStore::from([(
            repo_path("foo/.gitattributes").to_owned(),
            Ok("*.txt text".to_string()),
        )]);

        let store = TestStore::from([(
            repo_path("bar/.gitattributes").to_owned(),
            Ok("*.txt text".to_string()),
        )]);

        let git_attributes = GitAttributes::new(store, disk);
        assert_search_output_with_priority(
            &git_attributes,
            "foo/bar.txt",
            "text",
            State::Set,
            SearchPriority::Disk,
        );
        assert_search_output_with_priority(
            &git_attributes,
            "foo/bar.txt",
            "text",
            State::Set,
            SearchPriority::Store,
        );
        assert_search_output_with_priority(
            &git_attributes,
            "bar/bar.txt",
            "text",
            State::Set,
            SearchPriority::Store,
        );
        assert_search_output_with_priority(
            &git_attributes,
            "bar/bar.txt",
            "text",
            State::Set,
            SearchPriority::Disk,
        );
    }

    #[test]
    fn test_file_loader_io_error() {
        let store: TestStore = TestStore::from([(
            repo_path(".gitattributes").to_owned(),
            Err("There was an IO error".to_string()),
        )]);

        let git_attributes = GitAttributes::new(store, HashMap::new());
        let result = &git_attributes
            .search(repo_path("foo/bar.txt"), &["text"], SearchPriority::Disk)
            .block_on();
        assert!(result.is_err());
        assert_eq!(
            &result.as_ref().unwrap_err().message,
            "There was an IO error",
        );
    }

    fn matches(input: &str, path: &str) -> bool {
        let data = TestStore::from([(
            RepoPathBuf::from_internal_string(".gitattributes").unwrap(),
            Ok(input.to_string()),
        )]);
        let attributes = GitAttributes::new(HashMap::new(), data);

        attributes
            .filter_matches(repo_path(path), &["lfs".to_string()], SearchPriority::Disk)
            .block_on()
    }

    #[test]
    fn test_gitattributes_empty_file() {
        assert!(!matches("", "foo"));
    }

    #[test]
    fn test_gitattributes_simple_match() {
        assert!(matches("*.bin filter=lfs\n", "file.bin"));
        assert!(!matches("*.bin filter=lfs\n", "file.txt"));
        assert!(!matches("*.bin filter=other\n", "file.bin"));
        assert!(!matches("*.bin filter=other\n", "path/to/file.bin"));
    }

    #[test]
    fn test_gitattributes_directory_match() {
        // patterns that match a directory do not recursively match paths inside that
        // directory (so using the trailing-slash path/ syntax is pointless in
        // an attributes file; use path/** instead)
        // https://git-scm.com/docs/gitattributes#_description
        assert!(!matches("dir/ filter=lfs\n", "dir/file.txt"));
        assert!(!matches("dir/ filter=lfs\n", "other/file.txt"));
        assert!(!matches("dir/ filter=lfs\n", "dir"));
    }

    #[test]
    fn test_gitattributes_path_match() {
        assert!(matches("path/to/file.bin filter=lfs\n", "path/to/file.bin"));
        assert!(!matches("path/to/file.bin filter=lfs\n", "path/file.bin"));
    }

    #[test]
    fn test_gitattributes_wildcard_match() {
        assert!(matches("*.bin filter=lfs\n", "file.bin"));
        assert!(matches("file.* filter=lfs\n", "file.bin"));
        assert!(matches("**/file.bin filter=lfs\n", "path/to/file.bin"));
    }

    #[test]
    fn test_gitattributes_multiple_attributes() {
        let input = "*.bin filter=lfs diff=binary\n";
        assert!(matches(input, "file.bin"));
        assert!(!matches("*.bin diff=binary\n", "file.bin")); // Only testing filter=lfs
    }

    #[test]
    fn test_gitattributes_chained_files() {
        let data = TestStore::from([
            (
                repo_path(".gitattributes").to_owned(),
                Ok("*.bin filter=lfs\n".to_string()),
            ),
            (
                repo_path("subdir/.gitattributes").to_owned(),
                Ok("*.txt filter=text\n".to_string()),
            ),
        ]);
        let attributes = GitAttributes::new(HashMap::new(), data);

        let filters = &["lfs".to_string(), "text".to_string()];
        assert!(
            attributes
                .filter_matches(repo_path("file.bin"), filters, SearchPriority::Disk)
                .block_on()
        );
        assert!(
            attributes
                .filter_matches(repo_path("subdir/file.txt"), filters, SearchPriority::Disk)
                .block_on()
        );
        assert!(
            !attributes
                .filter_matches(repo_path("file.txt"), filters, SearchPriority::Disk)
                .block_on()
        ); // Not in subdir
    }

    #[test]
    fn test_gitattributes_negated_pattern() {
        let input = "*.bin filter=lfs\n!important.bin filter=lfs\n";
        assert!(matches(input, "file.bin"));
        // negative patterns are forbidden
        // https://git-scm.com/docs/gitattributes#_description
        assert!(matches(input, "important.bin"));
    }

    #[test]
    fn test_gitattributes_multiple_filters() {
        let data = TestStore::from([
            (
                repo_path(".gitattributes").to_owned(),
                Ok(indoc! {"
                    *.bin filter=lfs
                    *.secret filter=git-crypt
                    *.txt filter=other
                "}
                .to_string()),
            ),
            (
                repo_path("subdir/.gitattributes").to_owned(),
                Ok("*.txt filter=text\n".to_string()),
            ),
        ]);

        // Create a GitAttributesFile with both "lfs" and "git-crypt" as ignore filters
        let attributes = GitAttributes::new(HashMap::new(), data);

        let filters = &["lfs".to_string(), "git-crypt".to_string()];

        // Test with lfs filter
        assert!(
            attributes
                .filter_matches(repo_path("file.bin"), filters, SearchPriority::Disk)
                .block_on()
        );
        // Test with git-crypt filter
        assert!(
            attributes
                .filter_matches(
                    repo_path("credentials.secret"),
                    filters,
                    SearchPriority::Disk
                )
                .block_on()
        );
        // Not In the filter
        assert!(
            !attributes
                .filter_matches(repo_path("file.bin2"), filters, SearchPriority::Disk)
                .block_on()
        );
        // Test that other filters don't match
        assert!(
            !attributes
                .filter_matches(repo_path("file.txt"), filters, SearchPriority::Disk)
                .block_on()
        );
    }
}
