// Copyright 2023 The Jujutsu Authors
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

//! Generic algorithms for working with merged values, plus specializations for
//! some common types of merged values.

use std::borrow::Borrow;
use std::collections::HashMap;
use std::fmt::Debug;
use std::fmt::Formatter;
use std::fmt::Write as _;
use std::hash::Hash;
use std::iter::zip;
use std::slice;
use std::sync::Arc;

use futures::future::try_join_all;
use itertools::Itertools as _;
use smallvec::SmallVec;
use smallvec::smallvec_inline;

use crate::backend::BackendResult;
use crate::backend::CopyId;
use crate::backend::FileId;
use crate::backend::TreeValue;
use crate::content_hash::ContentHash;
use crate::content_hash::DigestUpdate;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathComponent;
use crate::store::Store;
use crate::tree::Tree;

/// A generic diff/transition from one value to another.
///
/// This is not a diff in the `patch(1)` sense. See `diff::ContentDiff` for
/// that.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Diff<T> {
    /// The state before
    pub before: T,
    /// The state after
    pub after: T,
}

impl<T> Diff<T> {
    /// Create a new diff
    pub fn new(before: T, after: T) -> Self {
        Self { before, after }
    }

    /// Apply a function to both values
    pub fn map<U>(self, mut f: impl FnMut(T) -> U) -> Diff<U> {
        Diff {
            before: f(self.before),
            after: f(self.after),
        }
    }

    /// Convert a `&Diff<T>` into a `Diff<&T>`.
    pub fn as_ref(&self) -> Diff<&T> {
        Diff {
            before: &self.before,
            after: &self.after,
        }
    }

    /// Convert a diff into an array `[before, after]`.
    pub fn into_array(self) -> [T; 2] {
        [self.before, self.after]
    }
}

impl<T: Eq> Diff<T> {
    /// Whether the diff represents a change, i.e. if `before` and `after` are
    /// not equal
    pub fn is_changed(&self) -> bool {
        self.before != self.after
    }
}

/// Whether to resolve conflict that makes the same change at all sides.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SameChange {
    /// Leaves same-change conflict unresolved.
    Keep,
    /// Resolves same-change conflict as if one side were unchanged.
    /// (i.e. `A+(A-B)=A`)
    ///
    /// This matches what Git and Mercurial do (in the 3-way case at least), but
    /// not what Darcs does. It means that repeated 3-way merging of multiple
    /// trees may give different results depending on the order of merging.
    Accept,
}

/// Attempt to resolve trivial conflicts between the inputs. There must be
/// an odd number of terms.
pub fn trivial_merge<T>(values: &[T], same_change: SameChange) -> Option<&T>
where
    T: Eq + Hash,
{
    assert!(
        values.len() % 2 == 1,
        "trivial_merge() requires an odd number of terms"
    );
    // Optimize the common cases of 3-way merge and 1-way (non-)merge
    if let [add] = values {
        return Some(add);
    } else if let [add0, remove, add1] = values {
        return if add0 == add1 && same_change == SameChange::Accept {
            Some(add0)
        } else if add0 == remove {
            Some(add1)
        } else if add1 == remove {
            Some(add0)
        } else {
            None
        };
    }

    // Number of occurrences of each value, with positive indexes counted as +1 and
    // negative as -1, thereby letting positive and negative terms with the same
    // value (i.e. key in the map) cancel each other.
    let mut counts: HashMap<&T, i32> = HashMap::new();
    for (value, n) in zip(values, [1, -1].into_iter().cycle()) {
        counts.entry(value).and_modify(|e| *e += n).or_insert(n);
    }

    // Collect non-zero value. Values with a count of 0 means that they have
    // canceled out.
    counts.retain(|_, count| *count != 0);
    if counts.len() == 1 {
        // If there is a single value with a count of 1 left, then that is the result.
        let (value, count) = counts.into_iter().next().unwrap();
        assert_eq!(count, 1);
        Some(value)
    } else if counts.len() == 2 && same_change == SameChange::Accept {
        // All sides made the same change.
        let [(value1, count1), (value2, count2)] = counts.into_iter().next_array().unwrap();
        assert_eq!(count1 + count2, 1);
        if count1 > 0 {
            Some(value1)
        } else {
            Some(value2)
        }
    } else {
        None
    }
}

/// A generic representation of merged values.
///
/// There is exactly one more `adds()` than `removes()`. When interpreted as a
/// series of diffs, the merge's (i+1)-st add is matched with the i-th
/// remove. The zeroth add is considered a diff from the non-existent state.
#[derive(PartialEq, Eq, Hash, Clone, serde::Serialize)]
#[serde(transparent)]
pub struct Merge<T> {
    /// Alternates between positive and negative terms, starting with positive.
    values: SmallVec<[T; 1]>,
}

impl<T: Debug> Debug for Merge<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        // Format like an enum with two variants to make it less verbose in the common
        // case of a resolved state.
        if let Some(value) = self.as_resolved() {
            f.debug_tuple("Resolved").field(value).finish()
        } else {
            f.debug_tuple("Conflicted").field(&self.values).finish()
        }
    }
}

impl<T> Merge<T> {
    /// Creates a `Merge` from the given values, in which positive and negative
    /// terms alternate.
    pub fn from_vec(values: impl Into<SmallVec<[T; 1]>>) -> Self {
        let values = values.into();
        assert!(values.len() % 2 != 0, "must have an odd number of terms");
        Self { values }
    }

    /// Creates a new merge object from the given removes and adds.
    pub fn from_removes_adds(
        removes: impl IntoIterator<Item = T>,
        adds: impl IntoIterator<Item = T>,
    ) -> Self {
        let removes = removes.into_iter();
        let mut adds = adds.into_iter();
        let mut values = SmallVec::with_capacity(removes.size_hint().0 * 2 + 1);
        values.push(adds.next().expect("must have at least one add"));
        for diff in removes.zip_longest(adds) {
            let (remove, add) = diff.both().expect("must have one more adds than removes");
            values.extend([remove, add]);
        }
        Self { values }
    }

    /// Creates a `Merge` with a single resolved value.
    pub const fn resolved(value: T) -> Self {
        Self {
            values: smallvec_inline![value],
        }
    }

    /// Create a `Merge` from a `removes` and `adds`, padding with `None` to
    /// make sure that there is exactly one more `adds` than `removes`.
    pub fn from_legacy_form(
        removes: impl IntoIterator<Item = T>,
        adds: impl IntoIterator<Item = T>,
    ) -> Merge<Option<T>> {
        let removes = removes.into_iter();
        let mut adds = adds.into_iter().fuse();
        let mut values = smallvec_inline![adds.next()];
        for diff in removes.zip_longest(adds) {
            let (remove, add) = diff.map_any(Some, Some).or_default();
            values.extend([remove, add]);
        }
        Merge { values }
    }

