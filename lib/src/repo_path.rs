// Copyright 2020 The Jujutsu Authors
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

use std::borrow::Borrow;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::fmt::Debug;
use std::fmt::Formatter;
use std::iter;
use std::iter::FusedIterator;
use std::ops::Deref;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use itertools::Itertools as _;
use ref_cast::RefCastCustom;
use ref_cast::ref_cast_custom;
use thiserror::Error;

use crate::content_hash::ContentHash;
use crate::file_util;
use crate::merge::Diff;

/// Owned `RepoPath` component.
#[derive(ContentHash, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepoPathComponentBuf {
    // Don't add more fields. Eq, Hash, and Ord must be compatible with the
    // borrowed RepoPathComponent type.
    value: String,
}

impl RepoPathComponentBuf {
    /// Wraps `value` as `RepoPathComponentBuf`.
    ///
    /// Returns an error if the input `value` is empty or contains path
    /// separator.
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidNewRepoPathError> {
        let value: String = value.into();
        if is_valid_repo_path_component_str(&value) {
            Ok(Self { value })
        } else {
            Err(InvalidNewRepoPathError { value })
        }
    }
}

/// Borrowed `RepoPath` component.
#[derive(PartialEq, Eq, PartialOrd, Ord, Hash, RefCastCustom)]
#[repr(transparent)]
pub struct RepoPathComponent {
    value: str,
}

impl RepoPathComponent {
    /// Wraps `value` as `RepoPathComponent`.
    ///
    /// Returns an error if the input `value` is empty or contains path
    /// separator.
    pub fn new(value: &str) -> Result<&Self, InvalidNewRepoPathError> {
        if is_valid_repo_path_component_str(value) {
            Ok(Self::new_unchecked(value))
        } else {
            Err(InvalidNewRepoPathError {
                value: value.to_string(),
            })
        }
    }

    #[ref_cast_custom]
    const fn new_unchecked(value: &str) -> &Self;

    /// Returns the underlying string representation.
    pub fn as_internal_str(&self) -> &str {
        &self.value
    }

    /// Returns a normal filesystem entry name if this path component is valid
    /// as a file/directory name.
    pub fn to_fs_name(&self) -> Result<&str, InvalidRepoPathComponentError> {
        let mut components = Path::new(&self.value).components().fuse();
        match (components.next(), components.next()) {
            // Trailing "." can be normalized by Path::components(), so compare
            // component name. e.g. "foo\." (on Windows) should be rejected.
            (Some(Component::Normal(name)), None) if name == &self.value => Ok(&self.value),
            // e.g. ".", "..", "foo\bar" (on Windows)
            _ => Err(InvalidRepoPathComponentError {
                component: self.value.into(),
            }),
        }
    }
}

impl Debug for RepoPathComponent {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", &self.value)
    }
}

impl Debug for RepoPathComponentBuf {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        <RepoPathComponent as Debug>::fmt(self, f)
    }
}

impl AsRef<Self> for RepoPathComponent {
    fn as_ref(&self) -> &Self {
        self
    }
}

impl AsRef<RepoPathComponent> for RepoPathComponentBuf {
    fn as_ref(&self) -> &RepoPathComponent {
        self
    }
}

impl Borrow<RepoPathComponent> for RepoPathComponentBuf {
    fn borrow(&self) -> &RepoPathComponent {
        self
    }
}

impl Deref for RepoPathComponentBuf {
    type Target = RepoPathComponent;

    fn deref(&self) -> &Self::Target {
        RepoPathComponent::new_unchecked(&self.value)
    }
}

impl ToOwned for RepoPathComponent {
    type Owned = RepoPathComponentBuf;

    fn to_owned(&self) -> Self::Owned {
        let value = self.value.to_owned();
        RepoPathComponentBuf { value }
    }

    fn clone_into(&self, target: &mut Self::Owned) {
        self.value.clone_into(&mut target.value);
    }
}

/// Iterator over `RepoPath` components.
#[derive(Clone, Debug)]
pub struct RepoPathComponentsIter<'a> {
    value: &'a str,
}

impl<'a> RepoPathComponentsIter<'a> {
    /// Returns the remaining part as repository path.
    pub fn as_path(&self) -> &'a RepoPath {
        RepoPath::from_internal_string_unchecked(self.value)
    }
}

impl<'a> Iterator for RepoPathComponentsIter<'a> {
    type Item = &'a RepoPathComponent;

    fn next(&mut self) -> Option<Self::Item> {
        if self.value.is_empty() {
            return None;
        }
        let (name, remainder) = self
            .value
            .split_once('/')
            .unwrap_or_else(|| (self.value, &self.value[self.value.len()..]));
        self.value = remainder;
        Some(RepoPathComponent::new_unchecked(name))
    }
}

