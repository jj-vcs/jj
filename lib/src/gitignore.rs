// Copyright 2021 The Jujutsu Authors
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

use std::fs;
use std::io;
use std::iter;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use thiserror::Error;

use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;

#[derive(Debug, Error)]
pub enum GitIgnoreError {
    #[error("Failed to read ignore patterns from file {path}")]
    ReadFile { path: PathBuf, source: io::Error },
}

/// Models the effective contents of multiple .gitignore files.
#[derive(Debug)]
pub struct GitIgnoreFile {
    parent: Option<Arc<Self>>,
    matcher: gix_ignore::Search,
    prefix: RepoPathBuf,
}

impl GitIgnoreFile {
    pub fn empty() -> Arc<Self> {
        Arc::new(Self {
            parent: None,
            matcher: gix_ignore::Search::default(),
            prefix: RepoPathBuf::root(),
        })
    }

    /// Concatenates new `.gitignore` content at the `prefix` directory.
    pub fn chain(
        self: &Arc<Self>,
        prefix: &RepoPath,
        ignore_path: &Path,
        input: &[u8],
    ) -> Result<Arc<Self>, GitIgnoreError> {
        // Construct the gix search object.
        let mut matcher = gix_ignore::Search::default();
        // Since we strip the path prefix manually in matches(), the root path
        // shouldn't be set. add_patterns_buffer() expects filesystem path pairs
        // e.g. ignore_path = "/repo/bar/.gitignore" and root = "/repo".
        let root = None;
        matcher.add_patterns_buffer(
            input,
            ignore_path,
            root,
            gix_ignore::search::Ignore {
                support_precious: false,
            },
        );

        let parent = if self.matcher.patterns.is_empty() {
            self.parent.clone() // omit the empty root
        } else {
            Some(self.clone())
        };
        Ok(Arc::new(Self {
            parent,
            matcher,
            prefix: prefix.to_owned(),
        }))
    }

    /// Concatenates new `.gitignore` file at the `prefix` directory.
    pub fn chain_with_file(
        self: &Arc<Self>,
        prefix: &RepoPath,
        file: PathBuf,
    ) -> Result<Arc<Self>, GitIgnoreError> {
        if file.is_file() {
            let buf = fs::read(&file).map_err(|err| GitIgnoreError::ReadFile {
                path: file.clone(),
                source: err,
            })?;
            self.chain(prefix, &file, &buf)
        } else {
            Ok(self.clone())
        }
    }

    /// Returns whether the specified file path should be ignored.
    ///
    /// This method does not directly define which files should not be tracked
    /// in the repository. Instead, it performs a simple matching against the
    /// last applicable .gitignore line.
    ///
    /// This only performs exact matching; callers handle recursion of parent
    /// directories. Callers shouldn't recursively match inside ignored
    /// directories, because all (untracked) child files should also be ignored;
    /// the exact matching logic won't give correct results in that case.
    pub fn matches_file(&self, path: &RepoPath) -> bool {
        self.matches(path, false)
    }

    /// Returns whether the specified directory path should be ignored.
    ///
    /// See [`GitIgnoreFile::matches_file()`] for details.
    pub fn matches_dir(&self, path: &RepoPath) -> bool {
        self.matches(path, true)
    }