    /// The removed values, also called negative terms.
    pub fn removes(&self) -> impl ExactSizeIterator<Item = &T> {
        self.values[1..].iter().step_by(2)
    }

    /// The added values, also called positive terms.
    pub fn adds(&self) -> impl ExactSizeIterator<Item = &T> {
        self.values.iter().step_by(2)
    }

    /// Returns the zeroth added value, which is guaranteed to exist.
    pub fn first(&self) -> &T {
        &self.values[0]
    }

    /// Returns the `index`-th removed value, which is considered belonging to
    /// the `index`-th diff pair.
    pub fn get_remove(&self, index: usize) -> Option<&T> {
        self.values.get(index * 2 + 1)
    }

    /// Returns the `index`-th added value, which is considered belonging to the
    /// `index-1`-th diff pair. The zeroth add is a diff from the non-existent
    /// state.
    pub fn get_add(&self, index: usize) -> Option<&T> {
        self.values.get(index * 2)
    }

    /// Removes the specified "removed"/"added" values. The removed slots are
    /// replaced by the last "removed"/"added" values.
    pub fn swap_remove(&mut self, remove_index: usize, add_index: usize) -> (T, T) {
        // Swap with the last "added" and "removed" values in order.
        let add = self.values.swap_remove(add_index * 2);
        let remove = self.values.swap_remove(remove_index * 2 + 1);
        (remove, add)
    }

    /// The number of positive terms in the conflict.
    pub fn num_sides(&self) -> usize {
        self.values.len() / 2 + 1
    }

    /// Whether this merge is resolved. Does not resolve trivial merges.
    pub fn is_resolved(&self) -> bool {
        self.values.len() == 1
    }

    /// Returns the resolved value, if this merge is resolved. Does not
    /// resolve trivial merges.
    pub fn as_resolved(&self) -> Option<&T> {
        if let [value] = &self.values[..] {
            Some(value)
        } else {
            None
        }
    }

    /// Returns the resolved value, if this merge is resolved. Otherwise returns
    /// the merge itself as an `Err`. Does not resolve trivial merges.
    pub fn into_resolved(mut self) -> Result<T, Self> {
        if self.values.len() == 1 {
            Ok(self.values.pop().unwrap())
        } else {
            Err(self)
        }
    }

    /// Returns a vector mapping of a value's index in the simplified merge to
    /// its original index in the unsimplified merge.
    ///
    /// The merge is simplified by removing identical values in add and remove
    /// values.
    fn get_simplified_mapping(&self) -> Vec<usize>
    where
        T: PartialEq,
    {
        let unsimplified_len = self.values.len();
        let mut simplified_to_original_indices = (0..unsimplified_len).collect_vec();

        let mut add_index = 0;
        while add_index < simplified_to_original_indices.len() {
            let add = &self.values[simplified_to_original_indices[add_index]];
            let mut remove_indices = simplified_to_original_indices
                .iter()
                .enumerate()
                .skip(1)
                .step_by(2);
            if let Some((remove_index, _)) = remove_indices
                .find(|&(_, original_remove_index)| &self.values[*original_remove_index] == add)
            {
                // Align the current "add" value to the `remove_index/2`-th diff, then
                // delete the diff pair.
                simplified_to_original_indices.swap(remove_index + 1, add_index);
                simplified_to_original_indices.drain(remove_index..remove_index + 2);
            } else {
                add_index += 2;
            }
        }

        simplified_to_original_indices
    }

    /// Simplify the merge by joining diffs like A->B and B->C into A->C.
    /// Also drops trivial diffs like A->A.
    #[must_use]
    pub fn simplify(&self) -> Self
    where
        T: PartialEq + Clone,
    {
        let mapping = self.get_simplified_mapping();
        // Reorder values based on their new indices in the simplified merge.
        let values = mapping
            .iter()
            .map(|index| self.values[*index].clone())
            .collect();
        Self { values }
    }

    /// Updates the merge based on the given simplified merge.
    pub fn update_from_simplified(mut self, simplified: Self) -> Self
    where
        T: PartialEq,
    {
        let mapping = self.get_simplified_mapping();
        assert_eq!(mapping.len(), simplified.values.len());
        for (index, value) in mapping.into_iter().zip(simplified.values.into_iter()) {
            self.values[index] = value;
        }
        self
    }

    /// If this merge can be trivially resolved, returns the value it resolves
    /// to.
    pub fn resolve_trivial(&self, same_change: SameChange) -> Option<&T>
    where
        T: Eq + Hash,
    {
        trivial_merge(&self.values, same_change)
    }

    /// Pads this merge with to the specified number of sides with the specified
    /// value. No-op if the requested size is not larger than the current size.
    pub fn pad_to(&mut self, num_sides: usize, value: &T)
    where
        T: Clone,
    {
        if num_sides <= self.num_sides() {
            return;
        }
        self.values.resize(num_sides * 2 - 1, value.clone());
    }

    /// Returns a slice containing the terms. The items will alternate between
    /// positive and negative terms, starting with positive (since there's one
    /// more of those).
    pub fn as_slice(&self) -> &[T] {
        &self.values
    }

    /// Returns an iterator over references to the terms. The items will
    /// alternate between positive and negative terms, starting with
    /// positive (since there's one more of those).
    pub fn iter(&self) -> slice::Iter<'_, T> {
        self.values.iter()
    }

    /// A version of `Merge::iter()` that iterates over mutable references.
    pub fn iter_mut(&mut self) -> slice::IterMut<'_, T> {
        self.values.iter_mut()
    }

    /// Creates a new merge by applying `f` to each remove and add.
    pub fn map<'a, U>(&'a self, f: impl FnMut(&'a T) -> U) -> Merge<U> {
        let values = self.values.iter().map(f).collect();
        Merge { values }
    }

    /// Creates a new merge by applying `f` to each remove and add, returning
    /// `Err` if `f` returns `Err` for any of them.
    pub fn try_map<'a, U, E>(
        &'a self,
        f: impl FnMut(&'a T) -> Result<U, E>,
    ) -> Result<Merge<U>, E> {
        let values = self.values.iter().map(f).try_collect()?;
        Ok(Merge { values })
    }

    /// Creates a new merge by applying the async function `f` to each remove
    /// and add, running them concurrently, and returning `Err` if `f`
    /// returns `Err` for any of them.
    pub async fn try_map_async<'a, F, U, E>(
        &'a self,
        f: impl FnMut(&'a T) -> F,
    ) -> Result<Merge<U>, E>
    where
        F: Future<Output = Result<U, E>>,
    {
        let values = try_join_all(self.values.iter().map(f)).await?;
        Ok(Merge {
            values: values.into(),
        })
    }
}

