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

//! Rebases selected local commits after Git remote bookmarks move forward.
//!
//! This module provides the rebase step for a `git pull --rebase`-like
//! workflow. The caller selects local commits before fetching, imports the
//! updated remote bookmarks, and then asks this module to rebase the selected
//! commits onto the new remote targets.
//!
//! Multiple independent remote bookmark updates can be applied in one rewrite.
//! Bookmarks and working copies follow the rewritten commits. Heads of
//! anonymous branches are updated as well, so those branches remain visible.
//!
//! The operation is intentionally conservative. Unrelated remote updates are
//! ignored. It rejects deletions, conflicts, force-pushes, incomplete
//! selections, and overlapping updates that cannot be applied without losing
//! commits or parent relationships.
//!
//! See [`sync_imported_refs()`] for the phase and transaction-safety contract.

use std::collections::HashMap;
use std::collections::VecDeque;

use futures::future::try_join_all;
use indexmap::IndexMap;
use indexmap::IndexSet;
use itertools::Itertools as _;
use thiserror::Error;

use crate::backend::BackendError;
use crate::backend::CommitId;
use crate::commit::Commit;
use crate::git::GitImportRefUpdate;
use crate::git::GitImportStats;
use crate::index::IndexError;
use crate::op_store::RefTarget;
use crate::ref_name::RefNameBuf;
use crate::ref_name::RemoteRefSymbolBuf;
use crate::repo::MutableRepo;
use crate::repo::Repo as _;
use crate::rewrite::EmptyBehavior;
use crate::rewrite::RebaseOptions;
use crate::rewrite::RebasedCommit;
use crate::rewrite::rebase_commit_with_options;

/// Rebases selected commits onto updated tracked Git remote bookmarks.
///
/// For each fast-forward update, this function rebases selected commits with a
/// parent between the old and new remote targets onto the new target. Their
/// selected descendants are rewritten as part of the same operation. Multiple
/// independent updates can be applied in one rewrite.
///
/// # Rebase policy
///
/// The caller's empty-commit and merge-simplification policies apply to rewrite
/// roots. Other rewritten commits keep empty commits and explicit merge
/// parents.
///
/// Reference rewriting for the whole operation remains controlled by
/// `rebase_options.rewrite_refs`.
///
/// # Returns
///
/// The returned map contains every commit rewritten or abandoned by this
/// operation, keyed by its original commit ID.
///
/// Unaffected selected commits and commits outside the selection are omitted.
///
/// # Errors
///
/// Returns an error if:
///
/// - the repository has pending rewrite mappings;
/// - an imported target is absent or conflicted, or did not move by
///   fast-forward;
/// - imported updates disagree or overlap in a way that could lose a selected
///   commit or parent edge;
/// - the selection omits an intermediate commit required by the rebase; or
/// - an imported destination would itself be rewritten.
///
/// Backend and index failures are propagated.
///
/// # Preconditions
///
/// - The commits to synchronize must be selected before fetching, and their IDs
///   passed in `selected_commit_ids`.
/// - `import_stats` must come from the immediately following Git import on the
///   same `mut_repo`.
/// - The function must be called before processing rewrite mappings left by the
///   import or otherwise rewriting the selected commits.
///
/// The function checks that the repository has no pending rewrite mappings,
/// but cannot verify when the commits were selected or where `import_stats`
/// came from.
///
/// If the import leaves rewrite mappings, the pre-fetch commit IDs can no
/// longer be used safely. The caller must abort or use another fallback.
///
/// # Transaction safety
///
/// The caller must discard the transaction if this function returns an error
/// or if its future is cancelled before completion. The function cannot roll
/// back partial in-memory mutations.
pub async fn sync_imported_refs(
    mut_repo: &mut MutableRepo,
    import_stats: &GitImportStats,
    selected_commit_ids: Vec<CommitId>,
    rebase_options: &RebaseOptions,
) -> Result<HashMap<CommitId, RebasedCommit>, GitSyncError> {
    if mut_repo.has_rewrites() {
        return Err(GitSyncError::PendingRewrites);
    }

    let selected = SelectedGraph::load(mut_repo, selected_commit_ids).await?;
    let moves = derive_boundary_moves(mut_repo, &selected, import_stats).await?;
    let plan = GitSyncPlanBuilder::new(mut_repo, selected, moves)
        .build()
        .await?;
    plan.apply(mut_repo, rebase_options).await
}

