// Copyright 2023-2025 The Jujutsu Authors
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

//! Merge trees by recursing into entries (subtrees, files)

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::iter::zip;
use std::sync::Arc;
use std::vec;

use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::future::BoxFuture;
use futures::future::try_join_all;
use futures::stream::FuturesUnordered;
use itertools::Itertools as _;
use tokio::io::AsyncReadExt as _;

use crate::backend;
use crate::backend::BackendError;
use crate::backend::BackendResult;
use crate::backend::TreeId;
use crate::backend::TreeValue;
use crate::config::ConfigGetError;
use crate::files;
use crate::files::FileMergeHunkLevel;
use crate::merge::Merge;
use crate::merge::MergedTreeVal;
use crate::merge::MergedTreeValue;
use crate::merge::SameChange;
use crate::merged_tree::all_merged_tree_entries;
use crate::object_id::ObjectId as _;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::repo_path::RepoPathComponentBuf;
use crate::settings::UserSettings;
use crate::store::Store;
use crate::tree::Tree;

/// Options for tree/file conflict resolution.
#[derive(Clone, Debug)]
pub struct MergeOptions {
    /// Granularity of hunks when merging files.
    pub hunk_level: FileMergeHunkLevel,
    /// Whether to resolve conflict that makes the same change at all sides.
    pub same_change: SameChange,
}

impl MergeOptions {
    /// Loads merge options from `settings`.
    pub fn from_settings(settings: &UserSettings) -> Result<Self, ConfigGetError> {
        Ok(Self {
            // Maybe we can add hunk-level=file to disable content merging if
            // needed. It wouldn't be translated to FileMergeHunkLevel.
            hunk_level: settings.get("merge.hunk-level")?,
            same_change: settings.get("merge.same-change")?,
        })
    }
}

/// The returned conflict will either be resolved or have the same number of
/// sides as the input.
pub async fn merge_trees(store: &Arc<Store>, merge: Merge<TreeId>) -> BackendResult<Merge<TreeId>> {
    let merge = match merge.into_resolved() {
        Ok(tree) => return Ok(Merge::resolved(tree)),
        Err(merge) => merge,
    };

    let mut merger = TreeMerger {
        store: store.clone(),
        trees_to_resolve: BTreeMap::new(),
        work: FuturesUnordered::new(),
        unstarted_work: BTreeMap::new(),
    };
    merger.enqueue_tree_read(
        RepoPathBuf::root(),
        merge.map(|tree_id| Some(TreeValue::Tree(tree_id.clone()))),
    );
    let trees = merger.merge().await?;
    Ok(trees.map(|tree| tree.id().clone()))
}

struct MergedTreeInput {
    resolved: BTreeMap<RepoPathComponentBuf, TreeValue>,
    /// Entries that we're currently waiting for data for in order to resolve
    /// them. When this set becomes empty, we're ready to write the tree(s).
    pending_lookup: HashSet<RepoPathComponentBuf>,
    conflicts: BTreeMap<RepoPathComponentBuf, MergedTreeValue>,
}

impl MergedTreeInput {
    fn new(resolved: BTreeMap<RepoPathComponentBuf, TreeValue>) -> Self {
        Self {
            resolved,
            pending_lookup: HashSet::new(),
            conflicts: BTreeMap::new(),
        }
    }

    fn mark_completed(
        &mut self,
        basename: RepoPathComponentBuf,
        value: MergedTreeValue,
        same_change: SameChange,
    ) {
        let was_pending = self.pending_lookup.remove(&basename);
        assert!(was_pending, "No pending lookup for {basename:?}");
        if let Some(resolved) = value.resolve_trivial(same_change) {
            if let Some(resolved) = resolved.as_ref() {
                self.resolved.insert(basename, resolved.clone());
            }
        } else {
            self.conflicts.insert(basename, value);
        }
    }