/// Helper for consuming items from an iterator and then creating a `Merge`.
///
/// By not collecting directly into `Merge`, we can avoid creating invalid
/// instances of it. If we had `Merge::from_iter()` we would need to allow it to
/// accept iterators of any length (including 0). We couldn't make it panic on
/// even lengths because we can get passed such iterators from e.g.
/// `Option::from_iter()`. By collecting into `MergeBuilder` instead, we move
/// the checking until after `from_iter()` (to `MergeBuilder::build()`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeBuilder<T> {
    values: SmallVec<[T; 1]>,
}

impl<T> Default for MergeBuilder<T> {
    fn default() -> Self {
        Self {
            values: Default::default(),
        }
    }
}

impl<T> MergeBuilder<T> {
    /// Requires that exactly one more "adds" than "removes" have been added to
    /// this builder.
    pub fn build(self) -> Merge<T> {
        Merge::from_vec(self.values)
    }
}

impl<T> IntoIterator for Merge<T> {
    type Item = T;
    type IntoIter = smallvec::IntoIter<[T; 1]>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.into_iter()
    }
}

impl<T> FromIterator<T> for MergeBuilder<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut builder = Self::default();
        builder.extend(iter);
        builder
    }
}

impl<T> Extend<T> for MergeBuilder<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        self.values.extend(iter);
    }
}

impl<T> Merge<Option<T>> {
    /// Creates a resolved merge with a value of `None`.
    pub fn absent() -> Self {
        Self::resolved(None)
    }

    /// Creates a resolved merge with a value of `Some(value)`.
    pub fn normal(value: T) -> Self {
        Self::resolved(Some(value))
    }

    /// Whether this represents a resolved value of `None`.
    pub fn is_absent(&self) -> bool {
        matches!(self.as_resolved(), Some(None))
    }

    /// The opposite of `is_absent()`.
    pub fn is_present(&self) -> bool {
        !self.is_absent()
    }

    /// Returns the value if this is present and non-conflicting.
    pub fn as_normal(&self) -> Option<&T> {
        self.as_resolved()?.as_ref()
    }

    /// Creates lists of `removes` and `adds` from a `Merge` by dropping
    /// `None` values. Note that the conversion is lossy: the order of `None`
    /// values is not preserved when converting back to a `Merge`.
    pub fn into_legacy_form(self) -> (Vec<T>, Vec<T>) {
        // Allocate the maximum size assuming there would be few `None`s.
        let mut removes = Vec::with_capacity(self.values.len() / 2);
        let mut adds = Vec::with_capacity(self.values.len() / 2 + 1);
        let mut values = self.values.into_iter();
        adds.extend(values.next().unwrap());
        while let Some(remove) = values.next() {
            removes.extend(remove);
            adds.extend(values.next().unwrap());
        }
        (removes, adds)
    }
}

impl<T: Clone> Merge<Option<&T>> {
    /// Creates a new merge by cloning inner `Option<&T>`s.
    pub fn cloned(&self) -> Merge<Option<T>> {
        self.map(|value| value.cloned())
    }
}

impl<T> Merge<Merge<T>> {
    /// Flattens a nested merge into a regular merge.
    ///
    /// Let's say we have a 3-way merge of 3-way merges like this:
    ///
    /// ```text
    /// 4 5   7 8
    ///  3     6
    ///    1 2
    ///     0
    /// ```
    ///
    /// Flattening that results in this 9-way merge:
    ///
    /// ```text
    /// 4 5 0 7 8
    ///  3 2 1 6
    /// ```
    pub fn flatten(self) -> Merge<T> {
        let mut outer_values = self.values.into_iter();
        let mut result = outer_values.next().unwrap();
        while let Some(mut remove) = outer_values.next() {
            // Add removes reversed, and with the first element moved last, so we preserve
            // the diffs
            remove.values.rotate_left(1);
            for i in 0..remove.values.len() / 2 {
                remove.values.swap(i * 2, i * 2 + 1);
            }
            result.values.extend(remove.values);
            let add = outer_values.next().unwrap();
            result.values.extend(add.values);
        }
        result
    }
}

impl<T: ContentHash> ContentHash for Merge<T> {
    fn hash(&self, state: &mut impl DigestUpdate) {
        self.values.hash(state);
    }
}

/// Borrowed `MergedTreeValue`.
pub type MergedTreeVal<'a> = Merge<Option<&'a TreeValue>>;

/// The value at a given path in a commit.
///
/// It depends on the context whether it can be absent
/// (`Merge::is_absent()`). For example, when getting the value at a
/// specific path, it may be, but when iterating over entries in a
/// tree, it shouldn't be.
pub type MergedTreeValue = Merge<Option<TreeValue>>;

impl<T> Merge<Option<T>>
where
    T: Borrow<TreeValue>,
{
    /// Whether this merge should be recursed into when doing directory walks.
    pub fn is_tree(&self) -> bool {
        self.is_present()
            && self.iter().all(|value| {
                matches!(
                    borrow_tree_value(value.as_ref()),
                    Some(TreeValue::Tree(_)) | None
                )
            })
    }

    /// Whether this merge is present and not a tree
    pub fn is_file_like(&self) -> bool {
        self.is_present() && !self.is_tree()
    }

    /// If this merge contains only files or absent entries, returns a merge of
    /// the `FileId`s. The executable bits and copy IDs will be ignored. Use
    /// `Merge::with_new_file_ids()` to produce a new merge with the original
    /// executable bits preserved.
    pub fn to_file_merge(&self) -> Option<Merge<Option<FileId>>> {
        let file_ids = self
            .try_map(|term| match borrow_tree_value(term.as_ref()) {
                None => Ok(None),
                Some(TreeValue::File {
                    id,
                    executable: _,
                    copy_id: _,
                }) => Ok(Some(id.clone())),
                _ => Err(()),
            })
            .ok()?;

        Some(file_ids)
    }

    /// If this merge contains only files or absent entries, returns a merge of
    /// the files' executable bits.
    pub fn to_executable_merge(&self) -> Option<Merge<Option<bool>>> {
        self.try_map(|term| match borrow_tree_value(term.as_ref()) {
            None => Ok(None),
            Some(TreeValue::File {
                id: _,
                executable,
                copy_id: _,
            }) => Ok(Some(*executable)),
            _ => Err(()),
        })
        .ok()
    }

    /// If this merge contains only files or absent entries, returns a merge of
    /// the files' copy IDs.
    pub fn to_copy_id_merge(&self) -> Option<Merge<Option<CopyId>>> {
        self.try_map(|term| match borrow_tree_value(term.as_ref()) {
            None => Ok(None),
            Some(TreeValue::File {
                id: _,
                executable: _,
                copy_id,
            }) => Ok(Some(copy_id.clone())),
            _ => Err(()),
        })
        .ok()
    }

    /// If every non-`None` term of a `MergedTreeValue`
    /// is a `TreeValue::Tree`, this converts it to
    /// a `Merge<Tree>`, with empty trees instead of
    /// any `None` terms. Otherwise, returns `None`.
    pub async fn to_tree_merge(
        &self,
        store: &Arc<Store>,
        dir: &RepoPath,
    ) -> BackendResult<Option<Merge<Tree>>> {
        let tree_id_merge = self.try_map(|term| match borrow_tree_value(term.as_ref()) {
            None => Ok(None),
            Some(TreeValue::Tree(id)) => Ok(Some(id)),
            Some(_) => Err(()),
        });
        if let Ok(tree_id_merge) = tree_id_merge {
            Ok(Some(
                tree_id_merge
                    .try_map_async(async |id| {
                        if let Some(id) = id {
                            store.get_tree_async(dir.to_owned(), id).await
                        } else {
                            Ok(Tree::empty(store.clone(), dir.to_owned()))
                        }
                    })
                    .await?,
            ))
        } else {
            Ok(None)
        }
    }

    /// Creates a new merge with the file ids from the given merge. In other
    /// words, only the executable bits from `self` will be preserved.
    ///
    /// The given `file_ids` should have the same shape as `self`. Only the
    /// `FileId` values may differ.
    pub fn with_new_file_ids(&self, file_ids: &Merge<Option<FileId>>) -> Merge<Option<TreeValue>> {
        assert_eq!(self.values.len(), file_ids.values.len());
        let values = zip(self.iter(), file_ids.iter().cloned())
            .map(
                |(tree_value, file_id)| match (borrow_tree_value(tree_value.as_ref()), file_id) {
                    (
                        Some(TreeValue::File {
                            id: _,
                            executable,
                            copy_id,
                        }),
                        Some(id),
                    ) => Some(TreeValue::File {
                        id,
                        executable: *executable,
                        copy_id: copy_id.clone(),
                    }),
                    (None, None) => None,
                    (old, new) => panic!("incompatible update: {old:?} to {new:?}"),
                },
            )
            .collect();
        Merge { values }
    }

    /// Give a summary description of the conflict's "removes" and "adds"
    pub fn describe(&self) -> String {
        let mut buf = String::new();
        writeln!(buf, "Conflict:").unwrap();
        for term in self.removes().flatten() {
            writeln!(buf, "  Removing {}", describe_conflict_term(term.borrow())).unwrap();
        }
        for term in self.adds().flatten() {
            writeln!(buf, "  Adding {}", describe_conflict_term(term.borrow())).unwrap();
        }
        buf
    }
}

