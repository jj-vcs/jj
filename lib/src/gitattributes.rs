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

//! Utilities to read the `gitattributes` files and match against them.
//!
//! [`GitAttributes`] provides the key functionality to cache, query the states
//! of gitattributes associated to a file, and read and parse the
//! `gitattributes` files lazily. [`GitAttributes::search`] is the query
//! interface.
//!
//! [`FileLoader`] a trait that encapsulates the implementation details on how
//! to read `gitattributes` files given paths, either from the
//! [`Store`](crate::store::Store) or from the file system. [`TreeFileLoader`]
//! and [`DiskFileLoader`] are 2 implementations. [`FileLoader`]s are used to
//! initialize [`GitAttributes`].

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fmt::Debug;
use std::fmt::Formatter;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use bstr::BStr;
use gix_attributes::glob::pattern::Case;
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

/// The state an attribute can be in.
///
/// This type is directly from the [state defined] in the git attributes
/// document. Note that this doesn't contain the name.
///
/// [state defined]: https://git-scm.com/docs/gitattributes#_description
#[derive(PartialEq, Eq, Clone)]
pub enum State {
    /// The path has the attribute with special value "true"; this is specified
    /// by listing only the name of the attribute in the attribute list.
    ///
    /// For example, with the below `gitattributes` file, the `text` attribute
    /// is in the set state for the `a.txt` file.
    ///
    /// ```gitattributes
    /// a.txt text
    /// ```
    Set,
    /// The path has the attribute with special value "false"; this is specified
    /// by listing the name of the attribute prefixed with a dash `-` in the
    /// attribute list.
    ///
    /// For example, with the below `gitattributes` file, the `text` attribute
    /// is in the unset state for the `a.png` file.
    ///
    /// ```gitattributes
    /// a.png -text
    /// ```
    Unset,
    /// Set to a value. The path has the attribute with specified string value;
    /// this is specified by listing the name of the attribute followed by an
    /// equal sign `=` and its value in the attribute list.
    ///
    /// For example, with the below `gitattributes` file, the `eol` attribute is
    /// in the set to `"crlf"` state for the `a.bat` file.
    ///
    /// ```gitattributes
    /// a.bat eol=crlf
    /// ```
    Value(Vec<u8>),
    /// No pattern matches the path, and nothing says if the path has or does
    /// not have the attribute, the attribute for the path is said to be
    /// Unspecified. Listing the name of the attribute prefixed with an
    /// exclamation point `!` also makes that attribute unspecified.
    ///
    /// For example, with the below `gitattributes` file, the `text` attribute
    /// is in the unspecified state for the `a.txt` file.
    ///
    /// ```gitattributes
    /// a.txt !text
    /// ```
    Unspecified,
}

impl Debug for State {
    /// Implement [`Debug::fmt`] manually, so that [`State::Value`] prints as a
    /// byte string instead of a list of numbers for readability.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        use State::*;
        match self {
            Set => f.write_str("State::Set"),
            Unset => f.write_str("State::Unset"),
            Value(value) => f
                .debug_tuple("State::Value")
                .field(&BStr::new(&value))
                .finish(),
            Unspecified => f.write_str("State::Unspecified"),
        }
    }
}

impl From<gix_attributes::StateRef<'_>> for State {
    fn from(value: gix_attributes::StateRef) -> Self {
        use gix_attributes::StateRef::*;
        match value {
            Set => Self::Set,
            Unset => Self::Unset,
            Value(value) => Self::Value(value.as_bstr().to_vec()),
            Unspecified => Self::Unspecified,
        }
    }
}

/// The error type for `gitattributes` operations.
///
/// Errors mostly originate from reading from the store, reading from the file
/// system, and path conversion.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct GitAttributesError {
    message: String,
    #[source]
    source: Box<dyn std::error::Error + Send + Sync>,
}

type Result<T> = std::result::Result<T, GitAttributesError>;

/// An abstraction over the operation of reading the gitattributes file contents
/// from the [`Store`](crate::store::Store) or from the file system.
///
/// The [`Debug`] trait requirement is used to format error message when error
/// happens. This makes it easier to debug which [`FileLoader`] fails from the
/// log or the error message.
#[async_trait]
pub trait FileLoader: Debug + Send + Sync {
    /// Given a path to a `gitattributes` file, return the contents of that
    /// file.
    ///
    /// This function will return the contents of a `gitattributes` file. If the
    /// file doesn't exist or the path points to an entry that is not a
    /// file(e.g., a folder), this method must return
    /// [`Ok(None)`](std::result::Result::Ok), so that the caller knows the file
    /// is missing instead of an IO error that can't be recovered. According to
    /// [the `gitattributes` document], the method should not follow symbolic
    /// links.
    ///
    /// # Errors
    ///
    /// If the underlying [`Store`](crate::store::Store) operation or the file
    /// system IO fails, an error is returned.
    ///
    /// It is **not** considered an error if `path` points to nothing or doesn't
    /// point to a file.
    ///
    /// [the `gitattributes` document]: https://git-scm.com/docs/gitattributes#_notes
    async fn load(&self, path: &RepoPath) -> Result<Option<Box<dyn AsyncRead + Send + Unpin>>>;
}

