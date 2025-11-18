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

//! Labels for conflicted trees.

use std::fmt;
use std::sync::Arc;

use crate::content_hash::ContentHash;
use crate::merge::Merge;

/// Optionally contains a set of labels for the terms of a conflict. Resolved
/// merges cannot be labeled. The conflict labels are reference-counted to make
/// them more efficient to clone.
#[derive(ContentHash, PartialEq, Eq, Clone)]
pub struct ConflictLabels {
    labels: Option<Arc<Merge<String>>>,
}

impl ConflictLabels {
    /// Create a `ConflictLabels` with no labels.
    pub const fn unlabeled() -> Self {
        Self { labels: None }
    }

    /// Create a `ConflictLabels` from a `Merge<String>`. If the merge is
    /// resolved or if any label is empty, the labels will be discarded, since
    /// resolved merges cannot have labels, and labels cannot be empty.
    pub fn new(labels: Merge<String>) -> Self {
        if labels.is_resolved() || labels.iter().any(|label| label.is_empty()) {
            Self::unlabeled()
        } else {
            Self {
                labels: Some(Arc::new(labels)),
            }
        }
    }

    /// Create a `ConflictLabels` from a `Vec<String>`, with an empty vec
    /// representing no labels.
    pub fn from_vec(labels: Vec<String>) -> Self {
        if labels.is_empty() {
            Self::unlabeled()
        } else {
            Self::new(Merge::from_vec(labels))
        }
    }

    /// Returns true if there are labels present.
    pub fn is_present(&self) -> bool {
        self.labels.is_some()
    }

    /// Returns the number of labeled sides, or `None` if unlabeled.
    pub fn num_sides(&self) -> Option<usize> {
        self.labels.as_ref().map(|labels| labels.num_sides())
    }

    /// Returns the underlying labels as an `Option<&Merge<String>>`.
    pub fn as_merge(&self) -> Option<&Merge<String>> {
        self.labels.as_ref().map(Arc::as_ref)
    }

    /// Returns the underlying labels as an `Option<Merge<String>>`, cloning if
    /// necessary.
    pub fn into_merge(self) -> Option<Merge<String>> {
        self.labels.map(Arc::unwrap_or_clone)
    }

    /// Returns the conflict labels as a slice. If there are no labels, returns
    /// an empty slice.
    pub fn as_slice(&self) -> &[String] {
        self.as_merge().map_or(&[], |labels| labels.as_slice())
    }

    /// Get the label for a side at an index.
    pub fn get_add(&self, add_index: usize) -> Option<&str> {
        self.as_merge()
            .and_then(|merge| merge.get_add(add_index).map(String::as_str))
    }

    /// Get the label for a base at an index.
    pub fn get_remove(&self, remove_index: usize) -> Option<&str> {
        self.as_merge()
            .and_then(|merge| merge.get_remove(remove_index).map(String::as_str))
    }

    /// Simplify a merge with the same number of sides while preserving the
    /// conflict labels corresponding to each side of the merge.
    pub fn simplify_with<T: PartialEq + Clone>(&self, merge: &Merge<T>) -> (Self, Merge<T>) {
        if let Some(labels) = self.as_merge() {
            let (labels, simplified) = labels
                .as_ref()
                .zip(merge.as_ref())
                .simplify_by(|&(_label, item)| item)
                .unzip();
            (Self::new(labels.cloned()), simplified.cloned())
        } else {
            let simplified = merge.simplify();
            (Self::unlabeled(), simplified)
        }
    }
}

impl From<Merge<String>> for ConflictLabels {
    fn from(value: Merge<String>) -> Self {
        Self::new(value)
    }
}

impl From<Merge<&'_ str>> for ConflictLabels {
    fn from(value: Merge<&str>) -> Self {
        Self::new(value.map(|&label| label.to_owned()))
    }
}

impl<T> From<Option<T>> for ConflictLabels
where
    T: Into<Self>,
{
    fn from(value: Option<T>) -> Self {
        value.map_or_else(Self::unlabeled, T::into)
    }
}

impl fmt::Debug for ConflictLabels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(labels) = self.as_merge() {
            f.debug_tuple("Labeled").field(&labels.as_slice()).finish()
        } else {
            write!(f, "Unlabeled")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conflict_labels_from_vec() {
        // From empty vec for unlabeled
        assert_eq!(
            ConflictLabels::from_vec(vec![]),
            ConflictLabels::unlabeled()
        );
        // From non-empty vec of terms
        assert_eq!(
            ConflictLabels::from_vec(vec![
                String::from("left"),
                String::from("base"),
                String::from("right")
            ]),
            ConflictLabels::from(Some(Merge::from_vec(vec!["left", "base", "right"])))
        );
    }

    #[test]
    fn test_conflict_labels_as_slice() {
        // Empty slice for unlabeled
        let empty: &[String] = &[];
        assert_eq!(ConflictLabels::unlabeled().as_slice(), empty);
        // Slice of terms for labeled
        assert_eq!(
            ConflictLabels::from(Some(Merge::from_vec(vec!["left", "base", "right"]))).as_slice(),
            &[
                String::from("left"),
                String::from("base"),
                String::from("right")
            ]
        );
    }
}