fn borrow_tree_value<T: Borrow<TreeValue> + ?Sized>(term: Option<&T>) -> Option<&TreeValue> {
    term.map(|value| value.borrow())
}

fn describe_conflict_term(value: &TreeValue) -> String {
    match value {
        TreeValue::File {
            id,
            executable: false,
            copy_id: _,
        } => {
            // TODO: include the copy here once we start using it
            format!("file with id {id}")
        }
        TreeValue::File {
            id,
            executable: true,
            copy_id: _,
        } => {
            // TODO: include the copy here once we start using it
            format!("executable file with id {id}")
        }
        TreeValue::Symlink(id) => {
            format!("symlink with id {id}")
        }
        TreeValue::Tree(id) => {
            format!("tree with id {id}")
        }
        TreeValue::GitSubmodule(id) => {
            format!("Git submodule with id {id}")
        }
    }
}

impl Merge<Tree> {
    /// The directory that is shared by all trees in the merge.
    pub fn dir(&self) -> &RepoPath {
        debug_assert!(self.iter().map(|tree| tree.dir()).all_equal());
        self.first().dir()
    }

    /// The value at the given basename. The value can be `Resolved` even if
    /// `self` is conflicted, which happens if the value at the path can be
    /// trivially merged. Does not recurse, so if `basename` refers to a Tree,
    /// then a `TreeValue::Tree` will be returned.
    pub fn value(&self, basename: &RepoPathComponent) -> MergedTreeVal<'_> {
        if let Some(tree) = self.as_resolved() {
            return Merge::resolved(tree.value(basename));
        }
        let same_change = self.first().store().merge_options().same_change;
        let value = self.map(|tree| tree.value(basename));
        if let Some(resolved) = value.resolve_trivial(same_change) {
            return Merge::resolved(*resolved);
        }
        value
    }

    /// Gets the `Merge<Tree>` in a subdirectory of the current tree. If the
    /// path doesn't correspond to a tree in any of the inputs to the merge,
    /// then that entry will be replaced by an empty tree in the result.
    pub async fn sub_tree(&self, name: &RepoPathComponent) -> BackendResult<Option<Self>> {
        let store = self.first().store();
        match self.value(name).into_resolved() {
            Ok(Some(TreeValue::Tree(sub_tree_id))) => {
                let subdir = self.dir().join(name);
                Ok(Some(Self::resolved(
                    store.get_tree_async(subdir, sub_tree_id).await?,
                )))
            }
            Ok(_) => Ok(None),
            Err(merge) => {
                if !merge.is_tree() {
                    return Ok(None);
                }
                let trees = merge
                    .try_map_async(async |value| match value {
                        Some(TreeValue::Tree(sub_tree_id)) => {
                            let subdir = self.dir().join(name);
                            store.get_tree_async(subdir, sub_tree_id).await
                        }
                        Some(_) => unreachable!(),
                        None => {
                            let subdir = self.dir().join(name);
                            Ok(Tree::empty(store.clone(), subdir))
                        }
                    })
                    .await?;
                Ok(Some(trees))
            }
        }
    }

    /// Look up the tree at the given path.
    pub async fn sub_tree_recursive(&self, path: &RepoPath) -> BackendResult<Option<Self>> {
        let mut current_tree = self.clone();
        for name in path.components() {
            match current_tree.sub_tree(name).await? {
                None => {
                    return Ok(None);
                }
                Some(sub_tree) => {
                    current_tree = sub_tree;
                }
            }
        }
        Ok(Some(current_tree))
    }
}

#[cfg(test)]
mod tests {
    use test_case::test_case;

    use super::*;

    fn c<T: Clone>(terms: &[T]) -> Merge<T> {
        Merge::from_vec(terms.to_vec())
    }