/// An abstraction over the operation of reading the gitattributes file contents
/// from the [`Store`](crate::store::Store).
pub struct TreeFileLoader {
    tree: MergedTree,
}

impl TreeFileLoader {
    /// Create a [`TreeFileLoader`] from a given [`MergedTree`].
    ///
    /// [`TreeFileLoader::load`] reads the contents of the `gitattributes` file
    /// from the `tree`.
    pub fn new(tree: MergedTree) -> Self {
        Self { tree }
    }
}

#[async_trait]
impl FileLoader for TreeFileLoader {
    /// Given a path to a `gitattributes` file, return the contents from the
    /// [`MergedTree`] passed in in [`TreeFileLoader::new`].
    ///
    /// If there are conflicts on the `gitattributes` file, we always resolve
    /// the conflict before returning the contents.
    async fn load(&self, path: &RepoPath) -> Result<Option<Box<dyn AsyncRead + Send + Unpin>>> {
        let tree_values =
            self.tree
                .path_value_async(path)
                .await
                .map_err(|source| GitAttributesError {
                    message: format!(
                        "Failed to obtain the tree value at the path {}",
                        path.as_internal_file_string()
                    ),
                    source: source.into(),
                })?;
        let maybe_file_merge = tree_values.to_file_merge();
        let file_id = match maybe_file_merge
            .as_ref()
            .and_then(|files| files.resolve_trivial(SameChange::Accept))
        {
            // Trivially resolve to a file.
            Some(Some(file_id)) => file_id,
            // Trivially resolve to an absent entry: files.resolve_trivial() returns Some(None).
            Some(None) => return Ok(None),
            // The rest case. The merge can be trivially resolved neither to a file nor an absent
            // entry. Particularly, we don't follow the symbolic link as required by the
            // `gitattributes` document.
            None => {
                // Let's just find the first file entry and use it for simplicity. We can
                // improve the conflict case better if needed.
                let Some(id) = tree_values.iter().find_map(|tree_value| {
                    let Some(TreeValue::File { id, .. }) = tree_value else {
                        return None;
                    };
                    Some(id)
                }) else {
                    // None of the conflict sides are files.
                    return Ok(None);
                };
                id
            }
        };
        let file = self
            .tree
            .store()
            .read_file(path, file_id)
            .await
            .map_err(|source| GitAttributesError {
                message: format!(
                    "Failed to read the file from the store at the path {}",
                    path.as_internal_file_string()
                ),
                source: source.into(),
            })?;
        Ok(Some(Box::new(file)))
    }
}

impl Debug for TreeFileLoader {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TreeFileLoader").field(&self.tree).finish()
    }
}

/// An abstraction over the operation of reading the gitattributes file contents
/// from the file system.
pub struct DiskFileLoader {
    repo_root: PathBuf,
}

impl DiskFileLoader {
    /// Create a [`DiskFileLoader`] from a given project root path.
    ///
    /// `repo_root` is used in [`DiskFileLoader::load`] to resolve a
    /// [`RepoPath`] to a file system path.
    pub fn new(repo_root: PathBuf) -> Self {
        Self { repo_root }
    }
}

#[async_trait]
impl FileLoader for DiskFileLoader {
    async fn load(&self, path: &RepoPath) -> Result<Option<Box<dyn AsyncRead + Send + Unpin>>> {
        let path = path
            .to_fs_path(&self.repo_root)
            .map_err(|source| GitAttributesError {
                message: format!(
                    "Failed to convert the input path({}) to a filesystem path",
                    path.as_internal_file_string()
                ),
                source: source.into(),
            })?;
        // According to the `gitattributes` document, we shouldn't follow the symbolic
        // link.
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
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
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(GitAttributesError {
                    message: format!("Failed to open the file at {}", path.display()),
                    source: err.into(),
                });
            }
        };
        Ok(Some(Box::new(BlockingAsyncReader::new(file)) as Box<_>))
    }
}

impl Debug for DiskFileLoader {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("DiskFileLoader")
            .field(&format_args!("{}", self.repo_root.display()))
            .finish()
    }
}

#[derive(Clone)]
struct GitAttributesNode {
    search: gix_attributes::Search,
    metadata_collection: gix_attributes::search::MetadataCollection,
}