impl DoubleEndedIterator for RepoPathComponentsIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.value.is_empty() {
            return None;
        }
        let (remainder, name) = self
            .value
            .rsplit_once('/')
            .unwrap_or_else(|| (&self.value[..0], self.value));
        self.value = remainder;
        Some(RepoPathComponent::new_unchecked(name))
    }
}

impl FusedIterator for RepoPathComponentsIter<'_> {}

/// Owned repository path.
#[derive(ContentHash, Clone, Eq, Hash, PartialEq, serde::Serialize)]
#[serde(transparent)]
pub struct RepoPathBuf {
    // Don't add more fields. Eq, Hash, and Ord must be compatible with the
    // borrowed RepoPath type.
    value: String,
}

/// Borrowed repository path.
#[derive(ContentHash, Eq, Hash, PartialEq, RefCastCustom, serde::Serialize)]
#[repr(transparent)]
#[serde(transparent)]
pub struct RepoPath {
    value: str,
}

impl Debug for RepoPath {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", &self.value)
    }
}

impl Debug for RepoPathBuf {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        <RepoPath as Debug>::fmt(self, f)
    }
}

/// The `value` is not a valid repo path because it contains empty path
/// component. For example, `"/"`, `"/foo"`, `"foo/"`, `"foo//bar"` are all
/// invalid.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error(r#"Invalid repo path input "{value}""#)]
pub struct InvalidNewRepoPathError {
    value: String,
}

impl RepoPathBuf {
    /// Creates owned repository path pointing to the root.
    pub const fn root() -> Self {
        Self {
            value: String::new(),
        }
    }

    /// Creates `RepoPathBuf` from valid string representation.
    pub fn from_internal_string(value: impl Into<String>) -> Result<Self, InvalidNewRepoPathError> {
        let value: String = value.into();
        if is_valid_repo_path_str(&value) {
            Ok(Self { value })
        } else {
            Err(InvalidNewRepoPathError { value })
        }
    }

    /// Converts repo-relative `Path` to `RepoPathBuf`.
    ///
    /// The input path should not contain redundant `.` or `..`.
    pub fn from_relative_path(
        relative_path: impl AsRef<Path>,
    ) -> Result<Self, RelativePathParseError> {
        let relative_path = relative_path.as_ref();
        if relative_path == Path::new(".") {
            return Ok(Self::root());
        }

        let mut components = relative_path
            .components()
            .map(|c| match c {
                Component::Normal(name) => {
                    name.to_str()
                        .ok_or_else(|| RelativePathParseError::InvalidUtf8 {
                            path: relative_path.into(),
                        })
                }
                _ => Err(RelativePathParseError::InvalidComponent {
                    component: c.as_os_str().to_string_lossy().into(),
                    path: relative_path.into(),
                }),
            })
            .fuse();
        let mut value = String::with_capacity(relative_path.as_os_str().len());
        if let Some(name) = components.next() {
            value.push_str(name?);
        }
        for name in components {
            value.push('/');
            value.push_str(name?);
        }
        Ok(Self { value })
    }

    /// Parses an `input` path into a `RepoPathBuf` relative to `base`.
    ///
    /// The `cwd` and `base` paths are supposed to be absolute and normalized in
    /// the same manner. The `input` path may be either relative to `cwd` or
    /// absolute.
    pub fn parse_fs_path(
        cwd: &Path,
        base: &Path,
        input: impl AsRef<Path>,
    ) -> Result<Self, FsPathParseError> {
        let input = input.as_ref();
        let abs_input_path = file_util::normalize_path(&cwd.join(input));
        let repo_relative_path = file_util::relative_path(base, &abs_input_path);
        Self::from_relative_path(repo_relative_path).map_err(|source| FsPathParseError {
            base: file_util::relative_path(cwd, base).into(),
            input: input.into(),
            source,
        })
    }

    /// Consumes this and returns the underlying string representation.
    pub fn into_internal_string(self) -> String {
        self.value
    }
}

