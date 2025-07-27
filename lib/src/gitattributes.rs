// Copyright 2024 The Jujutsu Authors
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

#![allow(missing_docs)]

use std::borrow::Cow;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use gix::attrs as gix_attrs;
use gix::glob as gix_glob;
use gix::path as gix_path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitAttributesError {
    #[error("Failed to read attributes patterns from file {path}")]
    ReadFile { path: PathBuf, source: io::Error },
}

/// Models the effective contents of multiple .gitattributes files.
#[derive(Debug)]
pub struct GitAttributesFile {
    search: gix_attrs::Search,
    collection: gix_attrs::search::MetadataCollection,
    ignore_filters: Vec<String>,
    /// root working path
    working_path: PathBuf,
}

impl GitAttributesFile {
    pub fn new(ignore_filters: &[String], working_path: PathBuf) -> Self {
        let base_attributes = Self::default();

        Self {
            ignore_filters: ignore_filters.to_vec(),
            working_path,
            ..base_attributes
        }
    }

    pub fn chain(
        self: &Arc<Self>,
        source_file: PathBuf,
        input: &[u8],
    ) -> Result<Arc<Self>, GitAttributesError> {
        let mut search = self.search.clone();
        let working_path = self.working_path.clone();
        let mut collection = self.collection.clone();
        let ignore_filters = self.ignore_filters.clone();

        search.add_patterns_buffer(
            input,
            source_file,
            Some(&self.working_path),
            &mut collection,
            true,
        );

        Ok(Arc::new(Self {
            search,
            collection,
            ignore_filters,
            working_path,
        }))
    }

    /// Concatenates new `.gitattributes` file.
    pub fn chain_with_file(
        self: &Arc<Self>,
        file: PathBuf,
    ) -> Result<Arc<Self>, GitAttributesError> {
        if file.is_file() {
            let input = std::fs::read(&file).map_err(|err| GitAttributesError::ReadFile {
                path: file.clone(),
                source: err,
            })?;
            self.chain(file, &input)
        } else {
            Ok(self.clone())
        }
    }

    pub fn matches(&self, path: &str) -> bool {
        // If path ends with slash, consider it as a directory.
        let (path, is_dir) = match path.strip_suffix('/') {
            Some(path) => (path, true),
            None => (path, false),
        };

        let mut out = gix_attrs::search::Outcome::default();
        out.initialize_with_selection(&self.collection, ["filter"]);
        self.search.pattern_matching_relative_path(
            path.into(),
            gix_glob::pattern::Case::Sensitive,
            Some(is_dir),
            &mut out,
        );

        let matched = out
            .iter_selected()
            .filter_map(|attr| {
                if let gix_attrs::StateRef::Value(value_ref) = attr.assignment.state {
                    Some(value_ref.as_bstr())
                } else {
                    None
                }
            })
            .any(|value| self.ignore_filters.iter().any(|state| value == state));
        matched
    }
}