/// A cached, lazy query interface to query the states of given git attributes
/// associated to a file.
pub struct GitAttributes {
    /// The key is the path of a directory, the value is the associated
    /// [`GitAttributesNode`].
    ///
    /// The [`GitAttributesNode`] of a folder has direct influence on all the
    /// files under that folder, and indirect influence on files in its
    /// descendant folder. The attribute states of files under the same folder,
    /// e.g. `foo/bar1.txt`, `foo/bar2.txt`, are decided by the same
    /// [`GitAttributesNode`] object, and the cache key is the path to the
    /// folder, e.g., `foo`. This allows files under the same folder properly
    /// share the same cache entry.
    node_cache: Mutex<HashMap<RepoPathBuf, Arc<OnceCell<Arc<GitAttributesNode>>>>>,
    file_loaders: Box<[Arc<dyn FileLoader>]>,
}

impl GitAttributes {
    async fn initialize_node(&self, path: &RepoPath) -> Result<Arc<GitAttributesNode>> {
        let parent = path.parent();
        let base_objects = match parent {
            Some(parent_path) => Box::pin(self.get_node(parent_path))
                .await
                .map_err(|source| GitAttributesError {
                    message: format!(
                        "Failed to obtain the parent of {}",
                        path.to_internal_dir_string()
                    ),
                    source: source.into(),
                })?,
            None => {
                // `path` points to the repository root.
                let mut search = gix_attributes::Search::default();
                let mut metadata = gix_attributes::search::MetadataCollection::default();
                const BUILT_IN: &[u8] = b"[attr]binary -diff -merge -text";
                search.add_patterns_buffer(BUILT_IN, "[builtin]".into(), None, &mut metadata, true);
                Arc::new(GitAttributesNode {
                    search,
                    metadata_collection: metadata,
                })
            }
        };
        let gitattributes_path = path.join(RepoPathComponent::DOT_GITATTRIBUTES);
        let mut file = None;
        for file_loader in &self.file_loaders {
            file = file_loader
                .load(&gitattributes_path)
                .await
                .map_err(|source| GitAttributesError {
                    message: format!(
                        "Failed to read the .gitattributes file from {:?} at {}",
                        file_loader,
                        gitattributes_path.to_internal_dir_string(),
                    ),
                    source: source.into(),
                })?;
            if file.is_some() {
                break;
            }
        }
        let Some(mut file) = file else {
            // If no `gitattributes` files exist for the current folder, we just use the
            // same Search object as the parent.
            return Ok(base_objects);
        };
        let mut contents = vec![];
        file.read_to_end(&mut contents)
            .await
            .map_err(|source| GitAttributesError {
                message: format!(
                    "Failed to read the contents of .gitattributes file at {}",
                    gitattributes_path.as_internal_file_string()
                ),
                source: source.into(),
            })?;
        let mut res = GitAttributesNode::clone(&*base_objects);
        res.search.add_patterns_buffer(
            &contents,
            gitattributes_path
                .to_fs_path(&PathBuf::new())
                .map_err(|source| GitAttributesError {
                    message: "Failed to convert the gitattributes path from RepoPath to Path"
                        .to_string(),
                    source: source.into(),
                })?,
            // The root parameter should be `Some("")` to enable local relative search mode
            // according to:
            //
            // * https://docs.rs/gix-attributes/0.26.1/gix_attributes/struct.Search.html#method.add_patterns_file
            // * https://docs.rs/gix-glob/0.21.0/gix_glob/search/pattern/struct.List.html#method.from_bytes
            Some(&PathBuf::new()),
            &mut res.metadata_collection,
            // Only allow macros for the root `gitattributes` file, i.e., path doesn't have a
            // parent, because Git only allows macro definition in limited files:
            //
            // > Custom macro attributes can be defined only in top-level gitattributes files
            // > (`$GIT_DIR/info/attributes`, the `.gitattributes` file at the top level of the
            // > working tree, or the global or system-wide gitattributes files), not in
            // > `.gitattributes` files in working tree subdirectories.
            parent.is_none(),
        );
        Ok(Arc::new(res))
    }

    /// The only valid way to access [`Self::node_cache`].
    async fn get_node(&self, path: &RepoPath) -> Result<Arc<GitAttributesNode>> {
        let node = match self.node_cache.lock().unwrap().entry(path.to_owned()) {
            Entry::Occupied(node) => node.get().clone(),
            Entry::Vacant(node) => Arc::clone(node.insert(Arc::new(OnceCell::new()))),
        };
        // We perform the actual initialization without the lock held for better
        // parallelism.
        node.get_or_try_init(|| self.initialize_node(path))
            .await
            .cloned()
    }