    #[test_case(SameChange::Keep)]
    #[test_case(SameChange::Accept)]
    fn test_trivial_merge(same_change: SameChange) {
        let accept_same_change = same_change == SameChange::Accept;
        let merge = |values| trivial_merge(values, same_change);
        assert_eq!(merge(&[0]), Some(&0));
        assert_eq!(merge(&[0, 0, 0]), Some(&0));
        assert_eq!(merge(&[0, 0, 1]), Some(&1));
        assert_eq!(merge(&[0, 1, 0]), accept_same_change.then_some(&0));
        assert_eq!(merge(&[0, 1, 1]), Some(&0));
        assert_eq!(merge(&[0, 1, 2]), None);
        assert_eq!(merge(&[0, 0, 0, 0, 0]), Some(&0));
        assert_eq!(merge(&[0, 0, 0, 0, 1]), Some(&1));
        assert_eq!(merge(&[0, 0, 0, 1, 0]), accept_same_change.then_some(&0));
        assert_eq!(merge(&[0, 0, 0, 1, 1]), Some(&0));
        assert_eq!(merge(&[0, 0, 0, 1, 2]), None);
        assert_eq!(merge(&[0, 0, 1, 0, 0]), Some(&1));
        assert_eq!(merge(&[0, 0, 1, 0, 1]), accept_same_change.then_some(&1));
        assert_eq!(merge(&[0, 0, 1, 0, 2]), None);
        assert_eq!(merge(&[0, 0, 1, 1, 0]), Some(&0));
        assert_eq!(merge(&[0, 0, 1, 1, 1]), Some(&1));
        assert_eq!(merge(&[0, 0, 1, 1, 2]), Some(&2));
        assert_eq!(merge(&[0, 0, 1, 2, 0]), None);
        assert_eq!(merge(&[0, 0, 1, 2, 1]), accept_same_change.then_some(&1));
        assert_eq!(merge(&[0, 0, 1, 2, 2]), Some(&1));
        assert_eq!(merge(&[0, 0, 1, 2, 3]), None);
        assert_eq!(merge(&[0, 1, 0, 0, 0]), accept_same_change.then_some(&0));
        assert_eq!(merge(&[0, 1, 0, 0, 1]), Some(&0));
        assert_eq!(merge(&[0, 1, 0, 0, 2]), None);
        assert_eq!(merge(&[0, 1, 0, 1, 0]), accept_same_change.then_some(&0));
        assert_eq!(merge(&[0, 1, 0, 1, 1]), accept_same_change.then_some(&0));
        assert_eq!(merge(&[0, 1, 0, 1, 2]), None);
        assert_eq!(merge(&[0, 1, 0, 2, 0]), None);
        assert_eq!(merge(&[0, 1, 0, 2, 1]), accept_same_change.then_some(&0));
        assert_eq!(merge(&[0, 1, 0, 2, 2]), accept_same_change.then_some(&0));
        assert_eq!(merge(&[0, 1, 0, 2, 3]), None);
        assert_eq!(merge(&[0, 1, 1, 0, 0]), Some(&0));
        assert_eq!(merge(&[0, 1, 1, 0, 1]), Some(&1));
        assert_eq!(merge(&[0, 1, 1, 0, 2]), Some(&2));
        assert_eq!(merge(&[0, 1, 1, 1, 0]), accept_same_change.then_some(&0));
        assert_eq!(merge(&[0, 1, 1, 1, 1]), Some(&0));
        assert_eq!(merge(&[0, 1, 1, 1, 2]), None);
        assert_eq!(merge(&[0, 1, 1, 2, 0]), accept_same_change.then_some(&0));
        assert_eq!(merge(&[0, 1, 1, 2, 1]), None);
        assert_eq!(merge(&[0, 1, 1, 2, 2]), Some(&0));
        assert_eq!(merge(&[0, 1, 1, 2, 3]), None);
        assert_eq!(merge(&[0, 1, 2, 0, 0]), None);
        assert_eq!(merge(&[0, 1, 2, 0, 1]), Some(&2));
        assert_eq!(merge(&[0, 1, 2, 0, 2]), accept_same_change.then_some(&2));
        assert_eq!(merge(&[0, 1, 2, 0, 3]), None);
        assert_eq!(merge(&[0, 1, 2, 1, 0]), None);
        assert_eq!(merge(&[0, 1, 2, 1, 1]), None);
        assert_eq!(merge(&[0, 1, 2, 1, 2]), None);
        assert_eq!(merge(&[0, 1, 2, 1, 3]), None);
        assert_eq!(merge(&[0, 1, 2, 2, 0]), accept_same_change.then_some(&0));
        assert_eq!(merge(&[0, 1, 2, 2, 1]), Some(&0));
        assert_eq!(merge(&[0, 1, 2, 2, 2]), None);
        assert_eq!(merge(&[0, 1, 2, 2, 3]), None);
        assert_eq!(merge(&[0, 1, 2, 3, 0]), None);
        assert_eq!(merge(&[0, 1, 2, 3, 1]), None);
        assert_eq!(merge(&[0, 1, 2, 3, 2]), None);
        assert_eq!(merge(&[0, 1, 2, 3, 3]), None);
        assert_eq!(merge(&[0, 1, 2, 3, 4]), None);
    }

    #[test]
    fn test_legacy_form_conversion() {
        fn test_equivalent<T>(legacy_form: (Vec<T>, Vec<T>), merge: Merge<Option<T>>)
        where
            T: Clone + PartialEq + std::fmt::Debug,
        {
            assert_eq!(merge.clone().into_legacy_form(), legacy_form);
            assert_eq!(Merge::from_legacy_form(legacy_form.0, legacy_form.1), merge);
        }
        // Non-conflict
        test_equivalent(
            (vec![], vec![0]),
            Merge::from_removes_adds(vec![], vec![Some(0)]),
        );
        // Regular 3-way conflict
        test_equivalent(
            (vec![0], vec![1, 2]),
            Merge::from_removes_adds(vec![Some(0)], vec![Some(1), Some(2)]),
        );
        // Modify/delete conflict
        test_equivalent(
            (vec![0], vec![1]),
            Merge::from_removes_adds(vec![Some(0)], vec![Some(1), None]),
        );
        // Add/add conflict
        test_equivalent(
            (vec![], vec![0, 1]),
            Merge::from_removes_adds(vec![None], vec![Some(0), Some(1)]),
        );
        // 5-way conflict
        test_equivalent(
            (vec![0, 1], vec![2, 3, 4]),
            Merge::from_removes_adds(vec![Some(0), Some(1)], vec![Some(2), Some(3), Some(4)]),
        );
        // 5-way delete/delete conflict
        test_equivalent(
            (vec![0, 1], vec![]),
            Merge::from_removes_adds(vec![Some(0), Some(1)], vec![None, None, None]),
        );
    }

    #[test]
    fn test_as_resolved() {
        assert_eq!(
            Merge::from_removes_adds(vec![], vec![0]).as_resolved(),
            Some(&0)
        );
        // Even a trivially resolvable merge is not resolved
        assert_eq!(
            Merge::from_removes_adds(vec![0], vec![0, 1]).as_resolved(),
            None
        );
    }