    fn matches(&self, path: &RepoPath, is_dir: bool) -> bool {
        for file in iter::successors(Some(self), |file| file.parent.as_deref()) {
            if let Some(relative_path) = path.strip_prefix(&file.prefix)
                && !relative_path.is_root()
            {
                let m = file.matcher.pattern_matching_relative_path(
                    relative_path.as_internal_file_string().as_ref(),
                    Some(is_dir),
                    gix_ignore::glob::pattern::Case::Sensitive,
                );
                if let Some(m) = m {
                    return !m.pattern.is_negative();
                }
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    // Would ideally be a constant, but we can't create a Path at compile time.
    fn ignore_path() -> &'static Path {
        Path::new(".gitignore")
    }

    fn repo_path(value: &str) -> &RepoPath {
        RepoPath::from_internal_string(value).unwrap()
    }

    fn matches(input: &[u8], path: &str) -> bool {
        let file = GitIgnoreFile::empty()
            .chain(RepoPath::root(), ignore_path(), input)
            .unwrap();
        match path.strip_suffix('/') {
            Some(dir) => file.matches_dir(repo_path(dir)),
            None => file.matches_file(repo_path(path)),
        }
    }

    #[test]
    fn test_gitignore_empty_file() {
        let file = GitIgnoreFile::empty();
        assert!(!file.matches_file(repo_path("foo")));
    }

    #[test]
    fn test_gitignore_empty_file_with_prefix() {
        let file = GitIgnoreFile::empty()
            .chain(repo_path("dir"), ignore_path(), b"")
            .unwrap();
        assert!(!file.matches_file(repo_path("dir/foo")));
    }

    #[test]
    fn test_gitignore_literal() {
        let file = GitIgnoreFile::empty()
            .chain(RepoPath::root(), ignore_path(), b"foo\n")
            .unwrap();
        assert!(file.matches_file(repo_path("foo")));
        assert!(file.matches_file(repo_path("dir/foo")));
        assert!(file.matches_file(repo_path("dir/subdir/foo")));
        assert!(!file.matches_file(repo_path("food")));
        assert!(!file.matches_file(repo_path("dir/food")));
    }

    #[test]
    fn test_gitignore_literal_with_prefix() {
        let file = GitIgnoreFile::empty()
            .chain(repo_path("dir"), ignore_path(), b"foo\n")
            .unwrap();
        assert!(file.matches_file(repo_path("dir/foo")));
        assert!(file.matches_file(repo_path("dir/subdir/foo")));
    }

    #[test]
    fn test_gitignore_pattern_same_as_prefix() {
        let file = GitIgnoreFile::empty()
            .chain(repo_path("dir"), ignore_path(), b"dir\n")
            .unwrap();
        assert!(file.matches_file(repo_path("dir/dir")));
        // We don't want the "dir" pattern to apply to the parent directory
        assert!(!file.matches_file(repo_path("dir/foo")));
    }

    #[test]
    fn test_gitignore_rooted_literal() {
        let file = GitIgnoreFile::empty()
            .chain(RepoPath::root(), ignore_path(), b"/foo\n")
            .unwrap();
        assert!(file.matches_file(repo_path("foo")));
        assert!(!file.matches_file(repo_path("dir/foo")));
    }

    #[test]
    fn test_gitignore_rooted_literal_with_prefix() {
        let file = GitIgnoreFile::empty()
            .chain(repo_path("dir"), ignore_path(), b"/foo\n")
            .unwrap();
        assert!(file.matches_file(repo_path("dir/foo")));
        assert!(!file.matches_file(repo_path("dir/subdir/foo")));
    }

    #[test]
    fn test_gitignore_deep_dir() {
        let file = GitIgnoreFile::empty()
            .chain(RepoPath::root(), ignore_path(), b"/dir1/dir2/dir3\n")
            .unwrap();
        assert!(!file.matches_file(repo_path("foo")));
        assert!(!file.matches_dir(repo_path("dir1")));
        assert!(!file.matches_dir(repo_path("dir1/dir2")));
        assert!(file.matches_dir(repo_path("dir1/dir2/dir3")));
        assert!(!file.matches_dir(repo_path("dir1/dir2/dir3/dir4")));
    }

    #[test]
    fn test_gitignore_deep_dir_chained() {
        // Prefix is relative to root, not to parent file
        let file = GitIgnoreFile::empty()
            .chain(RepoPath::root(), ignore_path(), b"/dummy\n")
            .unwrap()
            .chain(repo_path("dir1"), ignore_path(), b"/dummy\n")
            .unwrap()
            .chain(repo_path("dir1/dir2"), ignore_path(), b"/dir3\n")
            .unwrap();
        assert!(!file.matches_file(repo_path("foo")));
        assert!(!file.matches_dir(repo_path("dir1")));
        assert!(!file.matches_dir(repo_path("dir1/dir2")));
        assert!(file.matches_dir(repo_path("dir1/dir2/dir3")));
        assert!(!file.matches_dir(repo_path("dir1/dir2/dir3/dir4")));
    }

    #[test]
    fn test_gitignore_match_only_dir() {
        let file = GitIgnoreFile::empty()
            .chain(RepoPath::root(), ignore_path(), b"/dir/\n")
            .unwrap();
        assert!(!file.matches_file(repo_path("dir")));
        assert!(file.matches_dir(repo_path("dir")));
        assert!(!file.matches_file(repo_path("dir/subdir")));
    }

    #[test]
    fn test_gitignore_unusual_symbols() {
        assert!(matches(b"\\*\n", "*"));
        assert!(!matches(b"\\*\n", "foo"));
        assert!(matches(b"\\!\n", "!"));
        assert!(matches(b"\\?\n", "?"));
        assert!(!matches(b"\\?\n", "x"));
        assert!(matches(b"\\w\n", "w"));
        assert!(matches(b"\\\\\n", "\\"));
        assert!(!matches(b"\\\n", "\\\n"));
        assert!(!matches(b"\\\n", "\n"));
    }

    #[test]
    fn test_gitignore_backslash_path() {
        assert!(!matches(b"/foo/bar", "foo\\bar"));
        assert!(!matches(b"/foo/bar", "foo/bar\\"));

        assert!(!matches(b"/foo/bar/", "foo\\bar/"));
        assert!(!matches(b"/foo/bar/", "foo\\bar\\/"));
    }

    #[test]
    fn test_gitignore_whitespace() {
        assert!(!matches(b" \n", " "));
        assert!(matches(b"\\ \n", " "));
        assert!(!matches(b"\\\\ \n", " "));
        assert!(matches(b" a\n", " a"));
        assert!(matches(b"a b\n", "a b"));
        assert!(matches(b"a b \n", "a b"));
        assert!(!matches(b"a b \n", "a b "));
        assert!(matches(b"a b\\ \\ \n", "a b  "));
        assert!(matches(b"a b\\ \\  \n", "a b  "));
        // Trail CRs at EOL is ignored
        assert!(matches(b"a\r\n", "a"));
        assert!(!matches(b"a\r\n", "a\r"));
        assert!(matches(b"a\r\r\n", "a\r"));
        assert!(!matches(b"a\r\r\n", "a"));
        assert!(!matches(b"a\r\r\n", "a\r\r"));
        assert!(!matches(b"a\r\r\n", "a"));
        assert!(matches(b"\ra\n", "\ra"));
        assert!(!matches(b"\ra\n", "a"));
    }

    #[test]
    fn test_gitignore_glob() {
        assert!(!matches(b"*.o\n", "foo"));
        assert!(matches(b"*.o\n", "foo.o"));
        assert!(!matches(b"foo.?\n", "foo"));
        assert!(!matches(b"foo.?\n", "foo."));
        assert!(matches(b"foo.?\n", "foo.o"));
    }

    #[test]
    fn test_gitignore_range() {
        assert!(!matches(b"foo.[az]\n", "foo"));
        assert!(matches(b"foo.[az]\n", "foo.a"));
        assert!(!matches(b"foo.[az]\n", "foo.g"));
        assert!(matches(b"foo.[az]\n", "foo.z"));
        assert!(!matches(b"foo.[a-z]\n", "foo"));
        assert!(matches(b"foo.[a-z]\n", "foo.a"));
        assert!(matches(b"foo.[a-z]\n", "foo.g"));
        assert!(matches(b"foo.[a-z]\n", "foo.z"));
        assert!(matches(b"foo.[0-9a-fA-F]\n", "foo.5"));
        assert!(matches(b"foo.[0-9a-fA-F]\n", "foo.c"));
        assert!(matches(b"foo.[0-9a-fA-F]\n", "foo.E"));
        assert!(!matches(b"foo.[0-9a-fA-F]\n", "foo._"));
    }

    #[test]
    fn test_gitignore_leading_dir_glob() {
        let file1 = GitIgnoreFile::empty()
            .chain(RepoPath::root(), ignore_path(), b"**/foo\n")
            .unwrap();
        assert!(file1.matches_file(repo_path("foo")));
        assert!(file1.matches_file(repo_path("dir1/dir2/foo")));
        assert!(!file1.matches_file(repo_path("foo/file")));

        let file2 = file1
            .chain(RepoPath::root(), ignore_path(), b"**/foo\n")
            .unwrap();
        assert!(file2.matches_file(repo_path("dir/foo")));
        assert!(file2.matches_file(repo_path("dir1/dir2/dir/foo")));
    }

    #[test]
    fn test_gitignore_leading_dir_glob_with_prefix() {
        let file = GitIgnoreFile::empty()
            .chain(repo_path("dir1/dir2"), ignore_path(), b"**/foo\n")
            .unwrap();
        assert!(file.matches_file(repo_path("dir1/dir2/foo")));
        assert!(!file.matches_file(repo_path("dir1/dir2/bar")));
        assert!(file.matches_file(repo_path("dir1/dir2/sub1/sub2/foo")));
        assert!(!file.matches_file(repo_path("dir1/dir2/sub1/sub2/bar")));
    }

    #[test]
    fn test_gitignore_trailing_dir_glob() {
        assert!(!matches(b"abc/**\n", "abc"));
        assert!(matches(b"abc/**\n", "abc/file"));
        assert!(matches(b"abc/**\n", "abc/dir/file"));
    }

    #[test]
    fn test_gitignore_internal_dir_glob() {
        assert!(matches(b"a/**/b\n", "a/b"));
        assert!(matches(b"a/**/b\n", "a/x/b"));
        assert!(matches(b"a/**/b\n", "a/x/y/b"));
        assert!(!matches(b"a/**/b\n", "ax/y/b"));
        assert!(!matches(b"a/**/b\n", "a/x/yb"));
        assert!(!matches(b"a/**/b\n", "ab"));
    }

    #[test]
    fn test_gitignore_internal_dir_glob_not_really() {
        assert!(!matches(b"a/x**y/b\n", "a/b"));
        assert!(matches(b"a/x**y/b\n", "a/xy/b"));
        assert!(matches(b"a/x**y/b\n", "a/xzzzy/b"));
    }

    #[test]
    fn test_gitignore_glob_all_root() {
        let file = GitIgnoreFile::empty()
            .chain(RepoPath::root(), ignore_path(), b"*\n")
            .unwrap();
        assert!(!file.matches_dir(RepoPath::root()));
        assert!(file.matches_file(repo_path("foo")));
        assert!(file.matches_dir(repo_path("foo")));
        assert!(file.matches_file(repo_path("foo/bar")));
        assert!(file.matches_dir(repo_path("foo/bar")));
    }

    #[test]
    fn test_gitignore_glob_all_subdir() {
        let file = GitIgnoreFile::empty()
            .chain(repo_path("foo"), ignore_path(), b"*\n")
            .unwrap();
        assert!(!file.matches_dir(RepoPath::root()));
        assert!(!file.matches_file(repo_path("foo")));
        assert!(!file.matches_dir(repo_path("foo")));
        assert!(file.matches_file(repo_path("foo/bar")));
        assert!(file.matches_dir(repo_path("foo/bar")));
        assert!(!file.matches_file(repo_path("bar/baz")));
        assert!(!file.matches_dir(repo_path("bar/baz")));
    }

    #[test]
    fn test_gitignore_with_utf8_bom() {
        assert!(matches(b"\xef\xbb\xbffoo\n", "foo"));
        assert!(!matches(b"\n\xef\xbb\xbffoo\n", "foo"));
    }

    #[test]
    fn test_gitignore_line_ordering() {
        let file1 = GitIgnoreFile::empty()
            .chain(RepoPath::root(), ignore_path(), b"foo*\n!foobar*\n")
            .unwrap();
        assert!(file1.matches_file(repo_path("foo")));
        assert!(!file1.matches_file(repo_path("foobar")));
        assert!(!file1.matches_file(repo_path("foobarbaz")));

        let file2 = GitIgnoreFile::empty()
            .chain(
                RepoPath::root(),
                ignore_path(),
                b"foo*\n!foobar*\nfoobarbaz",
            )
            .unwrap();
        assert!(file2.matches_file(repo_path("foo")));
        assert!(!file2.matches_file(repo_path("foobar")));
        assert!(file2.matches_file(repo_path("foobarbaz")));
        assert!(!file2.matches_file(repo_path("foobarquux")));

        let file3 = GitIgnoreFile::empty()
            .chain(RepoPath::root(), ignore_path(), b"foo/*\n!foo/bar")
            .unwrap();
        assert!(file3.matches_file(repo_path("foo/baz")));
        assert!(!file3.matches_file(repo_path("foo/bar")));
    }

    #[test]
    fn test_gitignore_file_ordering() {
        let file1 = GitIgnoreFile::empty()
            .chain(RepoPath::root(), ignore_path(), b"/foo\n")
            .unwrap();
        assert!(file1.matches_file(repo_path("foo")));
        assert!(!file1.matches_file(repo_path("foo/bar")));
        assert!(!file1.matches_file(repo_path("foo/bar/baz")));

        let file2 = file1
            .chain(repo_path("foo"), ignore_path(), b"!/bar")
            .unwrap();
        assert!(file1.matches_dir(repo_path("foo")));
        assert!(!file2.matches_file(repo_path("foo/bar")));
        assert!(!file2.matches_file(repo_path("foo/bar/baz")));
        assert!(!file2.matches_file(repo_path("foo/baz")));

        let file3 = file2
            .chain(repo_path("foo/bar"), ignore_path(), b"/baz")
            .unwrap();
        assert!(!file2.matches_dir(repo_path("foo/bar")));
        assert!(file3.matches_file(repo_path("foo/bar/baz")));
        assert!(!file3.matches_file(repo_path("foo/bar/qux")));
    }

    #[test]
    fn test_gitignore_slash_after_glob() {
        let file = GitIgnoreFile::empty()
            .chain(RepoPath::root(), ignore_path(), b"/*/\n")
            .unwrap();
        assert!(!file.matches_file(repo_path("foo")));
        assert!(file.matches_dir(repo_path("foo")));
        assert!(!file.matches_file(repo_path("foo/bar")));
        assert!(!file.matches_file(repo_path("foo/bar/baz")));
    }

    #[test]
    fn test_gitignore_negative_parent_directory() {
        // The following script shows that Git ignores the file:
        //
        // ```bash
        // $ rm -rf test-repo && \
        //   git init test-repo &>/dev/null && \
        //   cd test-repo && \
        //   printf 'A/B.*\n!/A/\n' >.gitignore && \
        //   mkdir A && \
        //   touch A/B.ext && \
        //   git check-ignore A/B.ext
        // A/B.ext
        // ```
        let ignore = GitIgnoreFile::empty()
            .chain(RepoPath::root(), ignore_path(), b"foo/bar.*\n!/foo/\n")
            .unwrap();
        assert!(ignore.matches_file(repo_path("foo/bar.ext")));

        let ignore = GitIgnoreFile::empty()
            .chain(RepoPath::root(), ignore_path(), b"!/foo/\nfoo/bar.*\n")
            .unwrap();
        assert!(ignore.matches_file(repo_path("foo/bar.ext")));
    }

    #[test]
    fn test_gitignore_invalid_utf8() {
        // Non-UTF-8 paths should be parsed without an error.
        let ignore = GitIgnoreFile::empty().chain(RepoPath::root(), ignore_path(), &[224]);
        assert!(ignore.is_ok());
    }
}