/// Why an imported remote bookmark update cannot be synchronized.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum GitSyncUnsupportedReason {
    /// The old remote target does not resolve to a single commit.
    #[error("the old remote target {target:?} is conflicted")]
    OldTargetConflicted {
        /// The conflicted target recorded before import.
        target: RefTarget,
    },
    /// The imported remote bookmark no longer points to a commit.
    #[error("the imported target is absent")]
    NewTargetAbsent,
    /// The imported remote bookmark points to a conflicted target.
    #[error("the imported target {target:?} is conflicted")]
    NewTargetConflicted {
        /// The conflicted target recorded after import.
        target: RefTarget,
    },
    /// The imported target is not a descendant of the previous target.
    #[error("moving from {old} to the imported target {new} is not a fast-forward")]
    NonFastForward {
        /// The previous remote commit.
        old: CommitId,
        /// The imported remote commit.
        new: CommitId,
    },
}

/// Errors from synchronizing selected commits with imported Git updates.
#[derive(Debug, Error)]
pub enum GitSyncError {
    /// The transaction already contains rewrite mappings.
    #[error("the transaction has pending rewrites")]
    PendingRewrites,
    /// An imported remote bookmark update cannot be synchronized.
    #[error("unsupported remote bookmark change {symbol}: {reason}")]
    UnsupportedRemoteChange {
        /// The imported remote reference that changed.
        symbol: RemoteRefSymbolBuf,
        /// The reason the imported change cannot be synchronized.
        reason: GitSyncUnsupportedReason,
    },
    /// Tracked remote bookmarks with the same name were imported with
    /// different destinations.
    #[error(
        "tracked remote bookmarks named {name} have different destinations: {destinations:?}",
        name = .name.as_str()
    )]
    AmbiguousBookmark {
        /// The tracked bookmark name shared by the updates.
        name: RefNameBuf,
        /// The distinct destinations observed for the bookmark.
        destinations: Vec<CommitId>,
    },
    /// The same old target would move to different destinations.
    #[error("upstream boundary {old} has different destinations: {destinations:?}")]
    AmbiguousBoundary {
        /// The old upstream commit shared by the updates.
        old: CommitId,
        /// The distinct replacement destinations observed for that commit.
        destinations: Vec<CommitId>,
    },
    /// The same parent of a selected commit would be replaced with different
    /// destinations.
    #[error(
        "selected commit {commit_id} would replace parent {parent_id} with different \
         destinations: {destinations:?}"
    )]
    ConflictingParentReplacement {
        /// The selected commit whose parent list would conflict.
        commit_id: CommitId,
        /// The parent commit targeted for replacement.
        parent_id: CommitId,
        /// The distinct replacement destinations requested for that parent.
        destinations: Vec<CommitId>,
    },
    /// A commit would be both a rewrite root and a descendant of another
    /// rewrite root.
    #[error(
        "selected commits {commit_ids:?} would be rewrite roots for one remote update and \
         structural descendants of another"
    )]
    ConflictingRewriteRoles {
        /// The selected commits that would receive both rewrite roles.
        commit_ids: Vec<CommitId>,
    },
    /// A selected parent would be replaced even though it is already being
    /// rewritten.
    #[error(
        "selected commit {commit_id} would replace selected parent {parent_id} with imported \
         target {destination}"
    )]
    OverlappingRewrite {
        /// The selected commit whose parent would be replaced.
        commit_id: CommitId,
        /// The selected parent that is already part of the rewrite.
        parent_id: CommitId,
        /// The imported target requested as the replacement parent.
        destination: CommitId,
    },
    /// The selection omits an intermediate commit needed to follow an update.
    #[error(
        "selected commits {commit_ids:?} cannot follow remote update {old} -> {new} without \
         rewriting unselected intermediate commits"
    )]
    SelectionBoundary {
        /// The old upstream commit.
        old: CommitId,
        /// The imported upstream commit.
        new: CommitId,
        /// Selected commits that cannot be reached from the rewrite roots.
        commit_ids: Vec<CommitId>,
    },
    /// An imported destination would itself be rewritten by the plan.
    #[error("imported destinations would be rewritten: {commit_ids:?}")]
    MovingDestination {
        /// Imported destinations that are also part of the rewrite plan.
        commit_ids: Vec<CommitId>,
    },
    /// A backend operation failed while loading or rewriting commits.
    #[error(transparent)]
    Backend(#[from] BackendError),
    /// The commit index could not answer a graph query.
    #[error(transparent)]
    Index(#[from] IndexError),
}

/// The validated set of commits and parent overrides to rewrite.
#[derive(Debug)]
struct GitSyncPlan {
    commits_to_rewrite: Vec<Commit>,
    root_parent_overrides: HashMap<CommitId, Vec<CommitId>>,
}

impl GitSyncPlan {
    async fn apply(
        self,
        mut_repo: &mut MutableRepo,
        rebase_options: &RebaseOptions,
    ) -> Result<HashMap<CommitId, RebasedCommit>, GitSyncError> {
        let Self {
            commits_to_rewrite,
            root_parent_overrides,
        } = self;
        if commits_to_rewrite.is_empty() {
            return Ok(HashMap::new());
        }

        // Rewrite roots honor the caller's empty-commit and merge-simplification
        // policies. Structural descendants keep empty commits and retain explicit
        // ancestor merge parents. Reference rewriting remains caller-controlled.
        let descendant_options = RebaseOptions {
            empty: EmptyBehavior::Keep,
            rewrite_refs: rebase_options.rewrite_refs.clone(),
            simplify_ancestor_merge: false,
        };
        let mut rebased_commits = HashMap::new();
        mut_repo
            .transform_commits_exact(
                commits_to_rewrite,
                &root_parent_overrides,
                &rebase_options.rewrite_refs,
                async |rewriter| {
                    let old_id = rewriter.old_commit().id().clone();
                    let options = if root_parent_overrides.contains_key(&old_id) {
                        rebase_options
                    } else {
                        &descendant_options
                    };
                    let rebased = rebase_commit_with_options(rewriter, options).await?;
                    rebased_commits.insert(old_id, rebased);
                    Ok(())
                },
            )
            .await?;
        Ok(rebased_commits)
    }
}

/// The old and new targets of a remote bookmark fast-forward.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct BoundaryMove {
    old: CommitId,
    new: CommitId,
}