impl RepoPath {
    /// Returns repository path pointing to the root.
    pub const fn root() -> &'static Self {
        Self::from_internal_string_unchecked("")
    }

    /// Wraps valid string representation as `RepoPath`.
    ///
    /// Returns an error if the input `value` contains empty path component. For
    /// example, `"/"`, `"/foo"`, `"foo/"`, `"foo//bar"` are all invalid.
    pub fn from_internal_string(value: &str) -> Result<&Self, InvalidNewRepoPathError> {
        if is_valid_repo_path_str(value) {
            Ok(Self::from_internal_string_unchecked(value))
        } else {
            Err(InvalidNewRepoPathError {
                value: value.to_owned(),
            })
        }
    }

    #[ref_cast_custom]
    const fn from_internal_string_unchecked(value: &str) -> &Self;

    /// The full string form used internally, not for presenting to users (where
    /// we may want to use the platform's separator). This format includes a
    /// trailing slash, unless this path represents the root directory. That
    /// way it can be concatenated with a basename and produce a valid path.
    pub fn to_internal_dir_string(&self) -> String {
        if self.value.is_empty() {
            String::new()
        } else {
            [&self.value, "/"].concat()
        }
    }

    /// The full string form used internally, not for presenting to users (where
    /// we may want to use the platform's separator).
    pub fn as_internal_file_string(&self) -> &str {
        &self.value
    }

    /// Converts repository path to filesystem path relative to the `base`.
    ///
    /// The returned path should never contain `..`, `C:` (on Windows), etc.
    /// However, it may contain reserved working-copy directories such as `.jj`.
    pub fn to_fs_path(&self, base: &Path) -> Result<PathBuf, InvalidRepoPathError> {
        let mut result = PathBuf::with_capacity(base.as_os_str().len() + self.value.len() + 1);
        result.push(base);
        for c in self.components() {
            result.push(c.to_fs_name().map_err(|err| err.with_path(self))?);
        }
        if result.as_os_str().is_empty() {
            result.push(".");
        }
        Ok(result)
    }

    /// Converts repository path to filesystem path relative to the `base`,
    /// without checking invalid path components.
    ///
    /// The returned path may point outside of the `base` directory. Use this
    /// function only for displaying or testing purposes.
    pub fn to_fs_path_unchecked(&self, base: &Path) -> PathBuf {
        let mut result = PathBuf::with_capacity(base.as_os_str().len() + self.value.len() + 1);
        result.push(base);
        result.extend(self.components().map(RepoPathComponent::as_internal_str));
        if result.as_os_str().is_empty() {
            result.push(".");
        }
        result
    }

    pub fn is_root(&self) -> bool {
        self.value.is_empty()
    }

    /// Returns true if the `base` is a prefix of this path.
    pub fn starts_with(&self, base: &Self) -> bool {
        self.strip_prefix(base).is_some()
    }

    /// Returns the remaining path with the `base` path removed.
    pub fn strip_prefix(&self, base: &Self) -> Option<&Self> {
        if base.value.is_empty() {
            Some(self)
        } else {
            let tail = self.value.strip_prefix(&base.value)?;
            if tail.is_empty() {
                Some(Self::from_internal_string_unchecked(tail))
            } else {
                tail.strip_prefix('/')
                    .map(Self::from_internal_string_unchecked)
            }
        }
    }

    /// Returns the parent path without the base name component.
    pub fn parent(&self) -> Option<&Self> {
        self.split().map(|(parent, _)| parent)
    }

    /// Splits this into the parent path and base name component.
    pub fn split(&self) -> Option<(&Self, &RepoPathComponent)> {
        let mut components = self.components();
        let basename = components.next_back()?;
        Some((components.as_path(), basename))
    }

    pub fn components(&self) -> RepoPathComponentsIter<'_> {
        RepoPathComponentsIter { value: &self.value }
    }

    pub fn ancestors(&self) -> impl Iterator<Item = &Self> {
        std::iter::successors(Some(self), |path| path.parent())
    }

    pub fn join(&self, entry: &RepoPathComponent) -> RepoPathBuf {
        let value = if self.value.is_empty() {
            entry.as_internal_str().to_owned()
        } else {
            [&self.value, "/", entry.as_internal_str()].concat()
        };
        RepoPathBuf { value }
    }

    /// Splits this path at its common prefix with `other`.
    ///
    /// # Returns
    ///
    /// Returns the `(common_prefix, self_remainder)`.
    ///
    /// All paths will at least have `RepoPath::root()` as a common prefix,
    /// therefore even if `self` and `other` have no matching parent component
    /// this function will always return at least `(RepoPath::root(), self)`.
    ///
    ///
    /// # Examples
    ///
    /// ```
    /// use jj_lib::repo_path::RepoPath;
    ///
    /// let bing_path = RepoPath::from_internal_string("foo/bar/bing").unwrap();
    ///
    /// let baz_path = RepoPath::from_internal_string("foo/bar/baz").unwrap();
    ///
    /// let foo_bar_path = RepoPath::from_internal_string("foo/bar").unwrap();
    ///
    /// assert_eq!(
    ///     bing_path.split_common_prefix(&baz_path),
    ///     (foo_bar_path, RepoPath::from_internal_string("bing").unwrap())
    /// );
    ///
    /// let unrelated_path = RepoPath::from_internal_string("no/common/prefix").unwrap();
    /// assert_eq!(
    ///     baz_path.split_common_prefix(&unrelated_path),
    ///     (RepoPath::root(), baz_path)
    /// );
    /// ```
    pub fn split_common_prefix(&self, other: &Self) -> (&Self, &Self) {
        // Obtain the common prefix between these paths
        let mut prefix_len = 0;

        let common_components = self
            .components()
            .zip(other.components())
            .take_while(|(prev_comp, this_comp)| prev_comp == this_comp);

        for (self_comp, _other_comp) in common_components {
            if prefix_len > 0 {
                // + 1 for all paths to take their separators into account.
                // We skip the first one since there are ComponentCount - 1 separators in a
                // path.
                prefix_len += 1;
            }

            prefix_len += self_comp.value.len();
        }

        if prefix_len == 0 {
            // No common prefix except root
            return (Self::root(), self);
        }

        if prefix_len == self.value.len() {
            return (self, Self::root());
        }

        let common_prefix = Self::from_internal_string_unchecked(&self.value[..prefix_len]);
        let remainder = Self::from_internal_string_unchecked(&self.value[prefix_len + 1..]);

        (common_prefix, remainder)
    }
}