    fn into_backend_trees(self) -> Merge<backend::Tree> {
        assert!(self.pending_lookup.is_empty());

        fn by_name(
            (name1, _): &(RepoPathComponentBuf, TreeValue),
            (name2, _): &(RepoPathComponentBuf, TreeValue),
        ) -> bool {
            name1 < name2
        }

        if self.conflicts.is_empty() {
            let all_entries = self.resolved.into_iter().collect();
            Merge::resolved(backend::Tree::from_sorted_entries(all_entries))
        } else {
            // Create a Merge with the conflict entries for each side.
            let mut conflict_entries = self.conflicts.first_key_value().unwrap().1.map(|_| vec![]);
            for (basename, value) in self.conflicts {
                assert_eq!(value.num_sides(), conflict_entries.num_sides());
                for (entries, value) in zip(&mut conflict_entries, value) {
                    if let Some(value) = value {
                        entries.push((basename.clone(), value));
                    }
                }
            }

            let mut backend_trees = vec![];
            for entries in conflict_entries {
                let backend_tree = backend::Tree::from_sorted_entries(
                    self.resolved
                        .iter()
                        .map(|(name, value)| (name.clone(), value.clone()))
                        .merge_by(entries, by_name)
                        .collect(),
                );
                backend_trees.push(backend_tree);
            }
            Merge::from_vec(backend_trees)
        }
    }
}