/// A remote bookmark fast-forward and the selected commits it affects.
struct BoundarySelection {
    boundary: BoundaryMove,
    affected_commits: IndexSet<CommitId>,
    rewrite_roots: IndexSet<CommitId>,
}

/// Builds and validates a sync plan.
struct GitSyncPlanBuilder<'repo> {
    repo: &'repo MutableRepo,
    selected: SelectedGraph,
    boundaries: Vec<BoundarySelection>,
    parent_replacements: IndexMap<CommitId, IndexMap<CommitId, CommitId>>,
}

impl<'repo> GitSyncPlanBuilder<'repo> {
    fn new(repo: &'repo MutableRepo, selected: SelectedGraph, moves: Vec<BoundaryMove>) -> Self {
        let boundaries = moves
            .into_iter()
            .map(|boundary| BoundarySelection {
                boundary,
                affected_commits: IndexSet::new(),
                rewrite_roots: IndexSet::new(),
            })
            .collect_vec();
        Self {
            repo,
            selected,
            boundaries,
            parent_replacements: IndexMap::new(),
        }
    }

    async fn build(mut self) -> Result<GitSyncPlan, GitSyncError> {
        self.classify_affected_commits_and_rewrite_roots().await?;
        self.validate_selected_coverage()?;
        self.validate_non_overlapping_rewrite_roles()?;
        self.finish()
    }