impl AsRef<Self> for RepoPath {
    fn as_ref(&self) -> &Self {
        self
    }
}

impl AsRef<RepoPath> for RepoPathBuf {
    fn as_ref(&self) -> &RepoPath {
        self
    }
}

impl Borrow<RepoPath> for RepoPathBuf {
    fn borrow(&self) -> &RepoPath {
        self
    }
}

impl Deref for RepoPathBuf {
    type Target = RepoPath;

    fn deref(&self) -> &Self::Target {
        RepoPath::from_internal_string_unchecked(&self.value)
    }
}

impl ToOwned for RepoPath {
    type Owned = RepoPathBuf;

    fn to_owned(&self) -> Self::Owned {
        let value = self.value.to_owned();
        RepoPathBuf { value }
    }

    fn clone_into(&self, target: &mut Self::Owned) {
        self.value.clone_into(&mut target.value);
    }
}

impl Ord for RepoPath {
    fn cmp(&self, other: &Self) -> Ordering {
        // If there were leading/trailing slash, components-based Ord would
        // disagree with str-based Eq.
        debug_assert!(is_valid_repo_path_str(&self.value));
        self.components().cmp(other.components())
    }
}

impl Ord for RepoPathBuf {
    fn cmp(&self, other: &Self) -> Ordering {
        <RepoPath as Ord>::cmp(self, other)
    }
}

