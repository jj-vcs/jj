// Copyright 2026 The Jujutsu Authors
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

//! Utilities for converting between `RepoPath`s and plain strings as displayed
//! to the user (e.g. relative to CWD).

use std::iter;
use std::path::PathBuf;

use thiserror::Error;

use crate::file_util;
use crate::merge::Diff;
use crate::repo_path::FsPathParseError;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;

/// An error from `RepoPathUiConverter::parse_file_path`.
#[derive(Debug, Error)]
pub enum UiPathParseError {
    /// Failure to parse a path a relative path inside the repo.
    #[error(transparent)]
    Fs(FsPathParseError),
}

/// Converts `RepoPath`s to and from plain strings as displayed to the user
/// (e.g. relative to CWD).
#[derive(Debug, Clone)]
pub enum RepoPathUiConverter {
    /// Variant for a local file system. Paths are interpreted relative to `cwd`
    /// with the repo rooted in `base`.
    ///
    /// The `cwd` and `base` paths are supposed to be absolute and normalized in
    /// the same manner.
    Fs {
        /// The directory to which relative paths are interpreted.
        cwd: PathBuf,
        /// The repository root path.
        base: PathBuf,
    },
    // TODO: Add a no-op variant that uses the internal `RepoPath` representation. Can be useful
    // on a server.
}

impl RepoPathUiConverter {
    /// Format a path for display in the UI.
    pub fn format_file_path(&self, file: &RepoPath) -> String {
        match self {
            Self::Fs { cwd, base } => {
                file_util::relative_path(cwd, &file.to_fs_path_unchecked(base))
                    .display()
                    .to_string()
            }
        }
    }

    /// Format a copy from `before` to `after` for display in the UI by
    /// extracting common components and producing something like
    /// "common/prefix/{before => after}/common/suffix".
    ///
    /// If `before == after`, this is equivalent to `format_file_path()`.
    pub fn format_copied_path(&self, paths: Diff<&RepoPath>) -> String {
        match self {
            Self::Fs { .. } => {
                let paths = paths.map(|path| self.format_file_path(path));
                collapse_copied_path(paths.as_deref(), std::path::MAIN_SEPARATOR)
            }
        }
    }

    /// Parses a path from the UI.
    ///
    /// It's up to the implementation whether absolute paths are allowed, and
    /// where relative paths are interpreted as relative to.
    pub fn parse_file_path(&self, input: &str) -> Result<RepoPathBuf, UiPathParseError> {
        match self {
            Self::Fs { cwd, base } => {
                RepoPathBuf::parse_fs_path(cwd, base, input).map_err(UiPathParseError::Fs)
            }
        }
    }
}

fn collapse_copied_path(paths: Diff<&str>, separator: char) -> String {
    // The last component should never match middle components. This is ensured
    // by including trailing separators. e.g. ("a/b", "a/b/x") => ("a/", _)
    let components = paths.map(|path| path.split_inclusive(separator));
    let prefix_len: usize = iter::zip(components.before, components.after)
        .take_while(|(before, after)| before == after)
        .map(|(_, after)| after.len())
        .sum();
    if paths.before.len() == prefix_len && paths.after.len() == prefix_len {
        return paths.after.to_owned();
    }

    // The first component should never match middle components, but the first
    // uncommon middle component can. e.g. ("a/b", "x/a/b") => ("", "/b"),
    // ("a/b", "a/x/b") => ("a/", "/b")
    let components = paths.map(|path| {
        let mut remainder = &path[prefix_len.saturating_sub(1)..];
        iter::from_fn(move || {
            let pos = remainder.rfind(separator)?;
            let (prefix, last) = remainder.split_at(pos);
            remainder = prefix;
            Some(last)
        })
    });
    let suffix_len: usize = iter::zip(components.before, components.after)
        .take_while(|(before, after)| before == after)
        .map(|(_, after)| after.len())
        .sum();

    // Middle range may be invalid (start > end) because the same separator char
    // can be distributed to both common prefix and suffix. e.g.
    // ("a/b", "a/x/b") == ("a//b", "a/x/b") => ("a/", "/b")
    let middle = paths.map(|path| path.get(prefix_len..path.len() - suffix_len).unwrap_or(""));

    let mut collapsed = String::new();
    collapsed.push_str(&paths.after[..prefix_len]);
    collapsed.push('{');
    collapsed.push_str(middle.before);
    collapsed.push_str(" => ");
    collapsed.push_str(middle.after);
    collapsed.push('}');
    collapsed.push_str(&paths.after[paths.after.len() - suffix_len..]);
    collapsed
}