    /// Finds which selected commits need rebasing after each remote bookmark
    /// update.
    ///
    /// Commits between `old` and `new`, and commits already based on `new`, are
    /// skipped. A selected commit is rebased directly onto `new` if one of its
    /// parents lies between `old` and `new`; that parent is replaced with
    /// `new`.
    async fn classify_affected_commits_and_rewrite_roots(&mut self) -> Result<(), GitSyncError> {
        let index = self.repo.index();
        for selection in &mut self.boundaries {
            let boundary = &selection.boundary;
            for commit in self.selected.commits.values() {
                let id = commit.id();
                if !index.is_ancestor(&boundary.old, id).await? {
                    continue;
                }
                if index.is_ancestor(id, &boundary.new).await?
                    || index.is_ancestor(&boundary.new, id).await?
                {
                    continue;
                }
                selection.affected_commits.insert(id.clone());

                for parent in commit.parent_ids() {
                    if !index.is_ancestor(&boundary.old, parent).await?
                        || !index.is_ancestor(parent, &boundary.new).await?
                    {
                        continue;
                    }
                    selection.rewrite_roots.insert(id.clone());
                    let replacements = self.parent_replacements.entry(id.clone()).or_default();
                    if let Some(existing) = replacements.get(parent) {
                        if existing != &boundary.new {
                            let mut destinations = vec![existing.clone(), boundary.new.clone()];
                            destinations.sort_unstable();
                            destinations.dedup();
                            return Err(GitSyncError::ConflictingParentReplacement {
                                commit_id: id.clone(),
                                parent_id: parent.clone(),
                                destinations,
                            });
                        }
                    } else {
                        replacements.insert(parent.clone(), boundary.new.clone());
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_selected_coverage(&self) -> Result<(), GitSyncError> {
        // Check each move separately. Otherwise another move can make a path with an
        // unselected intermediate commit appear covered.
        for selection in &self.boundaries {
            let reachable = self.selected.descendant_closure(&selection.rewrite_roots);
            let uncovered = selection
                .affected_commits
                .difference(&reachable)
                .cloned()
                .collect_vec();
            if !uncovered.is_empty() {
                return Err(GitSyncError::SelectionBoundary {
                    old: selection.boundary.old.clone(),
                    new: selection.boundary.new.clone(),
                    commit_ids: uncovered,
                });
            }
        }
        Ok(())
    }

    fn validate_non_overlapping_rewrite_roles(&self) -> Result<(), GitSyncError> {
        // A commit with both roles would receive an explicit parent override for one
        // update instead of following the rewritten selected parent from another.
        // Without branch-ownership information, reject rather than detach or split
        // the selected chain.
        let all_rewrite_roots: IndexSet<_> = self
            .boundaries
            .iter()
            .flat_map(|selection| selection.rewrite_roots.iter().cloned())
            .collect();
        let mut commit_ids = Vec::new();
        for selection in &self.boundaries {
            let structural_ids = self.selected.descendant_closure(&selection.rewrite_roots);
            commit_ids.extend(
                structural_ids
                    .iter()
                    .filter(|id| {
                        !selection.rewrite_roots.contains(*id) && all_rewrite_roots.contains(*id)
                    })
                    .cloned(),
            );
        }
        commit_ids.sort_unstable();
        commit_ids.dedup();
        if commit_ids.is_empty() {
            Ok(())
        } else {
            Err(GitSyncError::ConflictingRewriteRoles { commit_ids })
        }
    }

    fn finish(self) -> Result<GitSyncPlan, GitSyncError> {
        let rewrite_root_ids: IndexSet<_> = self
            .boundaries
            .iter()
            .flat_map(|selection| selection.rewrite_roots.iter().cloned())
            .collect();
        let planned_ids = self.selected.descendant_closure(&rewrite_root_ids);
        // A destination cannot be rewritten because another move may use it as a
        // replacement parent.
        let mut moving_destinations = self
            .boundaries
            .into_iter()
            .map(|selection| selection.boundary.new)
            .filter(|id| planned_ids.contains(id))
            .collect_vec();
        moving_destinations.sort_unstable();
        moving_destinations.dedup();
        if !moving_destinations.is_empty() {
            return Err(GitSyncError::MovingDestination {
                commit_ids: moving_destinations,
            });
        }
        for (commit_id, replacements) in &self.parent_replacements {
            for (parent_id, destination) in replacements {
                if planned_ids.contains(parent_id) {
                    return Err(GitSyncError::OverlappingRewrite {
                        commit_id: commit_id.clone(),
                        parent_id: parent_id.clone(),
                        destination: destination.clone(),
                    });
                }
            }
        }

        let mut root_parent_overrides = HashMap::new();
        for (id, replacements) in self.parent_replacements {
            let commit = &self.selected.commits[&id];
            let new_parents = commit
                .parent_ids()
                .iter()
                .map(|parent| replacements.get(parent).unwrap_or(parent))
                .unique()
                .cloned()
                .collect_vec();
            root_parent_overrides.insert(id, new_parents);
        }
        let commits_to_rewrite = self
            .selected
            .commits
            .into_values()
            .filter(|commit| planned_ids.contains(commit.id()))
            .collect_vec();
        Ok(GitSyncPlan {
            commits_to_rewrite,
            root_parent_overrides,
        })
    }
}

/// The selected commits and a map from each parent to its selected children.
struct SelectedGraph {
    commits: IndexMap<CommitId, Commit>,
    children_by_parent: IndexMap<CommitId, Vec<CommitId>>,
}

impl SelectedGraph {
    async fn load(
        repo: &MutableRepo,
        mut selected_ids: Vec<CommitId>,
    ) -> Result<Self, BackendError> {
        selected_ids.sort_unstable();
        selected_ids.dedup();

        let store = repo.store();
        let commits = try_join_all(selected_ids.iter().map(|id| store.get_commit_async(id)))
            .await?
            .into_iter()
            .map(|commit| (commit.id().clone(), commit))
            .collect::<IndexMap<_, _>>();
        let mut children_by_parent = IndexMap::<CommitId, Vec<CommitId>>::new();
        for commit in commits.values() {
            for parent in commit.parent_ids() {
                if commits.contains_key(parent) {
                    children_by_parent
                        .entry(parent.clone())
                        .or_default()
                        .push(commit.id().clone());
                }
            }
        }
        for children in children_by_parent.values_mut() {
            children.sort_unstable();
            children.dedup();
        }
        Ok(Self {
            commits,
            children_by_parent,
        })
    }

    fn descendant_closure(&self, roots: &IndexSet<CommitId>) -> IndexSet<CommitId> {
        let mut reachable = roots.clone();
        let mut queue: VecDeque<_> = roots.iter().cloned().collect();
        while let Some(parent) = queue.pop_front() {
            for child in self.children_by_parent.get(&parent).into_iter().flatten() {
                if reachable.insert(child.clone()) {
                    queue.push_back(child.clone());
                }
            }
        }
        reachable
    }
}

async fn derive_boundary_moves(
    repo: &MutableRepo,
    selected: &SelectedGraph,
    import_stats: &GitImportStats,
) -> Result<Vec<BoundaryMove>, GitSyncError> {
    let mut moves = Vec::new();
    for (name, updates) in &import_stats
        .changed_remote_bookmarks
        .iter()
        // Git import records remote bookmark updates in symbol order, so equal
        // bookmark names are contiguous for this grouping.
        .chunk_by(|update| update.symbol.name.clone())
    {
        let tracked_updates = updates
            .filter(|update| {
                update.old_remote_ref.is_tracked()
                    || repo
                        .get_remote_bookmark(update.symbol.as_ref())
                        .is_tracked()
            })
            .collect_vec();
        if tracked_updates.is_empty()
            || !tracked_updates_are_relevant(repo, selected, &tracked_updates).await?
        {
            continue;
        }
        moves.extend(validate_bookmark_updates(repo, name, &tracked_updates).await?);
    }

    moves.sort_unstable();
    for (old, boundaries) in &moves.iter().chunk_by(|boundary| boundary.old.clone()) {
        let destinations = boundaries
            .map(|boundary| &boundary.new)
            .unique()
            .collect_vec();
        if destinations.len() > 1 {
            return Err(GitSyncError::AmbiguousBoundary {
                old,
                destinations: destinations.into_iter().cloned().collect_vec(),
            });
        }
    }
    moves.dedup();
    Ok(moves)
}

async fn tracked_updates_are_relevant(
    repo: &MutableRepo,
    selected: &SelectedGraph,
    updates: &[&GitImportRefUpdate],
) -> Result<bool, IndexError> {
    for update in updates {
        for old_id in update.old_remote_ref.target.added_ids() {
            for selected_id in selected.commits.keys() {
                if old_id != selected_id && repo.index().is_ancestor(old_id, selected_id).await? {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

async fn validate_bookmark_updates(
    repo: &MutableRepo,
    name: RefNameBuf,
    updates: &[&GitImportRefUpdate],
) -> Result<Vec<BoundaryMove>, GitSyncError> {
    let mut destinations = Vec::new();
    let mut moves = Vec::new();
    for update in updates {
        let Some(new) = update.new_target.as_normal() else {
            let reason = if update.new_target.is_absent() {
                GitSyncUnsupportedReason::NewTargetAbsent
            } else {
                GitSyncUnsupportedReason::NewTargetConflicted {
                    target: update.new_target.clone(),
                }
            };
            return Err(GitSyncError::UnsupportedRemoteChange {
                symbol: update.symbol.clone(),
                reason,
            });
        };
        destinations.push(new.clone());
        if update.old_remote_ref.target.is_absent() {
            continue;
        }
        let Some(old) = update.old_remote_ref.target.as_normal() else {
            return Err(GitSyncError::UnsupportedRemoteChange {
                symbol: update.symbol.clone(),
                reason: GitSyncUnsupportedReason::OldTargetConflicted {
                    target: update.old_remote_ref.target.clone(),
                },
            });
        };
        if !repo.index().is_ancestor(old, new).await? {
            return Err(GitSyncError::UnsupportedRemoteChange {
                symbol: update.symbol.clone(),
                reason: GitSyncUnsupportedReason::NonFastForward {
                    old: old.clone(),
                    new: new.clone(),
                },
            });
        }
        moves.push(BoundaryMove {
            old: old.clone(),
            new: new.clone(),
        });
    }
    destinations.sort_unstable();
    destinations.dedup();
    if destinations.len() > 1 {
        return Err(GitSyncError::AmbiguousBookmark { name, destinations });
    }
    Ok(moves)
}