    #[test]
    fn test_get_simplified_mapping() {
        // 1-way merge
        assert_eq!(c(&[0]).get_simplified_mapping(), vec![0]);
        // 3-way merge
        assert_eq!(c(&[0, 0, 0]).get_simplified_mapping(), vec![2]);
        assert_eq!(c(&[0, 0, 1]).get_simplified_mapping(), vec![2]);
        assert_eq!(c(&[0, 1, 0]).get_simplified_mapping(), vec![0, 1, 2]);
        assert_eq!(c(&[0, 1, 1]).get_simplified_mapping(), vec![0]);
        assert_eq!(c(&[0, 1, 2]).get_simplified_mapping(), vec![0, 1, 2]);
        // 5-way merge
        assert_eq!(c(&[0, 0, 0, 0, 0]).get_simplified_mapping(), vec![4]);
        assert_eq!(c(&[0, 0, 0, 0, 1]).get_simplified_mapping(), vec![4]);
        assert_eq!(c(&[0, 0, 0, 1, 0]).get_simplified_mapping(), vec![2, 3, 4]);
        assert_eq!(c(&[0, 0, 0, 1, 1]).get_simplified_mapping(), vec![2]);
        assert_eq!(c(&[0, 0, 0, 1, 2]).get_simplified_mapping(), vec![2, 3, 4]);
        assert_eq!(c(&[0, 0, 1, 0, 0]).get_simplified_mapping(), vec![2]);
        assert_eq!(c(&[0, 0, 1, 0, 1]).get_simplified_mapping(), vec![2, 3, 4]);
        assert_eq!(c(&[0, 0, 1, 0, 2]).get_simplified_mapping(), vec![2, 3, 4]);
        assert_eq!(c(&[0, 0, 1, 1, 0]).get_simplified_mapping(), vec![4]);
        assert_eq!(c(&[0, 0, 1, 1, 1]).get_simplified_mapping(), vec![4]);
        assert_eq!(c(&[0, 0, 1, 1, 2]).get_simplified_mapping(), vec![4]);
        assert_eq!(c(&[0, 0, 2, 1, 0]).get_simplified_mapping(), vec![2, 3, 4]);
        assert_eq!(c(&[0, 0, 2, 1, 1]).get_simplified_mapping(), vec![2]);
        assert_eq!(c(&[0, 0, 2, 1, 2]).get_simplified_mapping(), vec![2, 3, 4]);
        assert_eq!(c(&[0, 0, 2, 1, 3]).get_simplified_mapping(), vec![2, 3, 4]);
        assert_eq!(c(&[0, 1, 0, 0, 0]).get_simplified_mapping(), vec![4, 1, 2]);
        assert_eq!(c(&[0, 1, 0, 0, 1]).get_simplified_mapping(), vec![2]);
        assert_eq!(c(&[0, 1, 0, 0, 2]).get_simplified_mapping(), vec![4, 1, 2]);
        assert_eq!(
            c(&[0, 1, 0, 1, 0]).get_simplified_mapping(),
            vec![0, 1, 2, 3, 4]
        );
        assert_eq!(c(&[0, 1, 0, 1, 1]).get_simplified_mapping(), vec![0, 3, 2]);
        assert_eq!(
            c(&[0, 1, 0, 1, 2]).get_simplified_mapping(),
            vec![0, 1, 2, 3, 4]
        );
        assert_eq!(
            c(&[0, 1, 0, 2, 0]).get_simplified_mapping(),
            vec![0, 1, 2, 3, 4]
        );
        assert_eq!(c(&[0, 1, 0, 2, 1]).get_simplified_mapping(), vec![0, 3, 2]);
        assert_eq!(c(&[0, 1, 0, 2, 2]).get_simplified_mapping(), vec![0, 1, 2]);
        assert_eq!(
            c(&[0, 1, 0, 2, 3]).get_simplified_mapping(),
            vec![0, 1, 2, 3, 4]
        );
        assert_eq!(c(&[0, 1, 1, 0, 0]).get_simplified_mapping(), vec![4]);
        assert_eq!(c(&[0, 1, 1, 0, 1]).get_simplified_mapping(), vec![2]);
        assert_eq!(c(&[0, 1, 1, 0, 2]).get_simplified_mapping(), vec![4]);
        assert_eq!(c(&[0, 1, 1, 1, 0]).get_simplified_mapping(), vec![0, 3, 4]);
        assert_eq!(c(&[0, 1, 1, 1, 1]).get_simplified_mapping(), vec![0]);
        assert_eq!(c(&[0, 1, 1, 1, 2]).get_simplified_mapping(), vec![0, 3, 4]);
        assert_eq!(c(&[0, 1, 1, 2, 0]).get_simplified_mapping(), vec![0, 3, 4]);
        assert_eq!(c(&[0, 1, 1, 2, 1]).get_simplified_mapping(), vec![0, 3, 4]);
        assert_eq!(c(&[0, 1, 1, 2, 2]).get_simplified_mapping(), vec![0]);
        assert_eq!(c(&[0, 1, 1, 2, 3]).get_simplified_mapping(), vec![0, 3, 4]);
        assert_eq!(c(&[0, 1, 2, 0, 0]).get_simplified_mapping(), vec![4, 1, 2]);
        assert_eq!(c(&[0, 1, 2, 0, 1]).get_simplified_mapping(), vec![2]);
        assert_eq!(c(&[0, 1, 2, 0, 2]).get_simplified_mapping(), vec![4, 1, 2]);
        assert_eq!(c(&[0, 1, 2, 0, 3]).get_simplified_mapping(), vec![4, 1, 2]);
        assert_eq!(
            c(&[0, 1, 2, 1, 0]).get_simplified_mapping(),
            vec![0, 1, 2, 3, 4]
        );
        assert_eq!(c(&[0, 1, 2, 1, 1]).get_simplified_mapping(), vec![0, 3, 2]);
        assert_eq!(
            c(&[0, 1, 2, 1, 2]).get_simplified_mapping(),
            vec![0, 1, 2, 3, 4]
        );
        assert_eq!(
            c(&[0, 1, 2, 1, 3]).get_simplified_mapping(),
            vec![0, 1, 2, 3, 4]
        );
        assert_eq!(c(&[0, 1, 2, 2, 0]).get_simplified_mapping(), vec![0, 1, 4]);
        assert_eq!(c(&[0, 1, 2, 2, 1]).get_simplified_mapping(), vec![0]);
        assert_eq!(c(&[0, 1, 2, 2, 2]).get_simplified_mapping(), vec![0, 1, 4]);
        assert_eq!(c(&[0, 1, 2, 2, 3]).get_simplified_mapping(), vec![0, 1, 4]);
        assert_eq!(
            c(&[0, 1, 2, 3, 0]).get_simplified_mapping(),
            vec![0, 1, 2, 3, 4]
        );
        assert_eq!(c(&[0, 1, 2, 3, 1]).get_simplified_mapping(), vec![0, 3, 2]);
        assert_eq!(
            c(&[0, 1, 2, 3, 2]).get_simplified_mapping(),
            vec![0, 1, 2, 3, 4]
        );
        assert_eq!(
            c(&[0, 1, 2, 3, 4, 5, 1]).get_simplified_mapping(),
            vec![0, 3, 4, 5, 2]
        );
        assert_eq!(
            c(&[0, 1, 2, 3, 4]).get_simplified_mapping(),
            vec![0, 1, 2, 3, 4]
        );
        assert_eq!(c(&[2, 0, 3, 1, 1]).get_simplified_mapping(), vec![0, 1, 2]);
    }