    /// Query the states of git attributes associated to the `path`.
    ///
    /// If the `gitattributes` file is missing in one folder from one source,
    /// other sources provided in [`Self::new`] as the `file_loaders` will be
    /// used as fallbacks.
    ///
    /// * `path`: the path to the file whose `gitattributes` states are of
    ///   interest. While passing in a root path won't panic, the return value
    ///   is also unspecified.
    /// * `attribute_names`: the names of the `gitattributes` of interest. The
    ///   return value will only include the states of `gitattributes` mentioned
    ///   in this list.
    ///
    /// Note that currently, the pattern match is performed in a case sensitive
    /// way.
    ///
    /// # Return value
    ///
    /// In the returned [`HashMap`], the key is the name of the git attribute,
    /// and the value is the [`State`] of that git attribute. If the `path`
    /// doesn't match any pattern in the `gitattributes` files, the returned
    /// [`HashMap`] still contains that entry with the [`State::Unspecified`]
    /// value.
    ///
    /// # Error
    ///
    /// If the related `gitattributes` files are not cached, and the underlying
    /// [`FileLoader`] fails to load the file, e.g., an I/O error, an error will
    /// be returned.
    pub async fn search<'a>(
        &self,
        path: &RepoPath,
        attribute_names: impl AsRef<[&'a str]>,
    ) -> Result<HashMap<String, State>> {
        // The cache key for the GitAttributesNode object that controls `path`, is the
        // parent of `path`.
        let parent_path = path.parent().unwrap_or(path);
        let node = self
            .get_node(parent_path)
            .await
            .map_err(|source| GitAttributesError {
                message: format!(
                    "Failed to retrieve the search and metadata objects when retrieving \
                     gitattributes objects for {}",
                    parent_path.as_internal_file_string()
                ),
                source: source.into(),
            })?;
        let mut outcome = Outcome::default();
        let attribute_names = attribute_names.as_ref();
        outcome
            .initialize_with_selection(&node.metadata_collection, attribute_names.iter().copied());
        node.search.pattern_matching_relative_path(
            path.as_internal_file_string().as_bytes().into(),
            Case::Sensitive,
            None,
            &mut outcome,
        );
        let mut res = outcome
            .iter_selected()
            .map(|search_match| {
                let assignment = search_match.assignment;
                (
                    assignment.name.as_str().to_string(),
                    assignment.state.into(),
                )
            })
            .collect::<HashMap<_, _>>();
        for attribute_name in attribute_names {
            // Populate the missing attributes entries with the unspecified state value.
            let Entry::Vacant(entry) = res.entry(attribute_name.to_string()) else {
                continue;
            };
            entry.insert(State::Unspecified);
        }
        Ok(res)
    }