#[cfg(test)]
mod tests {

    use super::*;

    fn repo_path(value: &str) -> &RepoPath {
        RepoPath::from_internal_string(value).unwrap()
    }

    #[test]
    fn test_format_copied_path() {
        let ui = RepoPathUiConverter::Fs {
            cwd: PathBuf::from("."),
            base: PathBuf::from("."),
        };

        let format = |before, after| {
            ui.format_copied_path(Diff::new(repo_path(before), repo_path(after)))
                .replace('\\', "/")
        };

        assert_eq!(format("one/two/three", "one/two/three"), "one/two/three");
        assert_eq!(format("one/two", "one/two/three"), "one/{two => two/three}");
        assert_eq!(format("one/two", "zero/one/two"), "{one => zero/one}/two");
        assert_eq!(format("one/two/three", "one/two"), "one/{two/three => two}");
        assert_eq!(format("zero/one/two", "one/two"), "{zero/one => one}/two");
        assert_eq!(
            format("one/two", "one/two/three/one/two"),
            "one/{ => two/three/one}/two"
        );

        assert_eq!(format("two/three", "four/three"), "{two => four}/three");
        assert_eq!(
            format("one/two/three", "one/four/three"),
            "one/{two => four}/three"
        );
        assert_eq!(format("one/two/three", "one/three"), "one/{two => }/three");
        assert_eq!(format("one/two", "one/four"), "one/{two => four}");
        assert_eq!(format("two", "four"), "{two => four}");
        assert_eq!(format("file1", "file2"), "{file1 => file2}");
        assert_eq!(format("file-1", "file-2"), "{file-1 => file-2}");
        assert_eq!(
            format("x/something/something/2to1.txt", "x/something/2to1.txt"),
            "x/something/{something => }/2to1.txt"
        );
        assert_eq!(
            format("x/something/1to2.txt", "x/something/something/1to2.txt"),
            "x/something/{ => something}/1to2.txt"
        );
    }

    #[test]
    fn test_split_common_prefix() {
        assert_eq!(
            repo_path("foo/bar").split_common_prefix(repo_path("foo/bar/baz")),
            (repo_path("foo/bar"), repo_path(""))
        );

        assert_eq!(
            repo_path("foo/bar/baz").split_common_prefix(repo_path("foo/bar")),
            (repo_path("foo/bar"), repo_path("baz"))
        );

        assert_eq!(
            repo_path("foo/bar/bing").split_common_prefix(repo_path("foo/bar/baz")),
            (repo_path("foo/bar"), repo_path("bing"))
        );

        assert_eq!(
            repo_path("no/common/prefix").split_common_prefix(repo_path("foo/bar/baz")),
            (RepoPath::root(), repo_path("no/common/prefix"))
        );

        assert_eq!(
            repo_path("same/path").split_common_prefix(repo_path("same/path")),
            (repo_path("same/path"), RepoPath::root())
        );

        assert_eq!(
            RepoPath::root().split_common_prefix(repo_path("foo")),
            (RepoPath::root(), RepoPath::root())
        );

        assert_eq!(
            RepoPath::root().split_common_prefix(RepoPath::root()),
            (RepoPath::root(), RepoPath::root())
        );

        assert_eq!(
            repo_path("foo/bar").split_common_prefix(RepoPath::root()),
            (RepoPath::root(), repo_path("foo/bar"))
        );
    }
}