    #[test]
    fn test_simplify() {
        // 1-way merge
        assert_eq!(c(&[0]).simplify(), c(&[0]));
        // 3-way merge
        assert_eq!(c(&[0, 0, 0]).simplify(), c(&[0]));
        assert_eq!(c(&[0, 0, 1]).simplify(), c(&[1]));
        assert_eq!(c(&[1, 0, 0]).simplify(), c(&[1]));
        assert_eq!(c(&[1, 0, 1]).simplify(), c(&[1, 0, 1]));
        assert_eq!(c(&[1, 0, 2]).simplify(), c(&[1, 0, 2]));
        // 5-way merge
        assert_eq!(c(&[0, 0, 0, 0, 0]).simplify(), c(&[0]));
        assert_eq!(c(&[0, 0, 0, 0, 1]).simplify(), c(&[1]));
        assert_eq!(c(&[0, 0, 0, 1, 0]).simplify(), c(&[0, 1, 0]));
        assert_eq!(c(&[0, 0, 0, 1, 1]).simplify(), c(&[0]));
        assert_eq!(c(&[0, 0, 0, 1, 2]).simplify(), c(&[0, 1, 2]));
        assert_eq!(c(&[0, 0, 1, 0, 0]).simplify(), c(&[1]));
        assert_eq!(c(&[0, 0, 1, 0, 1]).simplify(), c(&[1, 0, 1]));
        assert_eq!(c(&[0, 0, 1, 0, 2]).simplify(), c(&[1, 0, 2]));
        assert_eq!(c(&[0, 0, 1, 1, 0]).simplify(), c(&[0]));
        assert_eq!(c(&[0, 0, 1, 1, 1]).simplify(), c(&[1]));
        assert_eq!(c(&[0, 0, 1, 1, 2]).simplify(), c(&[2]));
        assert_eq!(c(&[0, 0, 2, 1, 0]).simplify(), c(&[2, 1, 0]));
        assert_eq!(c(&[0, 0, 2, 1, 1]).simplify(), c(&[2]));
        assert_eq!(c(&[0, 0, 2, 1, 2]).simplify(), c(&[2, 1, 2]));
        assert_eq!(c(&[0, 0, 2, 1, 3]).simplify(), c(&[2, 1, 3]));
        assert_eq!(c(&[0, 1, 0, 0, 0]).simplify(), c(&[0, 1, 0]));
        assert_eq!(c(&[0, 1, 0, 0, 1]).simplify(), c(&[0]));
        assert_eq!(c(&[0, 1, 0, 0, 2]).simplify(), c(&[2, 1, 0]));
        assert_eq!(c(&[0, 1, 0, 1, 0]).simplify(), c(&[0, 1, 0, 1, 0]));
        assert_eq!(c(&[0, 1, 0, 1, 1]).simplify(), c(&[0, 1, 0]));
        assert_eq!(c(&[0, 1, 0, 1, 2]).simplify(), c(&[0, 1, 0, 1, 2]));
        assert_eq!(c(&[0, 1, 0, 2, 0]).simplify(), c(&[0, 1, 0, 2, 0]));
        assert_eq!(c(&[0, 1, 0, 2, 1]).simplify(), c(&[0, 2, 0]));
        assert_eq!(c(&[0, 1, 0, 2, 2]).simplify(), c(&[0, 1, 0]));
        assert_eq!(c(&[0, 1, 0, 2, 3]).simplify(), c(&[0, 1, 0, 2, 3]));
        assert_eq!(c(&[0, 1, 1, 0, 0]).simplify(), c(&[0]));
        assert_eq!(c(&[0, 1, 1, 0, 1]).simplify(), c(&[1]));
        assert_eq!(c(&[0, 1, 1, 0, 2]).simplify(), c(&[2]));
        assert_eq!(c(&[0, 1, 1, 1, 0]).simplify(), c(&[0, 1, 0]));
        assert_eq!(c(&[0, 1, 1, 1, 1]).simplify(), c(&[0]));
        assert_eq!(c(&[0, 1, 1, 1, 2]).simplify(), c(&[0, 1, 2]));
        assert_eq!(c(&[0, 1, 1, 2, 0]).simplify(), c(&[0, 2, 0]));
        assert_eq!(c(&[0, 1, 1, 2, 1]).simplify(), c(&[0, 2, 1]));
        assert_eq!(c(&[0, 1, 1, 2, 2]).simplify(), c(&[0]));
        assert_eq!(c(&[0, 1, 1, 2, 3]).simplify(), c(&[0, 2, 3]));
        assert_eq!(c(&[0, 1, 2, 0, 0]).simplify(), c(&[0, 1, 2]));
        assert_eq!(c(&[0, 1, 2, 0, 1]).simplify(), c(&[2]));
        assert_eq!(c(&[0, 1, 2, 0, 2]).simplify(), c(&[2, 1, 2]));
        assert_eq!(c(&[0, 1, 2, 0, 3]).simplify(), c(&[3, 1, 2]));
        assert_eq!(c(&[0, 1, 2, 1, 0]).simplify(), c(&[0, 1, 2, 1, 0]));
        assert_eq!(c(&[0, 1, 2, 1, 1]).simplify(), c(&[0, 1, 2]));
        assert_eq!(c(&[0, 1, 2, 1, 2]).simplify(), c(&[0, 1, 2, 1, 2]));
        assert_eq!(c(&[0, 1, 2, 1, 3]).simplify(), c(&[0, 1, 2, 1, 3]));
        assert_eq!(c(&[0, 1, 2, 2, 0]).simplify(), c(&[0, 1, 0]));
        assert_eq!(c(&[0, 1, 2, 2, 1]).simplify(), c(&[0]));
        assert_eq!(c(&[0, 1, 2, 2, 2]).simplify(), c(&[0, 1, 2]));
        assert_eq!(c(&[0, 1, 2, 2, 3]).simplify(), c(&[0, 1, 3]));
        assert_eq!(c(&[0, 1, 2, 3, 0]).simplify(), c(&[0, 1, 2, 3, 0]));
        assert_eq!(c(&[0, 1, 2, 3, 1]).simplify(), c(&[0, 3, 2]));
        assert_eq!(c(&[0, 1, 2, 3, 2]).simplify(), c(&[0, 1, 2, 3, 2]));
        assert_eq!(c(&[0, 1, 2, 3, 3]).simplify(), c(&[0, 1, 2]));
        assert_eq!(c(&[0, 1, 2, 3, 4]).simplify(), c(&[0, 1, 2, 3, 4]));
        assert_eq!(c(&[0, 1, 2, 3, 4, 5, 1]).simplify(), c(&[0, 3, 4, 5, 2]));
    }

