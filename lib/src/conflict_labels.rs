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

use crate::merge::Merge;

/// Optionally contains a set of labels for the terms of a conflict. Resolved
/// merges cannot be labeled.
#[derive(PartialEq, Eq, Clone)]
pub struct ConflictLabels {
    // If the merge is resolved, the label must be empty.
    labels: Merge<String>,
}

impl ConflictLabels {
    /// Create a `ConflictLabels` with no labels.
    pub const fn unlabeled() -> Self {
        Self {
            labels: Merge::resolved(String::new()),
        }
    }

    /// Create a `ConflictLabels` from a `Merge<String>`. If the merge is
    /// resolved, the labels will be discarded since resolved merges cannot have
    /// labels.
    pub fn from_merge(labels: Merge<String>) -> Self {
        if labels.is_resolved() || labels.iter().all(|label| label.is_empty()) {
            Self::unlabeled()
        } else {
            Self { labels }
        }
    }

    /// Create a `ConflictLabels` from a `Vec<String>`, with an empty vec
    /// representing no labels.
    pub fn from_vec(labels: Vec<String>) -> Self {
        if labels.is_empty() {
            Self::unlabeled()
        } else {
            Self::from_merge(Merge::from_vec(labels))
        }
    }

    /// Returns true if there are labels present.
    pub fn has_labels(&self) -> bool {
        !self.labels.is_resolved()
    }

    /// Returns the number of sides of the underlying merge if any terms have
    /// labels, or `None` if there are no labels.
    pub fn num_sides(&self) -> Option<usize> {
        self.has_labels().then_some(self.labels.num_sides())
    }

    /// Returns the underlying `Merge<String>`.
    pub fn as_merge(&self) -> &Merge<String> {
        &self.labels
    }

    /// Extracts the underlying `Merge<String>`.
    pub fn into_merge(self) -> Merge<String> {
        self.labels
    }

    /// Returns the conflict labels as a slice. If there are no labels, returns
    /// an empty slice.
    pub fn as_slice(&self) -> &[String] {
        if self.has_labels() {
            self.labels.as_slice()
        } else {
            &[]
        }
    }

    /// Get the label for a side at an index.
    pub fn get_add(&self, add_index: usize) -> Option<&str> {
        self.labels
            .get_add(add_index)
            .filter(|label| !label.is_empty())
            .map(String::as_str)
    }

    /// Get the label for a base at an index.
    pub fn get_remove(&self, remove_index: usize) -> Option<&str> {
        self.labels
            .get_remove(remove_index)
            .filter(|label| !label.is_empty())
            .map(String::as_str)
    }
}

impl fmt::Debug for ConflictLabels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.has_labels() {
            f.debug_tuple("Labeled")
                .field(&self.labels.as_slice())
                .finish()
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
            ConflictLabels::from_merge(Merge::from_vec(vec![
                String::from("left"),
                String::from("base"),
                String::from("right")
            ]))
        );
    }

    #[test]
    fn test_conflict_labels_as_slice() {
        // Empty slice for unlabeled
        let empty: &[String] = &[];
        assert_eq!(ConflictLabels::unlabeled().as_slice(), empty);
        // Slice of terms for labeled
        assert_eq!(
            ConflictLabels::from_merge(Merge::from_vec(vec![
                String::from("left"),
                String::from("base"),
                String::from("right")
            ]))
            .as_slice(),
            &[
                String::from("left"),
                String::from("base"),
                String::from("right")
            ]
        );
    }
}