    /// Create a [`GitAttributes`] instance with the given sources of
    /// `gitattributes` files.
    ///
    /// [`GitAttributes`] uses [`FileLoader`]s in the `file_loaders` parameter
    /// to obtain the contents of the `gitattributes` files. If `file_loaders`
    /// is empty, [`Self::search`] will always return [`State::Unspecified`] for
    /// all attributes. If there are multiple [`FileLoader`]s, when the
    /// `gitattributes` file is missing in one [`FileLoader`], the next
    /// [`FileLoader`] will be used as the fall-back. This makes it easier to
    /// implement the below feature mentioned in the [gitattributes document]:
    ///
    /// > When the `.gitattributes` file is missing from the work tree, the path
    /// > in the index is used as a fall-back. During checkout process,
    /// > `.gitattributes` in the index is used and then the file in the working
    /// > tree is used as a fall-back.
    ///
    /// [gitattributes document]: https://git-scm.com/docs/gitattributes#_description
    pub fn new(file_loaders: Vec<Arc<dyn FileLoader>>) -> Self {
        Self {
            node_cache: Mutex::new(HashMap::default()),
            file_loaders: file_loaders.into_boxed_slice(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::pin::Pin;
    use std::task::Poll;

    use gix_attributes::state::ValueRef;
    use indoc::indoc;
    use itertools::Itertools as _;
    use pollster::FutureExt as _;
    use test_case::test_case;
    #[cfg(unix)]
    use unix::*;

    use super::*;
    use crate::file_util::check_symlink_support;
    use crate::file_util::symlink_file;

    #[cfg(unix)]
    mod unix {
        pub use std::os::unix::fs::PermissionsExt;
        pub struct Defer<'a>(Box<dyn FnMut() + 'a>);
        impl<'a> Defer<'a> {
            pub fn new(f: impl FnMut() + 'a) -> Self {
                Self(Box::new(f))
            }
        }
        impl Drop for Defer<'_> {
            fn drop(&mut self) {
                (self.0)();
            }
        }
    }

    type FakeFileLoader = HashMap<RepoPathBuf, std::result::Result<String, String>>;
    #[async_trait]
    impl FileLoader for FakeFileLoader {
        async fn load(&self, path: &RepoPath) -> Result<Option<Box<dyn AsyncRead + Send + Unpin>>> {
            let Some(res) = self.get(path) else {
                return Ok(None);
            };
            match res.clone() {
                Ok(contents) => Ok(Some(Box::new(Cursor::new(Vec::from(contents))) as Box<_>)),
                Err(message) => Err(GitAttributesError {
                    message: message.clone(),
                    source: message.into(),
                }),
            }
        }
    }

    fn repo_path(value: &str) -> RepoPathBuf {
        RepoPathBuf::from_internal_string(value).unwrap()
    }

    #[test_case(State::Set, "Set"; "set")]
    #[test_case(State::Unset, "Unset"; "unset")]
    #[test_case(State::Value(b"git-lfs".to_vec()), "git-lfs"; "set to value")]
    #[test_case(State::Unspecified, "Unspecified"; "unspecified")]
    fn test_gitattr_state_debug(state: State, expected_substring: &str) {
        let debug_output = format!("{state:?}");
        assert!(
            debug_output.contains(expected_substring),
            "Expect string to contain {expected_substring:?}, but got: {debug_output:?}.",
        );
    }

    #[test_case(gix_attributes::StateRef::Set, State::Set; "set")]
    #[test_case(gix_attributes::StateRef::Unset, State::Unset; "unset")]
    #[test_case(
        gix_attributes::StateRef::Value(ValueRef::from_bytes(b"git-lfs")),
        State::Value(b"git-lfs".to_vec());
        "set to value"
    )]
    #[test_case(gix_attributes::StateRef::Unspecified, State::Unspecified; "unspecified")]
    fn test_gitattr_state_from_gix_attributes_state_ref(
        value: gix_attributes::StateRef,
        expected: State,
    ) {
        assert_eq!(State::from(value), expected);
    }

    #[test]
    fn test_gitattr_disk_file_loader_invalid_query_path() {
        let file_loader = DiskFileLoader::new(PathBuf::from("jj"));
        let res = file_loader.load(&repo_path("a\\b")).block_on();
        if cfg!(windows) {
            // a\b is probably only an invalid path on Windows.
            assert!(
                res.is_err(),
                "Expect DiskFileLoader::load() to return an error with an invalid path."
            );
        }
    }

    #[test]
    fn test_gitattr_disk_file_loader_should_not_follow_symlink() {
        if !check_symlink_support().unwrap() {
            eprintln!("Skip the test if symlink is not supported.");
            return;
        }
        let tempdir = tempfile::Builder::new()
            .prefix("jj-test-")
            .tempdir()
            .unwrap();
        let root_path = tempdir.path();
        let link_repo_path = repo_path(".gitattributes");
        let link_path = link_repo_path.to_fs_path(root_path).unwrap();
        let target_path = PathBuf::from("a.txt");
        fs::write(root_path.join(&target_path), "a.txt text\n").unwrap();
        symlink_file(target_path, link_path).unwrap();

        let file_loader = DiskFileLoader::new(root_path.to_owned());
        let res = file_loader.load(&link_repo_path).block_on().unwrap();
        assert!(
            res.is_none(),
            "Expect DiskFileLoader::load to return `None` when the path is a symlink."
        );
    }

    #[test]
    fn test_gitattr_disk_file_loader_path_doesnt_exist() {
        let tempdir = tempfile::Builder::new()
            .prefix("jj-test-")
            .tempdir()
            .unwrap();
        let root_path = tempdir.path();
        let repo_path = repo_path(".gitattributes");
        let path = repo_path.to_fs_path(root_path).unwrap();
        assert!(!fs::exists(path).unwrap());

        let file_loader = DiskFileLoader::new(root_path.to_owned());
        let res = file_loader.load(&repo_path).block_on().unwrap();
        assert!(
            res.is_none(),
            "Expect DiskFileLoader::load to return `None` when the path doesn't exist."
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_gitattr_disk_file_loader_path_cant_access_metadata() {
        let tempdir = tempfile::Builder::new()
            .prefix("jj-test-")
            .tempdir()
            .unwrap();
        let root_path = tempdir.path();
        let repo_path = repo_path("dir/.gitattributes");
        let dir_path = root_path.join("dir");
        fs::create_dir_all(&dir_path).unwrap();
        let path = repo_path.to_fs_path(root_path).unwrap();
        fs::write(&path, "a.txt text\n").unwrap();
        let mut perm = dir_path.symlink_metadata().unwrap().permissions();
        let old_perm = perm.clone();
        perm.set_mode(0o000);
        fs::set_permissions(&dir_path, perm.clone()).unwrap();
        let defer = Defer::new(|| fs::set_permissions(&dir_path, old_perm.clone()).unwrap());
        assert!(path.symlink_metadata().is_err());

        let file_loader = DiskFileLoader::new(root_path.to_owned());
        let res = file_loader.load(&repo_path).block_on();
        assert!(
            res.is_err(),
            "Expect DiskFileLoader::load to return an error when we can't access the metadata of \
             the path."
        );
        drop(defer);
    }

    #[test]
    #[cfg(unix)]
    fn test_gitattr_disk_file_loader_path_cant_access_file() {
        let tempdir = tempfile::Builder::new()
            .prefix("jj-test-")
            .tempdir()
            .unwrap();
        let root_path = tempdir.path();
        let repo_path = repo_path(".gitattributes");
        let path = repo_path.to_fs_path(root_path).unwrap();
        fs::write(&path, "a.txt text\n").unwrap();
        let mut perm = path.symlink_metadata().unwrap().permissions();
        let old_perm = perm.clone();
        perm.set_mode(0o000);
        fs::set_permissions(&path, perm.clone()).unwrap();
        let defer = Defer::new(|| fs::set_permissions(&path, old_perm.clone()).unwrap());
        assert!(fs::read(&path).is_err());

        let file_loader = DiskFileLoader::new(root_path.to_owned());
        let res = file_loader.load(&repo_path).block_on();
        assert!(
            res.is_err(),
            "Expect DiskFileLoader::load to return an error when we can't access the file."
        );
        drop(defer);
    }

    #[test]
    fn test_gitattr_disk_file_loader_load_file_contents() {
        let tempdir = tempfile::Builder::new()
            .prefix("jj-test-")
            .tempdir()
            .unwrap();
        let contents = "a.txt text\n";
        let root_path = tempdir.path();
        let repo_path = repo_path(".gitattributes");
        let path = repo_path.to_fs_path(root_path).unwrap();
        fs::write(&path, contents).unwrap();

        let file_loader = DiskFileLoader::new(root_path.to_owned());
        let mut actual_contents = String::new();
        file_loader
            .load(&repo_path)
            .block_on()
            .unwrap()
            .expect("the file exist")
            .read_to_string(&mut actual_contents)
            .block_on()
            .unwrap();
        assert_eq!(contents, actual_contents);
    }

    #[test]
    fn test_gitattr_disk_file_loader_debug() {
        let path = "jj-test/path";
        let file_loader = DiskFileLoader::new(PathBuf::from(path));
        let debug = format!("{file_loader:?}");
        assert!(
            debug.contains(path),
            "Expect string to contain {path:?}, but got: {debug:?}.",
        );
    }

    #[test]
    fn test_gitattr_load_file_error() {
        // File load failure on parent node should cover more paths.
        let file_loader =
            FakeFileLoader::from([(repo_path(".gitattributes"), Err("test error".to_owned()))]);
        let git_attributes = GitAttributes::new(vec![Arc::new(file_loader)]);
        git_attributes
            .search(&repo_path("dir/a.txt"), &["text"])
            .block_on()
            .expect_err("search should fail");
    }

    #[test_case(
        vec![Some("a.txt text\n"), Some("a.txt -text\n")] => State::Set;
        "exist in all sources"
    )]
    #[test_case(
        vec![None, Some("a.txt -text\n")] => State::Unset;
        "missing in the primary source"
    )]
    #[test_case(
        vec![Some("a.txt -text\n"), None] => State::Unset;
        "missing in the secondary source"
    )]
    #[test_case(
        vec![None, None] => State::Unspecified;
        "missing in both sources"
    )]
    fn test_gitattr_fallback(contents: Vec<Option<&str>>) -> State {
        let file_loaders = contents
            .into_iter()
            .map(|content| {
                let fake_file_loader: FakeFileLoader = content
                    .iter()
                    .map(|content| (repo_path(".gitattributes"), Ok(content.to_string())))
                    .collect();
                Arc::new(fake_file_loader) as Arc<_>
            })
            .collect_vec();
        let git_attributes = GitAttributes::new(file_loaders);
        git_attributes
            .search(&repo_path("a.txt"), &["text"])
            .block_on()
            .unwrap()
            .get("text")
            .unwrap()
            .clone()
    }

    #[test]
    fn test_gitattr_file_read_fail() {
        struct ErrorReader;
        impl AsyncRead for ErrorReader {
            fn poll_read(
                self: Pin<&mut Self>,
                _cx: &mut std::task::Context<'_>,
                _buf: &mut tokio::io::ReadBuf<'_>,
            ) -> Poll<std::io::Result<()>> {
                Poll::Ready(Err(std::io::Error::other("test error")))
            }
        }
        #[derive(Debug)]
        struct TestFileLoader;
        #[async_trait]
        impl FileLoader for TestFileLoader {
            async fn load(
                &self,
                _: &RepoPath,
            ) -> Result<Option<Box<dyn AsyncRead + Send + Unpin>>> {
                Ok(Some(Box::new(ErrorReader)))
            }
        }

        let gitattributes = GitAttributes::new(vec![Arc::new(TestFileLoader)]);
        gitattributes
            .search(&repo_path("a.txt"), &["text"])
            .block_on()
            .expect_err("search should fail");
    }

    #[test]
    fn test_gitattr_invalid_path() {
        let file_loader = FakeFileLoader::from([(
            repo_path("a/./b/.gitattributes"),
            Ok("a.txt text".to_owned()),
        )]);
        let gitattributes = GitAttributes::new(vec![Arc::new(file_loader)]);
        gitattributes
            .search(&repo_path("a/./b/a.txt"), &["text"])
            .block_on()
            .expect_err("search should fail");
    }

    type FilePath = &'static str;
    type AttributeName = &'static str;
    struct TestGitAttributesSearchConfig {
        gitattributes_files: Vec<(FilePath, &'static str)>,
        expected_queries: Vec<(AttributeName, Vec<(AttributeName, State)>)>,
    }

    #[test_case(
        TestGitAttributesSearchConfig {
            gitattributes_files: vec![(
                ".gitattributes",
                indoc! {"
                    a.txt text
                    a.png -text
                "},
            )],
            expected_queries: vec![
                ("a.txt", vec![("text", State::Set)]),
                ("a.png", vec![("text", State::Unset)]),
            ],
        };
        "cached gitattributes file"
    )]
    #[test_case(
        TestGitAttributesSearchConfig {
            gitattributes_files: vec![(
                ".gitattributes",
                indoc!{"
                    [attr]test_macro text
                    a.txt test_macro
                "},
            )],
            expected_queries: vec![("a.txt", vec![("text", State::Set)])],
        };
        "macro defined in root"
    )]
    #[test_case(
        TestGitAttributesSearchConfig {
            gitattributes_files: vec![(
                "a/.gitattributes",
                indoc!{"
                    [attr]test_macro text
                    a.txt test_macro
                "},
            )],
            expected_queries: vec![
                ("a/a.txt", vec![("text", State::Unspecified)])
            ],
        };
        "macro defined in subdirectory"
    )]
    #[test_case(
        TestGitAttributesSearchConfig {
            gitattributes_files: vec![(".gitattributes", "a.txt text\n")],
            expected_queries: vec![("dir/a.txt", vec![("text", State::Set)])],
        };
        "gitattr in parent folder"
    )]
    #[test_case(
        TestGitAttributesSearchConfig {
            gitattributes_files: vec![(".gitattributes", "a.out binary\n")],
            expected_queries: vec![(
                "a.out",
                vec![
                    ("diff", State::Unset),
                    ("merge", State::Unset),
                    ("text", State::Unset),
                ]
            )],
        };
        "builtin macro"
    )]
    #[test_case(
        TestGitAttributesSearchConfig {
            gitattributes_files: vec![(
                ".gitattributes",
                indoc!{"
                    a.txt text
                    B.png -text
                "},
            )],
            expected_queries: vec![
                ("a.txt", vec![("text", State::Set)]),
                ("A.txt", vec![("text", State::Unspecified)]),
                ("b.png", vec![("text", State::Unspecified)]),
                ("B.png", vec![("text", State::Unset)]),
            ],
        };
        "pattern should match case"
    )]
    #[test_case(
        TestGitAttributesSearchConfig {
            gitattributes_files: vec![],
            expected_queries: vec![
                ("a.txt", vec![("text", State::Unspecified)]),
            ],
        };
        "result should contain missing attributes"
    )]
    #[test_case(
        TestGitAttributesSearchConfig {
            gitattributes_files: vec![(".gitattributes", "a.txt crlf\n")],
            expected_queries: vec![("a.txt", vec![("crlf", State::Set)])],
        };
        "attr is set"
    )]
    #[test_case(
        TestGitAttributesSearchConfig {
            gitattributes_files: vec![(".gitattributes", "a.txt -crlf\n")],
            expected_queries: vec![("a.txt", vec![("crlf", State::Unset)])],
        };
        "attr is unset"
    )]
    #[test_case(
        TestGitAttributesSearchConfig {
            gitattributes_files: vec![(".gitattributes", "a.txt crlf=input\n")],
            expected_queries: vec![(
                "a.txt",
                vec![("crlf", State::Value(b"input".to_vec()))],
            )],
        };
        "attr is set to value"
    )]
    #[test_case(
        TestGitAttributesSearchConfig {
            gitattributes_files: vec![(".gitattributes", "a.txt !crlf\n")],
            expected_queries: vec![
                ("a.txt", vec![("crlf", State::Unspecified)]),
            ],
        };
        "attr is unspecified"
    )]
    #[test_case(
        TestGitAttributesSearchConfig {
            gitattributes_files: vec![
                (".gitattributes", "abc foo bar baz\n"),
                (
                    "t/.gitattributes",
                    indoc!{"
                        ab* merge=filfre
                        abc -foo -bar
                        *.c frotz
                    "}
                ),
            ],
            expected_queries: vec![(
                "t/abc",
                vec![
                    ("foo", State::Unset),
                    ("bar", State::Unset),
                    ("baz", State::Set),
                    ("merge", State::Value(b"filfre".to_vec())),
                    ("frotz", State::Unspecified),
                ]
            )],
        };
        // Modified based on https://git-scm.com/docs/gitattributes#_examples.
        "modified gitattr doc example"
    )]
    #[test_case(
        TestGitAttributesSearchConfig {
            gitattributes_files: vec![
                (".gitattributes", "hello.* text\n"),
                ("a/.gitattributes", "/hello.* -text\n"),
            ],
            expected_queries: vec![
                ("hello.txt", vec![("text", State::Set)]),
                ("hello.c", vec![("text", State::Set)]),
                ("a/hello.txt", vec![("text", State::Unset)]),
                ("a/hello.c", vec![("text", State::Unset)]),
                ("a/b/hello.txt", vec![("text", State::Set)]),
                ("a/b/hello.c", vec![("text", State::Set)]),
            ],
        };
        "star pattern"
    )]
    #[test_case(
        TestGitAttributesSearchConfig {
            gitattributes_files: vec![(
                ".gitattributes",
                indoc!{"
                    path/ text
                    path/** crlf
                "},
            )],
            expected_queries: vec![(
                "path/hello.txt",
                vec![
                    ("text", State::Unspecified),
                    ("crlf", State::Set),
                ]
            )],
        };
        "directory pattern"
    )]
    #[test_case(
        TestGitAttributesSearchConfig {
            gitattributes_files: vec![(
                ".gitattributes",
                indoc!{"
                    doc/frotz text
                    /doc/frotz crlf
                "},
            )],
            expected_queries: vec![
                (
                    "doc/frotz",
                    vec![
                        ("text", State::Set),
                        ("crlf", State::Set),
                    ]
                ),
                (
                    "a/doc/frotz",
                    vec![
                        ("text", State::Unspecified),
                        ("crlf", State::Unspecified),
                    ]
                ),
            ],
        };
        "irrelevant leading slash in pattern"
    )]
    #[test_case(
        TestGitAttributesSearchConfig {
            gitattributes_files: vec![(".gitattributes", "foo/* text\n")],
            expected_queries: vec![
                ("foo/test.json", vec![("text", State::Set)]),
                ("foo/bar", vec![("text", State::Set)]),
                ("foo/bar/hello.c", vec![("text", State::Unspecified)]),
            ],
        };
        "single asterisk won't match slash"
    )]
    fn test_gitattr_search(
        TestGitAttributesSearchConfig {
            gitattributes_files,
            expected_queries,
        }: TestGitAttributesSearchConfig,
    ) {
        let file_loader: FakeFileLoader = gitattributes_files
            .into_iter()
            .map(|(path, contents)| (repo_path(path), Ok(contents.to_owned())))
            .collect();
        let gitattributes = GitAttributes::new(vec![Arc::new(file_loader)]);
        for (path, expected_states) in expected_queries {
            let attributes_names = expected_states.iter().map(|(name, _)| *name).collect_vec();
            let states = gitattributes
                .search(&repo_path(path), &attributes_names)
                .block_on()
                .unwrap_or_else(|e| {
                    let attributes_names = attributes_names.join(", ");
                    panic!(
                        "Failed to search the attributes {attributes_names} of the {path} file: \
                         {e:?}"
                    )
                });
            for (name, state) in expected_states {
                let actual_state = states
                    .get(name)
                    .unwrap_or_else(|| {
                        panic!(
                            "The {name} attribute is missing in the search result of the {path} \
                             file."
                        )
                    })
                    .clone();
                assert_eq!(
                    actual_state, state,
                    "Expect the state of the {name} attribute of the {path} file matches \
                     {state:?}, but got: {actual_state:?}"
                );
            }
        }
    }

    #[test]
    fn test_gitattr_search_long_path() {
        let dir = "a/".repeat(200);
        let file_loader = FakeFileLoader::from([(
            repo_path(&format!("{dir}.gitattributes")),
            Ok("a.txt text\n".to_owned()),
        )]);
        let gitattributes = GitAttributes::new(vec![Arc::new(file_loader)]);
        let state = gitattributes
            .search(&repo_path(&format!("{dir}a.txt")), &["text"])
            .block_on()
            .expect("Search shouldn't fail")
            .get("text")
            .expect("The result should contain the text attribute state")
            .clone();
        assert_eq!(state, State::Set);
    }
}