impl PartialOrd for RepoPath {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialOrd for RepoPathBuf {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<P: AsRef<RepoPathComponent>> Extend<P> for RepoPathBuf {
    fn extend<T: IntoIterator<Item = P>>(&mut self, iter: T) {
        for component in iter {
            if !self.value.is_empty() {
                self.value.push('/');
            }
            self.value.push_str(component.as_ref().as_internal_str());
        }
    }
}

/// `RepoPath` contained invalid file/directory component such as `..`.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error(r#"Invalid repository path "{}""#, path.as_internal_file_string())]
pub struct InvalidRepoPathError {
    /// Path containing an error.
    pub path: RepoPathBuf,
    /// Source error.
    pub source: InvalidRepoPathComponentError,
}

/// `RepoPath` component was invalid. (e.g. `..`)
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error(r#"Invalid path component "{component}""#)]
pub struct InvalidRepoPathComponentError {
    pub component: Box<str>,
}

impl InvalidRepoPathComponentError {
    /// Attaches the `path` that caused the error.
    pub fn with_path(self, path: &RepoPath) -> InvalidRepoPathError {
        InvalidRepoPathError {
            path: path.to_owned(),
            source: self,
        }
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum RelativePathParseError {
    #[error(r#"Invalid component "{component}" in repo-relative path "{path}""#)]
    InvalidComponent {
        component: Box<str>,
        path: Box<Path>,
    },
    #[error(r#"Not valid UTF-8 path "{path}""#)]
    InvalidUtf8 { path: Box<Path> },
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error(r#"Path "{input}" is not in the repo "{base}""#)]
pub struct FsPathParseError {
    /// Repository or workspace root path relative to the `cwd`.
    pub base: Box<Path>,
    /// Input path without normalization.
    pub input: Box<Path>,
    /// Source error.
    pub source: RelativePathParseError,
}

fn is_valid_repo_path_component_str(value: &str) -> bool {
    !value.is_empty() && !value.contains('/')
}

fn is_valid_repo_path_str(value: &str) -> bool {
    !value.starts_with('/') && !value.ends_with('/') && !value.contains("//")
}

/// An error from `RepoPathUiConverter::parse_file_path`.
#[derive(Debug, Error)]
pub enum UiPathParseError {
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
    Fs { cwd: PathBuf, base: PathBuf },
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

/// Tree that maps `RepoPath` to value of type `V`.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct RepoPathTree<V> {
    entries: HashMap<RepoPathComponentBuf, Self>,
    value: V,
}

impl<V> RepoPathTree<V> {
    /// The value associated with this path.
    pub fn value(&self) -> &V {
        &self.value
    }

    /// Mutable reference to the value associated with this path.
    pub fn value_mut(&mut self) -> &mut V {
        &mut self.value
    }

    /// Set the value associated with this path.
    pub fn set_value(&mut self, value: V) {
        self.value = value;
    }

    /// The immediate children of this node.
    pub fn children(&self) -> impl Iterator<Item = (&RepoPathComponent, &Self)> {
        self.entries
            .iter()
            .map(|(component, value)| (component.as_ref(), value))
    }

    /// Whether this node has any children.
    pub fn has_children(&self) -> bool {
        !self.entries.is_empty()
    }

    /// Add a path to the tree. Normally called on the root tree.
    pub fn add(&mut self, path: &RepoPath) -> &mut Self
    where
        V: Default,
    {
        path.components().fold(self, |sub, name| {
            // Avoid name.clone() if entry already exists.
            if !sub.entries.contains_key(name) {
                sub.entries.insert(name.to_owned(), Self::default());
            }
            sub.entries.get_mut(name).unwrap()
        })
    }

    /// Get a reference to the node for the given `path`, if it exists in the
    /// tree.
    pub fn get(&self, path: &RepoPath) -> Option<&Self> {
        path.components()
            .try_fold(self, |sub, name| sub.entries.get(name))
    }

    /// Walks the tree from the root to the given `path`, yielding each sub tree
    /// and remaining path.
    pub fn walk_to<'a, 'b>(
        &'a self,
        path: &'b RepoPath,
    ) -> impl Iterator<Item = (&'a Self, &'b RepoPath)> {
        iter::successors(Some((self, path)), |(sub, path)| {
            let mut components = path.components();
            let name = components.next()?;
            Some((sub.entries.get(name)?, components.as_path()))
        })
    }
}

impl<V: Debug> Debug for RepoPathTree<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)?;
        f.write_str(" ")?;
        f.debug_map()
            .entries(
                self.entries
                    .iter()
                    .sorted_unstable_by_key(|&(name, _)| name),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::panic;

    use assert_matches::assert_matches;
    use itertools::Itertools as _;

    use super::*;
    use crate::tests::new_temp_dir;

    fn repo_path(value: &str) -> &RepoPath {
        RepoPath::from_internal_string(value).unwrap()
    }

    fn repo_path_component(value: &str) -> &RepoPathComponent {
        RepoPathComponent::new(value).unwrap()
    }

    #[test]
    fn test_is_root() {
        assert!(RepoPath::root().is_root());
        assert!(repo_path("").is_root());
        assert!(!repo_path("foo").is_root());
    }

    #[test]
    fn test_from_internal_string() {
        let repo_path_buf = |value: &str| RepoPathBuf::from_internal_string(value).unwrap();
        assert_eq!(repo_path_buf(""), RepoPathBuf::root());
        assert!(panic::catch_unwind(|| repo_path_buf("/")).is_err());
        assert!(panic::catch_unwind(|| repo_path_buf("/x")).is_err());
        assert!(panic::catch_unwind(|| repo_path_buf("x/")).is_err());
        assert!(panic::catch_unwind(|| repo_path_buf("x//y")).is_err());

        assert_eq!(repo_path(""), RepoPath::root());
        assert!(panic::catch_unwind(|| repo_path("/")).is_err());
        assert!(panic::catch_unwind(|| repo_path("/x")).is_err());
        assert!(panic::catch_unwind(|| repo_path("x/")).is_err());
        assert!(panic::catch_unwind(|| repo_path("x//y")).is_err());
    }

    #[test]
    fn test_as_internal_file_string() {
        assert_eq!(RepoPath::root().as_internal_file_string(), "");
        assert_eq!(repo_path("dir").as_internal_file_string(), "dir");
        assert_eq!(repo_path("dir/file").as_internal_file_string(), "dir/file");
    }

    #[test]
    fn test_to_internal_dir_string() {
        assert_eq!(RepoPath::root().to_internal_dir_string(), "");
        assert_eq!(repo_path("dir").to_internal_dir_string(), "dir/");
        assert_eq!(repo_path("dir/file").to_internal_dir_string(), "dir/file/");
    }

    #[test]
    fn test_starts_with() {
        assert!(repo_path("").starts_with(repo_path("")));
        assert!(repo_path("x").starts_with(repo_path("")));
        assert!(!repo_path("").starts_with(repo_path("x")));

        assert!(repo_path("x").starts_with(repo_path("x")));
        assert!(repo_path("x/y").starts_with(repo_path("x")));
        assert!(!repo_path("xy").starts_with(repo_path("x")));
        assert!(!repo_path("x/y").starts_with(repo_path("y")));

        assert!(repo_path("x/y").starts_with(repo_path("x/y")));
        assert!(repo_path("x/y/z").starts_with(repo_path("x/y")));
        assert!(!repo_path("x/yz").starts_with(repo_path("x/y")));
        assert!(!repo_path("x").starts_with(repo_path("x/y")));
        assert!(!repo_path("xy").starts_with(repo_path("x/y")));
    }

    #[test]
    fn test_strip_prefix() {
        assert_eq!(
            repo_path("").strip_prefix(repo_path("")),
            Some(repo_path(""))
        );
        assert_eq!(
            repo_path("x").strip_prefix(repo_path("")),
            Some(repo_path("x"))
        );
        assert_eq!(repo_path("").strip_prefix(repo_path("x")), None);

        assert_eq!(
            repo_path("x").strip_prefix(repo_path("x")),
            Some(repo_path(""))
        );
        assert_eq!(
            repo_path("x/y").strip_prefix(repo_path("x")),
            Some(repo_path("y"))
        );
        assert_eq!(repo_path("xy").strip_prefix(repo_path("x")), None);
        assert_eq!(repo_path("x/y").strip_prefix(repo_path("y")), None);

        assert_eq!(
            repo_path("x/y").strip_prefix(repo_path("x/y")),
            Some(repo_path(""))
        );
        assert_eq!(
            repo_path("x/y/z").strip_prefix(repo_path("x/y")),
            Some(repo_path("z"))
        );
        assert_eq!(repo_path("x/yz").strip_prefix(repo_path("x/y")), None);
        assert_eq!(repo_path("x").strip_prefix(repo_path("x/y")), None);
        assert_eq!(repo_path("xy").strip_prefix(repo_path("x/y")), None);
    }

    #[test]
    fn test_order() {
        assert!(RepoPath::root() < repo_path("dir"));
        assert!(repo_path("dir") < repo_path("dirx"));
        // '#' < '/', but ["dir", "sub"] < ["dir#"]
        assert!(repo_path("dir") < repo_path("dir#"));
        assert!(repo_path("dir") < repo_path("dir/sub"));
        assert!(repo_path("dir/sub") < repo_path("dir#"));

        assert!(repo_path("abc") < repo_path("dir/file"));
        assert!(repo_path("dir") < repo_path("dir/file"));
        assert!(repo_path("dis") > repo_path("dir/file"));
        assert!(repo_path("xyz") > repo_path("dir/file"));
        assert!(repo_path("dir1/xyz") < repo_path("dir2/abc"));
    }

    #[test]
    fn test_join() {
        let root = RepoPath::root();
        let dir = root.join(repo_path_component("dir"));
        assert_eq!(dir.as_ref(), repo_path("dir"));
        let subdir = dir.join(repo_path_component("subdir"));
        assert_eq!(subdir.as_ref(), repo_path("dir/subdir"));
        assert_eq!(
            subdir.join(repo_path_component("file")).as_ref(),
            repo_path("dir/subdir/file")
        );
    }

    #[test]
    fn test_extend() {
        let mut path = RepoPathBuf::root();
        path.extend(std::iter::empty::<RepoPathComponentBuf>());
        assert_eq!(path.as_ref(), RepoPath::root());
        path.extend([repo_path_component("dir")]);
        assert_eq!(path.as_ref(), repo_path("dir"));
        path.extend(std::iter::repeat_n(repo_path_component("subdir"), 3));
        assert_eq!(path.as_ref(), repo_path("dir/subdir/subdir/subdir"));
        path.extend(std::iter::empty::<RepoPathComponentBuf>());
        assert_eq!(path.as_ref(), repo_path("dir/subdir/subdir/subdir"));
    }

    #[test]
    fn test_parent() {
        let root = RepoPath::root();
        let dir_component = repo_path_component("dir");
        let subdir_component = repo_path_component("subdir");

        let dir = root.join(dir_component);
        let subdir = dir.join(subdir_component);

        assert_eq!(root.parent(), None);
        assert_eq!(dir.parent(), Some(root));
        assert_eq!(subdir.parent(), Some(dir.as_ref()));
    }

    #[test]
    fn test_split() {
        let root = RepoPath::root();
        let dir_component = repo_path_component("dir");
        let file_component = repo_path_component("file");

        let dir = root.join(dir_component);
        let file = dir.join(file_component);

        assert_eq!(root.split(), None);
        assert_eq!(dir.split(), Some((root, dir_component)));
        assert_eq!(file.split(), Some((dir.as_ref(), file_component)));
    }

    #[test]
    fn test_components() {
        assert!(RepoPath::root().components().next().is_none());
        assert_eq!(
            repo_path("dir").components().collect_vec(),
            vec![repo_path_component("dir")]
        );
        assert_eq!(
            repo_path("dir/subdir").components().collect_vec(),
            vec![repo_path_component("dir"), repo_path_component("subdir")]
        );

        // Iterates from back
        assert!(RepoPath::root().components().next_back().is_none());
        assert_eq!(
            repo_path("dir").components().rev().collect_vec(),
            vec![repo_path_component("dir")]
        );
        assert_eq!(
            repo_path("dir/subdir").components().rev().collect_vec(),
            vec![repo_path_component("subdir"), repo_path_component("dir")]
        );
    }

    #[test]
    fn test_ancestors() {
        assert_eq!(
            RepoPath::root().ancestors().collect_vec(),
            vec![RepoPath::root()]
        );
        assert_eq!(
            repo_path("dir").ancestors().collect_vec(),
            vec![repo_path("dir"), RepoPath::root()]
        );
        assert_eq!(
            repo_path("dir/subdir").ancestors().collect_vec(),
            vec![repo_path("dir/subdir"), repo_path("dir"), RepoPath::root()]
        );
    }

    #[test]
    fn test_to_fs_path() {
        assert_eq!(
            repo_path("").to_fs_path(Path::new("base/dir")).unwrap(),
            Path::new("base/dir")
        );
        assert_eq!(
            repo_path("").to_fs_path(Path::new("")).unwrap(),
            Path::new(".")
        );
        assert_eq!(
            repo_path("file").to_fs_path(Path::new("base/dir")).unwrap(),
            Path::new("base/dir/file")
        );
        assert_eq!(
            repo_path("some/deep/dir/file")
                .to_fs_path(Path::new("base/dir"))
                .unwrap(),
            Path::new("base/dir/some/deep/dir/file")
        );
        assert_eq!(
            repo_path("dir/file").to_fs_path(Path::new("")).unwrap(),
            Path::new("dir/file")
        );

        // Current/parent dir component
        assert!(repo_path(".").to_fs_path(Path::new("base")).is_err());
        assert!(repo_path("..").to_fs_path(Path::new("base")).is_err());
        assert!(
            repo_path("dir/../file")
                .to_fs_path(Path::new("base"))
                .is_err()
        );
        assert!(repo_path("./file").to_fs_path(Path::new("base")).is_err());
        assert!(repo_path("file/.").to_fs_path(Path::new("base")).is_err());
        assert!(repo_path("../file").to_fs_path(Path::new("base")).is_err());
        assert!(repo_path("file/..").to_fs_path(Path::new("base")).is_err());

        // Empty component (which is invalid as a repo path)
        assert!(
            RepoPath::from_internal_string_unchecked("/")
                .to_fs_path(Path::new("base"))
                .is_err()
        );
        assert_eq!(
            // Iterator omits empty component after "/", which is fine so long
            // as the returned path doesn't escape.
            RepoPath::from_internal_string_unchecked("a/")
                .to_fs_path(Path::new("base"))
                .unwrap(),
            Path::new("base/a")
        );
        assert!(
            RepoPath::from_internal_string_unchecked("/b")
                .to_fs_path(Path::new("base"))
                .is_err()
        );
        assert!(
            RepoPath::from_internal_string_unchecked("a//b")
                .to_fs_path(Path::new("base"))
                .is_err()
        );

        // Component containing slash (simulating Windows path separator)
        assert!(
            RepoPathComponent::new_unchecked("wind/ows")
                .to_fs_name()
                .is_err()
        );
        assert!(
            RepoPathComponent::new_unchecked("./file")
                .to_fs_name()
                .is_err()
        );
        assert!(
            RepoPathComponent::new_unchecked("file/.")
                .to_fs_name()
                .is_err()
        );
        assert!(RepoPathComponent::new_unchecked("/").to_fs_name().is_err());

        // Windows path separator and drive letter
        if cfg!(windows) {
            assert!(
                repo_path(r#"wind\ows"#)
                    .to_fs_path(Path::new("base"))
                    .is_err()
            );
            assert!(
                repo_path(r#".\file"#)
                    .to_fs_path(Path::new("base"))
                    .is_err()
            );
            assert!(
                repo_path(r#"file\."#)
                    .to_fs_path(Path::new("base"))
                    .is_err()
            );
            assert!(
                repo_path(r#"c:/foo"#)
                    .to_fs_path(Path::new("base"))
                    .is_err()
            );
        }
    }

    #[test]
    fn test_to_fs_path_unchecked() {
        assert_eq!(
            repo_path("").to_fs_path_unchecked(Path::new("base/dir")),
            Path::new("base/dir")
        );
        assert_eq!(
            repo_path("").to_fs_path_unchecked(Path::new("")),
            Path::new(".")
        );
        assert_eq!(
            repo_path("file").to_fs_path_unchecked(Path::new("base/dir")),
            Path::new("base/dir/file")
        );
        assert_eq!(
            repo_path("some/deep/dir/file").to_fs_path_unchecked(Path::new("base/dir")),
            Path::new("base/dir/some/deep/dir/file")
        );
        assert_eq!(
            repo_path("dir/file").to_fs_path_unchecked(Path::new("")),
            Path::new("dir/file")
        );
    }

    #[test]
    fn parse_fs_path_wc_in_cwd() {
        let temp_dir = new_temp_dir();
        let cwd_path = temp_dir.path().join("repo");
        let wc_path = &cwd_path;

        assert_eq!(
            RepoPathBuf::parse_fs_path(&cwd_path, wc_path, "").as_deref(),
            Ok(RepoPath::root())
        );
        assert_eq!(
            RepoPathBuf::parse_fs_path(&cwd_path, wc_path, ".").as_deref(),
            Ok(RepoPath::root())
        );
        assert_eq!(
            RepoPathBuf::parse_fs_path(&cwd_path, wc_path, "file").as_deref(),
            Ok(repo_path("file"))
        );
        // Both slash and the platform's separator are allowed
        assert_eq!(
            RepoPathBuf::parse_fs_path(
                &cwd_path,
                wc_path,
                format!("dir{}file", std::path::MAIN_SEPARATOR)
            )
            .as_deref(),
            Ok(repo_path("dir/file"))
        );
        assert_eq!(
            RepoPathBuf::parse_fs_path(&cwd_path, wc_path, "dir/file").as_deref(),
            Ok(repo_path("dir/file"))
        );
        assert_matches!(
            RepoPathBuf::parse_fs_path(&cwd_path, wc_path, ".."),
            Err(FsPathParseError {
                source: RelativePathParseError::InvalidComponent { .. },
                ..
            })
        );
        assert_eq!(
            RepoPathBuf::parse_fs_path(&cwd_path, &cwd_path, "../repo").as_deref(),
            Ok(RepoPath::root())
        );
        assert_eq!(
            RepoPathBuf::parse_fs_path(&cwd_path, &cwd_path, "../repo/file").as_deref(),
            Ok(repo_path("file"))
        );
        // Input may be absolute path with ".."
        assert_eq!(
            RepoPathBuf::parse_fs_path(
                &cwd_path,
                &cwd_path,
                cwd_path.join("../repo").to_str().unwrap()
            )
            .as_deref(),
            Ok(RepoPath::root())
        );
    }

    #[test]
    fn parse_fs_path_wc_in_cwd_parent() {
        let temp_dir = new_temp_dir();
        let cwd_path = temp_dir.path().join("dir");
        let wc_path = cwd_path.parent().unwrap().to_path_buf();

        assert_eq!(
            RepoPathBuf::parse_fs_path(&cwd_path, &wc_path, "").as_deref(),
            Ok(repo_path("dir"))
        );
        assert_eq!(
            RepoPathBuf::parse_fs_path(&cwd_path, &wc_path, ".").as_deref(),
            Ok(repo_path("dir"))
        );
        assert_eq!(
            RepoPathBuf::parse_fs_path(&cwd_path, &wc_path, "file").as_deref(),
            Ok(repo_path("dir/file"))
        );
        assert_eq!(
            RepoPathBuf::parse_fs_path(&cwd_path, &wc_path, "subdir/file").as_deref(),
            Ok(repo_path("dir/subdir/file"))
        );
        assert_eq!(
            RepoPathBuf::parse_fs_path(&cwd_path, &wc_path, "..").as_deref(),
            Ok(RepoPath::root())
        );
        assert_matches!(
            RepoPathBuf::parse_fs_path(&cwd_path, &wc_path, "../.."),
            Err(FsPathParseError {
                source: RelativePathParseError::InvalidComponent { .. },
                ..
            })
        );
        assert_eq!(
            RepoPathBuf::parse_fs_path(&cwd_path, &wc_path, "../other-dir/file").as_deref(),
            Ok(repo_path("other-dir/file"))
        );
    }

    #[test]
    fn parse_fs_path_wc_in_cwd_child() {
        let temp_dir = new_temp_dir();
        let cwd_path = temp_dir.path().join("cwd");
        let wc_path = cwd_path.join("repo");

        assert_matches!(
            RepoPathBuf::parse_fs_path(&cwd_path, &wc_path, ""),
            Err(FsPathParseError {
                source: RelativePathParseError::InvalidComponent { .. },
                ..
            })
        );
        assert_matches!(
            RepoPathBuf::parse_fs_path(&cwd_path, &wc_path, "not-repo"),
            Err(FsPathParseError {
                source: RelativePathParseError::InvalidComponent { .. },
                ..
            })
        );
        assert_eq!(
            RepoPathBuf::parse_fs_path(&cwd_path, &wc_path, "repo").as_deref(),
            Ok(RepoPath::root())
        );
        assert_eq!(
            RepoPathBuf::parse_fs_path(&cwd_path, &wc_path, "repo/file").as_deref(),
            Ok(repo_path("file"))
        );
        assert_eq!(
            RepoPathBuf::parse_fs_path(&cwd_path, &wc_path, "repo/dir/file").as_deref(),
            Ok(repo_path("dir/file"))
        );
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