/// The result from an asynchronously scheduled work item.
enum TreeMergerWorkOutput {
    /// Trees that have been read (i.e. `Read` is past tense)
    ReadTrees {
        dir: RepoPathBuf,
        result: BackendResult<Merge<Tree>>,
    },
    WrittenTrees {
        dir: RepoPathBuf,
        result: BackendResult<Merge<Tree>>,
    },
    MergedFiles {
        path: RepoPathBuf,
        result: BackendResult<MergedTreeValue>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum TreeMergeWorkItemKey {
    // `MergeFiles` variant before `ReadTrees` so files are polled before trees because they
    // typically take longer to process.
    MergeFiles { path: RepoPathBuf },
    ReadTrees { dir: RepoPathBuf },
}

struct TreeMerger {
    store: Arc<Store>,
    // Trees we're currently working on.
    trees_to_resolve: BTreeMap<RepoPathBuf, MergedTreeInput>,
    // Futures we're currently processing. In order to respect the backend's concurrency limit.
    work: FuturesUnordered<BoxFuture<'static, TreeMergerWorkOutput>>,
    // Futures we haven't started polling yet, in order to respect the backend's concurrency limit.
    unstarted_work: BTreeMap<TreeMergeWorkItemKey, BoxFuture<'static, TreeMergerWorkOutput>>,
}

impl TreeMerger {
    async fn merge(mut self) -> BackendResult<Merge<Tree>> {
        while let Some(work_item) = self.work.next().await {
            match work_item {
                TreeMergerWorkOutput::ReadTrees { dir, result } => {
                    let tree = result?;
                    self.process_tree(dir, tree);
                }
                TreeMergerWorkOutput::WrittenTrees { dir, result } => {
                    let tree = result?;
                    if dir.is_root() {
                        assert!(self.trees_to_resolve.is_empty());
                        assert!(self.work.is_empty());
                        assert!(self.unstarted_work.is_empty());
                        return Ok(tree);
                    }
                    // Propagate the write to the parent tree, replacing empty trees by `None`.
                    let new_value = tree.map(|tree| {
                        (tree.id() != self.store.empty_tree_id())
                            .then(|| TreeValue::Tree(tree.id().clone()))
                    });
                    self.mark_completed(&dir, new_value);
                }
                TreeMergerWorkOutput::MergedFiles { path, result } => {
                    let value = result?;
                    self.mark_completed(&path, value);
                }
            }

            while self.work.len() < self.store.concurrency() {
                if let Some((_key, work)) = self.unstarted_work.pop_first() {
                    self.work.push(work);
                } else {
                    break;
                }
            }
        }

        unreachable!("There was no work item for writing the root tree");
    }

    fn process_tree(&mut self, dir: RepoPathBuf, tree: Merge<Tree>) {
        // First resolve trivial merges (those that we don't need to load any more data
        // for)
        let same_change = self.store.merge_options().same_change;
        let mut resolved = vec![];
        let mut non_trivial = vec![];
        for (basename, path_merge) in all_merged_tree_entries(&tree) {
            if let Some(value) = path_merge.resolve_trivial(same_change) {
                if let Some(value) = value.cloned() {
                    resolved.push((basename.to_owned(), value));
                }
            } else {
                non_trivial.push((basename.to_owned(), path_merge.cloned()));
            }
        }

        // If there are no non-trivial merges, we can write the tree now.
        if non_trivial.is_empty() {
            let backend_trees = Merge::resolved(backend::Tree::from_sorted_entries(resolved));
            self.enqueue_tree_write(dir, backend_trees);
            return;
        }

        let mut unmerged_tree = MergedTreeInput::new(resolved.into_iter().collect());
        for (basename, value) in non_trivial {
            let path = dir.join(&basename);
            unmerged_tree.pending_lookup.insert(basename);
            if value.is_tree() {
                self.enqueue_tree_read(path, value);
            } else {
                // TODO: If it's e.g. a dir/file conflict, there's no need to try to
                // resolve it as a file. We should mark them to
                // `unmerged_tree.conflicts` instead.
                self.enqueue_file_merge(path, value);
            }
        }

        self.trees_to_resolve.insert(dir, unmerged_tree);
    }

    fn enqueue_tree_read(&mut self, dir: RepoPathBuf, value: MergedTreeValue) {
        let key = TreeMergeWorkItemKey::ReadTrees { dir: dir.clone() };
        let work_fut = read_trees(self.store.clone(), dir.clone(), value)
            .map(|result| TreeMergerWorkOutput::ReadTrees { dir, result });
        if self.work.len() < self.store.concurrency() {
            self.work.push(Box::pin(work_fut));
        } else {
            self.unstarted_work.insert(key, Box::pin(work_fut));
        }
    }

    fn enqueue_tree_write(&mut self, dir: RepoPathBuf, backend_trees: Merge<backend::Tree>) {
        let work_fut = write_trees(self.store.clone(), dir.clone(), backend_trees)
            .map(|result| TreeMergerWorkOutput::WrittenTrees { dir, result });
        // Bypass the `unstarted_work` queue because writing trees usually results in
        // saving memory (each tree gets replaced by a `TreeValue::Tree`)
        self.work.push(Box::pin(work_fut));
    }

    fn enqueue_file_merge(&mut self, path: RepoPathBuf, value: MergedTreeValue) {
        let key = TreeMergeWorkItemKey::MergeFiles { path: path.clone() };
        let work_fut = resolve_file_values_owned(self.store.clone(), path.clone(), value)
            .map(|result| TreeMergerWorkOutput::MergedFiles { path, result });
        if self.work.len() < self.store.concurrency() {
            self.work.push(Box::pin(work_fut));
        } else {
            self.unstarted_work.insert(key, Box::pin(work_fut));
        }
    }

    fn mark_completed(&mut self, path: &RepoPath, value: MergedTreeValue) {
        let (dir, basename) = path.split().unwrap();
        let tree = self.trees_to_resolve.get_mut(dir).unwrap();
        let same_change = self.store.merge_options().same_change;
        tree.mark_completed(basename.to_owned(), value, same_change);
        // If all entries in this tree have been processed (either resolved or still a
        // conflict), schedule the writing of the tree(s) to the backend.
        if tree.pending_lookup.is_empty() {
            let tree = self.trees_to_resolve.remove(dir).unwrap();
            self.enqueue_tree_write(dir.to_owned(), tree.into_backend_trees());
        }
    }
}

async fn read_trees(
    store: Arc<Store>,
    dir: RepoPathBuf,
    value: MergedTreeValue,
) -> BackendResult<Merge<Tree>> {
    let trees = value
        .to_tree_merge(&store, &dir)
        .await?
        .expect("Should be tree merge");
    Ok(trees)
}

async fn write_trees(
    store: Arc<Store>,
    dir: RepoPathBuf,
    backend_trees: Merge<backend::Tree>,
) -> BackendResult<Merge<Tree>> {
    // TODO: Could use `backend_trees.try_map_async()` here if it took `self` by
    // value or if `Backend::write_tree()` to an `Arc<backend::Tree>`.
    let trees = try_join_all(
        backend_trees
            .into_iter()
            .map(|backend_tree| store.write_tree(&dir, backend_tree)),
    )
    .await?;
    Ok(Merge::from_vec(trees))
}

async fn resolve_file_values_owned(
    store: Arc<Store>,
    path: RepoPathBuf,
    values: MergedTreeValue,
) -> BackendResult<MergedTreeValue> {
    let maybe_resolved = try_resolve_file_values(&store, &path, &values).await?;
    Ok(maybe_resolved.unwrap_or(values))
}

/// Tries to resolve file conflicts by merging the file contents. Treats missing
/// files as empty. If the file conflict cannot be resolved, returns the passed
/// `values` unmodified.
pub async fn resolve_file_values(
    store: &Arc<Store>,
    path: &RepoPath,
    values: MergedTreeValue,
) -> BackendResult<MergedTreeValue> {
    let same_change = store.merge_options().same_change;
    if let Some(resolved) = values.resolve_trivial(same_change) {
        return Ok(Merge::resolved(resolved.clone()));
    }

    let maybe_resolved = try_resolve_file_values(store, path, &values).await?;
    Ok(maybe_resolved.unwrap_or(values))
}

async fn try_resolve_file_values<T: Borrow<TreeValue>>(
    store: &Arc<Store>,
    path: &RepoPath,
    values: &Merge<Option<T>>,
) -> BackendResult<Option<MergedTreeValue>> {
    // The values may contain trees canceling each other (notably padded absent
    // trees), so we need to simplify them first.
    let simplified = values
        .map(|value| value.as_ref().map(Borrow::borrow))
        .simplify();
    // No fast path for simplified.is_resolved(). If it could be resolved, it would
    // have been caught by values.resolve_trivial() above.
    if let Some(resolved) = try_resolve_file_conflict(store, path, &simplified).await? {
        Ok(Some(Merge::normal(resolved)))
    } else {
        // Failed to merge the files, or the paths are not files
        Ok(None)
    }
}

/// Resolves file-level conflict by merging content hunks.
///
/// The input `conflict` is supposed to be simplified. It shouldn't contain
/// non-file values that cancel each other.
async fn try_resolve_file_conflict(
    store: &Store,
    filename: &RepoPath,
    conflict: &MergedTreeVal<'_>,
) -> BackendResult<Option<TreeValue>> {
    let options = store.merge_options();
    // If there are any non-file or any missing parts in the conflict, we can't
    // merge it. We check early so we don't waste time reading file contents if
    // we can't merge them anyway. At the same time we determine whether the
    // resulting file should be executable.
    let Ok(file_id_conflict) = conflict.try_map(|term| match term {
        Some(TreeValue::File {
            id,
            executable: _,
            copy_id: _,
        }) => Ok(id),
        _ => Err(()),
    }) else {
        return Ok(None);
    };
    let Ok(executable_conflict) = conflict.try_map(|term| match term {
        Some(TreeValue::File {
            id: _,
            executable,
            copy_id: _,
        }) => Ok(executable),
        _ => Err(()),
    }) else {
        return Ok(None);
    };
    let Ok(copy_id_conflict) = conflict.try_map(|term| match term {
        Some(TreeValue::File {
            id: _,
            executable: _,
            copy_id,
        }) => Ok(copy_id),
        _ => Err(()),
    }) else {
        return Ok(None);
    };
    // TODO: Whether to respect options.same_change to merge executable and
    // copy_id? Should also update conflicts::resolve_file_executable().
    let Some(&&executable) = executable_conflict.resolve_trivial(SameChange::Accept) else {
        // We're unable to determine whether the result should be executable
        return Ok(None);
    };
    let Some(&copy_id) = copy_id_conflict.resolve_trivial(SameChange::Accept) else {
        // We're unable to determine the file's copy ID
        return Ok(None);
    };
    if let Some(&resolved_file_id) = file_id_conflict.resolve_trivial(options.same_change) {
        // Don't bother reading the file contents if the conflict can be trivially
        // resolved.
        return Ok(Some(TreeValue::File {
            id: resolved_file_id.clone(),
            executable,
            copy_id: copy_id.clone(),
        }));
    }

    // While the input conflict should be simplified by caller, it might contain
    // terms which only differ in executable bits. Simplify the conflict further
    // for two reasons:
    // 1. Avoid reading unchanged file contents
    // 2. The simplified conflict can sometimes be resolved when the unsimplfied one
    //    cannot
    let file_id_conflict = file_id_conflict.simplify();

    let contents = file_id_conflict
        .try_map_async(async |file_id| {
            let mut content = vec![];
            let mut reader = store.read_file(filename, file_id).await?;
            reader
                .read_to_end(&mut content)
                .await
                .map_err(|err| BackendError::ReadObject {
                    object_type: file_id.object_type(),
                    hash: file_id.hex(),
                    source: err.into(),
                })?;
            BackendResult::Ok(content)
        })
        .await?;
    if let Some(merged_content) = files::try_merge(&contents, options) {
        let id = store
            .write_file(filename, &mut merged_content.as_slice())
            .await?;
        Ok(Some(TreeValue::File {
            id,
            executable,
            copy_id: copy_id.clone(),
        }))
    } else {
        Ok(None)
    }
}