    #[test]
    fn test_update_from_simplified() {
        // 1-way merge
        assert_eq!(c(&[0]).update_from_simplified(c(&[1])), c(&[1]));
        // 3-way merge
        assert_eq!(c(&[0, 0, 0]).update_from_simplified(c(&[1])), c(&[0, 0, 1]));
        assert_eq!(c(&[1, 0, 0]).update_from_simplified(c(&[2])), c(&[2, 0, 0]));
        assert_eq!(
            c(&[1, 0, 2]).update_from_simplified(c(&[2, 1, 3])),
            c(&[2, 1, 3])
        );
        // 5-way merge
        assert_eq!(
            c(&[0, 0, 0, 0, 0]).update_from_simplified(c(&[1])),
            c(&[0, 0, 0, 0, 1])
        );
        assert_eq!(
            c(&[0, 0, 0, 1, 0]).update_from_simplified(c(&[2, 3, 1])),
            c(&[0, 0, 2, 3, 1])
        );
        assert_eq!(
            c(&[0, 1, 0, 0, 0]).update_from_simplified(c(&[2, 3, 1])),
            c(&[0, 3, 1, 0, 2])
        );
        assert_eq!(
            c(&[2, 0, 3, 1, 4]).update_from_simplified(c(&[3, 1, 4, 2, 5])),
            c(&[3, 1, 4, 2, 5])
        );

        assert_eq!(c(&[0, 0, 3, 1, 3, 2, 4]).simplify(), c(&[3, 1, 3, 2, 4]));
        // Check that the `3`s are replaced correctly and that `4` ends up in the
        // correct position.
        assert_eq!(
            c(&[0, 0, 3, 1, 3, 2, 4]).update_from_simplified(c(&[10, 1, 11, 2, 4])),
            c(&[0, 0, 10, 1, 11, 2, 4])
        );
    }

    #[test]
    fn test_merge_invariants() {
        fn check_invariants(terms: &[u32]) {
            let merge = Merge::from_vec(terms.to_vec());
            // `simplify()` is idempotent
            assert_eq!(
                merge.simplify().simplify(),
                merge.simplify(),
                "simplify() not idempotent for {merge:?}"
            );
            // `resolve_trivial()` is unaffected by `simplify()`
            assert_eq!(
                merge.simplify().resolve_trivial(SameChange::Accept),
                merge.resolve_trivial(SameChange::Accept),
                "simplify() changed result of resolve_trivial() for {merge:?}"
            );
        }
        // 1-way merge
        check_invariants(&[0]);
        for i in 0..=1 {
            for j in 0..=i + 1 {
                // 3-way merge
                check_invariants(&[i, 0, j]);
                for k in 0..=j + 1 {
                    for l in 0..=k + 1 {
                        // 5-way merge
                        check_invariants(&[0, i, j, k, l]);
                    }
                }
            }
        }
    }

    #[test]
    fn test_swap_remove() {
        let mut x = c(&[0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(x.swap_remove(0, 1), (1, 2));
        assert_eq!(x, c(&[0, 5, 6, 3, 4]));
        assert_eq!(x.swap_remove(1, 0), (3, 0));
        assert_eq!(x, c(&[4, 5, 6]));
        assert_eq!(x.swap_remove(0, 1), (5, 6));
        assert_eq!(x, c(&[4]));
    }

    #[test]
    fn test_pad_to() {
        let mut x = c(&[1]);
        x.pad_to(3, &2);
        assert_eq!(x, c(&[1, 2, 2, 2, 2]));
        // No change if the requested size is smaller
        x.pad_to(1, &3);
        assert_eq!(x, c(&[1, 2, 2, 2, 2]));
    }

    #[test]
    fn test_iter() {
        // 1-way merge
        assert_eq!(c(&[1]).iter().collect_vec(), vec![&1]);
        // 5-way merge
        assert_eq!(
            c(&[1, 2, 3, 4, 5]).iter().collect_vec(),
            vec![&1, &2, &3, &4, &5]
        );
    }

    #[test]
    fn test_from_iter() {
        // 1-way merge
        assert_eq!(MergeBuilder::from_iter([1]).build(), c(&[1]));
        // 5-way merge
        assert_eq!(
            MergeBuilder::from_iter([1, 2, 3, 4, 5]).build(),
            c(&[1, 2, 3, 4, 5])
        );
    }

    #[test]
    #[should_panic]
    fn test_from_iter_empty() {
        MergeBuilder::from_iter([1; 0]).build();
    }

    #[test]
    #[should_panic]
    fn test_from_iter_even() {
        MergeBuilder::from_iter([1, 2]).build();
    }

    #[test]
    fn test_extend() {
        // 1-way merge
        let mut builder: MergeBuilder<i32> = Default::default();
        builder.extend([1]);
        assert_eq!(builder.build(), c(&[1]));
        // 5-way merge
        let mut builder: MergeBuilder<i32> = Default::default();
        builder.extend([1, 2]);
        builder.extend([3, 4, 5]);
        assert_eq!(builder.build(), c(&[1, 2, 3, 4, 5]));
    }

    #[test]
    fn test_map() {
        fn increment(i: &i32) -> i32 {
            i + 1
        }
        // 1-way merge
        assert_eq!(c(&[1]).map(increment), c(&[2]));
        // 3-way merge
        assert_eq!(c(&[1, 3, 5]).map(increment), c(&[2, 4, 6]));
    }

    #[test]
    fn test_try_map() {
        fn sqrt(i: &i32) -> Result<i32, ()> {
            if *i >= 0 {
                Ok(f64::from(*i).sqrt() as i32)
            } else {
                Err(())
            }
        }
        // 1-way merge
        assert_eq!(c(&[1]).try_map(sqrt), Ok(c(&[1])));
        assert_eq!(c(&[-1]).try_map(sqrt), Err(()));
        // 3-way merge
        assert_eq!(c(&[1, 4, 9]).try_map(sqrt), Ok(c(&[1, 2, 3])));
        assert_eq!(c(&[-1, 4, 9]).try_map(sqrt), Err(()));
        assert_eq!(c(&[1, -4, 9]).try_map(sqrt), Err(()));
    }

    #[test]
    fn test_flatten() {
        // 1-way merge of 1-way merge
        assert_eq!(c(&[c(&[0])]).flatten(), c(&[0]));
        // 1-way merge of 3-way merge
        assert_eq!(c(&[c(&[0, 1, 2])]).flatten(), c(&[0, 1, 2]));
        // 3-way merge of 1-way merges
        assert_eq!(c(&[c(&[0]), c(&[1]), c(&[2])]).flatten(), c(&[0, 1, 2]));
        // 3-way merge of 3-way merges
        assert_eq!(
            c(&[c(&[0, 1, 2]), c(&[3, 4, 5]), c(&[6, 7, 8])]).flatten(),
            c(&[0, 1, 2, 5, 4, 3, 6, 7, 8])
        );
    }
}