impl Default for GitAttributesFile {
    fn default() -> Self {
        let files = [
            gix_attrs::Source::GitInstallation,
            gix_attrs::Source::System,
            gix_attrs::Source::Git,
            gix_attrs::Source::Local,
        ]
        .iter()
        .filter_map(|source| {
            source
                .storage_location(&mut gix_path::env::var)
                .and_then(|p| p.is_file().then_some(p))
                .map(Cow::into_owned)
        });

        let mut buf = Vec::new();
        let mut collection = gix_attrs::search::MetadataCollection::default();
        let search = gix_attrs::Search::new_globals(files, &mut buf, &mut collection)
            .unwrap_or_else(|_| gix_attrs::Search::default());
        let ignore_filters = Vec::new();

        Self {
            search,
            collection,
            ignore_filters,
            working_path: PathBuf::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(input: &[u8], path: &str) -> bool {
        let file = Arc::new(GitAttributesFile::new(&["lfs".to_string()], PathBuf::new()))
            .chain(PathBuf::from(".gitattributes"), input)
            .unwrap();
        file.matches(path)
    }

    #[test]
    fn test_gitattributes_empty_file() {
        let file = GitAttributesFile::new(&["lfs".to_string()], PathBuf::new());
        assert!(!file.matches("foo"));
    }

    #[test]
    fn test_gitattributes_simple_match() {
        assert!(matches(b"*.bin filter=lfs\n", "file.bin"));
        assert!(!matches(b"*.bin filter=lfs\n", "file.txt"));
        assert!(!matches(b"*.bin filter=other\n", "file.bin"));
        assert!(!matches(b"*.bin filter=other\n", "path/to/file.bin"));
    }

    #[test]
    fn test_gitattributes_directory_match() {
        // patterns that match a directory do not recursively match paths inside that
        // directory (so using the trailing-slash path/ syntax is pointless in
        // an attributes file; use path/** instead) https://git-scm.com/docs/gitattributes#_description
        assert!(!matches(b"dir/ filter=lfs\n", "dir/file.txt"));
        assert!(matches(b"dir/ filter=lfs\n", "dir/"));
        assert!(!matches(b"dir/ filter=lfs\n", "other/file.txt"));
        assert!(!matches(b"dir/ filter=lfs\n", "dir"));
    }

    #[test]
    fn test_gitattributes_path_match() {
        assert!(matches(
            b"path/to/file.bin filter=lfs\n",
            "path/to/file.bin"
        ));
        assert!(!matches(b"path/to/file.bin filter=lfs\n", "path/file.bin"));
    }

    #[test]
    fn test_gitattributes_wildcard_match() {
        assert!(matches(b"*.bin filter=lfs\n", "file.bin"));
        assert!(matches(b"file.* filter=lfs\n", "file.bin"));
        assert!(matches(b"**/file.bin filter=lfs\n", "path/to/file.bin"));
    }

    #[test]
    fn test_gitattributes_multiple_attributes() {
        let input = b"*.bin filter=lfs diff=binary\n";
        assert!(matches(input, "file.bin"));
        assert!(!matches(b"*.bin diff=binary\n", "file.bin")); // Only testing
                                                               // filter=lfs
    }

    #[test]
    fn test_gitattributes_chained_files() {
        let base = Arc::new(GitAttributesFile::new(
            &["lfs".to_string(), "text".to_string()],
            PathBuf::new(),
        ));
        let with_first = base
            .chain(PathBuf::from(".gitattributes"), b"*.bin filter=lfs\n")
            .unwrap();
        let with_second = with_first
            .chain(
                PathBuf::from("subdir/.gitattributes"),
                b"*.txt filter=text\n",
            )
            .unwrap();

        assert!(with_second.matches("file.bin"));
        assert!(with_second.matches("subdir/file.txt"));
        assert!(!with_second.matches("file.txt")); // Not in subdir
    }

    #[test]
    fn test_gitattributes_negated_pattern() {
        let input = b"*.bin filter=lfs\n!important.bin filter=lfs\n";
        assert!(matches(input, "file.bin"));
        // negative patterns are forbidden
        // https://git-scm.com/docs/gitattributes#_description
        assert!(matches(input, "important.bin"));
    }

    #[test]
    fn test_gitattributes_multiple_filters() {
        // Create a GitAttributesFile with both "lfs" and "git-crypt" as ignore filters
        let file = Arc::new(GitAttributesFile::new(
            &["lfs".to_string(), "git-crypt".to_string()],
            PathBuf::new(),
        ));

        // Test with lfs filter
        let with_lfs = file
            .chain(PathBuf::from(".gitattributes"), b"*.bin filter=lfs\n")
            .unwrap();
        assert!(with_lfs.matches("file.bin"));

        // Test with git-crypt filter
        let with_git_crypt = file
            .chain(
                PathBuf::from(".gitattributes"),
                b"*.secret filter=git-crypt\n",
            )
            .unwrap();
        assert!(with_git_crypt.matches("credentials.secret"));

        // Test with both filters in the same file
        let with_both = file
            .chain(
                PathBuf::from(".gitattributes"),
                b"*.bin filter=lfs\n*.secret filter=git-crypt\n",
            )
            .unwrap();
        assert!(with_both.matches("file.bin"));
        assert!(with_both.matches("credentials.secret"));
        assert!(!with_both.matches("normal.txt"));

        // Test that other filters don't match
        let with_other = file
            .chain(PathBuf::from(".gitattributes"), b"*.txt filter=other\n")
            .unwrap();
        assert!(!with_other.matches("file.txt"));
    }
}
