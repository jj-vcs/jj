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

use std::collections::HashMap;
use std::collections::HashSet;
use std::slice;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::future::try_join_all;
use futures::try_join;
use indexmap::IndexMap;
use indexmap::IndexSet;
use itertools::Itertools as _;
use pollster::FutureExt as _;
use tracing::instrument;

use crate::backend::BackendError;
use crate::backend::BackendResult;
use crate::backend::CommitId;
use crate::commit::Commit;
use crate::commit::CommitIteratorExt as _;
use crate::commit_builder::CommitBuilder;
use crate::index::Index;
use crate::index::IndexResult;
use crate::iter_util::fallible_any;
use crate::matchers::Matcher;
use crate::matchers::Visit;
use crate::merge::Merge;
use crate::merged_tree::MergedTree;
use crate::merged_tree::MergedTreeBuilder;
use crate::merged_tree::TreeDiffEntry;
use crate::repo::MutableRepo;
use crate::repo::Repo;
use crate::repo_path::RepoPath;
use crate::revset::RevsetExpression;
use crate::revset::RevsetIteratorExt as _;
use crate::store::Store;

/// Merges `commits` and tries to resolve any conflicts recursively.
#[instrument(skip(repo))]
pub async fn merge_commit_trees(repo: &dyn Repo, commits: &[Commit]) -> BackendResult<MergedTree> {
    if let [commit] = commits {
        Ok(commit.tree())
    } else {
        merge_commit_trees_no_resolve_without_repo(repo.store(), repo.index(), commits)
            .await?
            .resolve()
            .await
    }
}

/// Merges `commits` without attempting to resolve file conflicts.
#[instrument(skip(index))]
pub async fn merge_commit_trees_no_resolve_without_repo(
    store: &Arc<Store>,
    index: &dyn Index,
    commits: &[Commit],
) -> BackendResult<MergedTree> {
    let commit_ids = commits
        .iter()
        .map(|commit| commit.id().clone())
        .collect_vec();
    let commit_id_merge = find_recursive_merge_commits(store, index, commit_ids)?;
    let tree_id_merge = commit_id_merge
        .try_map_async(async |commit_id| {
            let commit = store.get_commit_async(commit_id).await?;
            Ok::<_, BackendError>(commit.tree_ids().clone())
        })
        .await?;
    Ok(MergedTree::new(
        store.clone(),
        tree_id_merge.flatten().simplify(),
    ))
}

/// Find the commits to use as input to the recursive merge algorithm.
pub fn find_recursive_merge_commits(
    store: &Arc<Store>,
    index: &dyn Index,
    mut commit_ids: Vec<CommitId>,
) -> BackendResult<Merge<CommitId>> {
    if commit_ids.is_empty() {
        Ok(Merge::resolved(store.root_commit_id().clone()))
    } else if commit_ids.len() == 1 {
        Ok(Merge::resolved(commit_ids.pop().unwrap()))
    } else {
        let mut result = Merge::resolved(commit_ids[0].clone());
        for (i, other_commit_id) in commit_ids.iter().enumerate().skip(1) {
            let ancestor_ids = index
                .common_ancestors(&commit_ids[0..i], &commit_ids[i..][..1])
                // TODO: indexing error shouldn't be a "BackendError"
                .map_err(|err| BackendError::Other(err.into()))?;
            let ancestor_merge = find_recursive_merge_commits(store, index, ancestor_ids)?;
            result = Merge::from_vec(vec![
                result,
                ancestor_merge,
                Merge::resolved(other_commit_id.clone()),
            ])
            .flatten();
        }
        Ok(result)
    }
}

/// Restore matching paths from the source into the destination.
pub async fn restore_tree(
    source: &MergedTree,
    destination: &MergedTree,
    matcher: &dyn Matcher,
) -> BackendResult<MergedTree> {
    if matcher.visit(RepoPath::root()) == Visit::AllRecursively {
        // Optimization for a common case
        Ok(source.clone())
    } else {
        // TODO: We should be able to not traverse deeper in the diff if the matcher
        // matches an entire subtree.
        let mut tree_builder = MergedTreeBuilder::new(destination.clone());
        // TODO: handle copy tracking
        let mut diff_stream = source.diff_stream(destination, matcher);
        while let Some(TreeDiffEntry {
            path: repo_path,
            values,
        }) = diff_stream.next().await
        {
            let source_value = values?.before;
            tree_builder.set_or_remove(repo_path, source_value);
        }
        tree_builder.write_tree()
    }
}

pub async fn rebase_commit(
    mut_repo: &mut MutableRepo,
    old_commit: Commit,
    new_parents: Vec<CommitId>,
) -> BackendResult<Commit> {
    let rewriter = CommitRewriter::new(mut_repo, old_commit, new_parents);
    let builder = rewriter.rebase().await?;
    builder.write()
}

/// Helps rewrite a commit.
pub struct CommitRewriter<'repo> {
    mut_repo: &'repo mut MutableRepo,
    old_commit: Commit,
    new_parents: Vec<CommitId>,
}

impl<'repo> CommitRewriter<'repo> {
    /// Create a new instance.
    pub fn new(
        mut_repo: &'repo mut MutableRepo,
        old_commit: Commit,
        new_parents: Vec<CommitId>,
    ) -> Self {
        Self {
            mut_repo,
            old_commit,
            new_parents,
        }
    }

    /// Returns the `MutableRepo`.
    pub fn repo_mut(&mut self) -> &mut MutableRepo {
        self.mut_repo
    }

    /// The commit we're rewriting.
    pub fn old_commit(&self) -> &Commit {
        &self.old_commit
    }

    /// Get the old commit's intended new parents.
    pub fn new_parents(&self) -> &[CommitId] {
        &self.new_parents
    }

    /// Set the old commit's intended new parents.
    pub fn set_new_parents(&mut self, new_parents: Vec<CommitId>) {
        self.new_parents = new_parents;
    }

    /// Set the old commit's intended new parents to be the rewritten versions
    /// of the given parents.
    pub fn set_new_rewritten_parents(&mut self, unrewritten_parents: &[CommitId]) {
        self.new_parents = self.mut_repo.new_parents(unrewritten_parents);
    }

    /// Update the intended new parents by replacing any occurrence of
    /// `old_parent` by `new_parents`.
    pub fn replace_parent<'a>(
        &mut self,
        old_parent: &CommitId,
        new_parents: impl IntoIterator<Item = &'a CommitId>,
    ) {
        if let Some(i) = self.new_parents.iter().position(|p| p == old_parent) {
            self.new_parents
                .splice(i..i + 1, new_parents.into_iter().cloned());
            let mut unique = HashSet::new();
            self.new_parents.retain(|p| unique.insert(p.clone()));
        }
    }

    /// Checks if the intended new parents are different from the old commit's
    /// parents.
    pub fn parents_changed(&self) -> bool {
        self.new_parents != self.old_commit.parent_ids()
    }

    /// If a merge commit would end up with one parent being an ancestor of the
    /// other, then filter out the ancestor.
    pub fn simplify_ancestor_merge(&mut self) -> IndexResult<()> {
        let head_set: HashSet<_> = self
            .mut_repo
            .index()
            .heads(&mut self.new_parents.iter())?
            .into_iter()
            .collect();
        self.new_parents.retain(|parent| head_set.contains(parent));
        Ok(())
    }

    /// Records the old commit as abandoned with the new parents.
    ///
    /// This is equivalent to `reparent(settings).abandon()`, but is cheaper.
    pub fn abandon(self) {
        let old_commit_id = self.old_commit.id().clone();
        let new_parents = self.new_parents;
        self.mut_repo
            .record_abandoned_commit_with_parents(old_commit_id, new_parents);
    }

    /// Rebase the old commit onto the new parents. Returns a `CommitBuilder`
    /// for the new commit. Returns `None` if the commit was abandoned.
    pub async fn rebase_with_empty_behavior(
        self,
        empty: EmptyBehavior,
    ) -> BackendResult<Option<CommitBuilder<'repo>>> {
        let old_parents_fut = self.old_commit.parents_async();
        let new_parents_fut = try_join_all(
            self.new_parents
                .iter()
                .map(|new_parent_id| self.mut_repo.store().get_commit_async(new_parent_id)),
        );
        let (old_parents, new_parents) = try_join!(old_parents_fut, new_parents_fut)?;
        let old_parent_trees = old_parents
            .iter()
            .map(|parent| parent.tree_ids().clone())
            .collect_vec();
        let new_parent_trees = new_parents
            .iter()
            .map(|parent| parent.tree_ids().clone())
            .collect_vec();

        let (was_empty, new_tree) = if new_parent_trees == old_parent_trees {
            (
                // Optimization: was_empty is only used for newly empty, but when the
                // parents haven't changed it can't be newly empty.
                true,
                // Optimization: Skip merging.
                self.old_commit.tree(),
            )
        } else {
            // We wouldn't need to resolve merge conflicts here if the
            // same-change rule is "keep". See 9d4a97381f30 "rewrite: don't
            // resolve intermediate parent tree when rebasing" for details.
            let old_base_tree_fut = merge_commit_trees(self.mut_repo, &old_parents);
            let new_base_tree_fut = merge_commit_trees(self.mut_repo, &new_parents);
            let old_tree = self.old_commit.tree();
            let (old_base_tree, new_base_tree) = try_join!(old_base_tree_fut, new_base_tree_fut)?;
            (
                old_base_tree.tree_ids() == self.old_commit.tree_ids(),
                new_base_tree.merge(old_base_tree, old_tree).await?,
            )
        };
        // Ensure we don't abandon commits with multiple parents (merge commits), even
        // if they're empty.
        if let [parent] = &new_parents[..] {
            let should_abandon = match empty {
                EmptyBehavior::Keep => false,
                EmptyBehavior::AbandonNewlyEmpty => {
                    parent.tree_ids() == new_tree.tree_ids() && !was_empty
                }
                EmptyBehavior::AbandonAllEmpty => parent.tree_ids() == new_tree.tree_ids(),
            };
            if should_abandon {
                self.abandon();
                return Ok(None);
            }
        }

        let builder = self
            .mut_repo
            .rewrite_commit(&self.old_commit)
            .set_parents(self.new_parents)
            .set_tree(new_tree);
        Ok(Some(builder))
    }

    /// Rebase the old commit onto the new parents. Returns a `CommitBuilder`
    /// for the new commit.
    pub async fn rebase(self) -> BackendResult<CommitBuilder<'repo>> {
        let builder = self.rebase_with_empty_behavior(EmptyBehavior::Keep).await?;
        Ok(builder.unwrap())
    }

    /// Rewrite the old commit onto the new parents without changing its
    /// contents. Returns a `CommitBuilder` for the new commit.
    pub fn reparent(self) -> CommitBuilder<'repo> {
        self.mut_repo
            .rewrite_commit(&self.old_commit)
            .set_parents(self.new_parents)
    }
}

pub enum RebasedCommit {
    Rewritten(Commit),
    Abandoned { parent_id: CommitId },
}

pub fn rebase_commit_with_options(
    mut rewriter: CommitRewriter<'_>,
    options: &RebaseOptions,
) -> BackendResult<RebasedCommit> {
    // If specified, don't create commit where one parent is an ancestor of another.
    if options.simplify_ancestor_merge {
        rewriter
            .simplify_ancestor_merge()
            // TODO: indexing error shouldn't be a "BackendError"
            .map_err(|err| BackendError::Other(err.into()))?;
    }

    let single_parent = match &rewriter.new_parents[..] {
        [parent_id] => Some(parent_id.clone()),
        _ => None,
    };
    let new_parents_len = rewriter.new_parents.len();
    if let Some(builder) = rewriter
        .rebase_with_empty_behavior(options.empty)
        .block_on()?
    {
        let new_commit = builder.write()?;
        Ok(RebasedCommit::Rewritten(new_commit))
    } else {
        assert_eq!(new_parents_len, 1);
        Ok(RebasedCommit::Abandoned {
            parent_id: single_parent.unwrap(),
        })
    }
}

/// Moves changes from `sources` to the `destination` parent, returns new tree.
pub fn rebase_to_dest_parent(
    repo: &dyn Repo,
    sources: &[Commit],
    destination: &Commit,
) -> BackendResult<MergedTree> {
    if let [source] = sources
        && source.parent_ids() == destination.parent_ids()
    {
        return Ok(source.tree());
    }
    sources.iter().try_fold(
        destination.parent_tree(repo)?,
        |destination_tree, source| {
            let source_parent_tree = source.parent_tree(repo)?;
            let source_tree = source.tree();
            destination_tree
                .merge(source_parent_tree, source_tree)
                .block_on()
        },
    )
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum EmptyBehavior {
    /// Always keep empty commits
    #[default]
    Keep,
    /// Skips commits that would be empty after the rebase, but that were not
    /// originally empty.
    /// Will never skip merge commits with multiple non-empty parents.
    AbandonNewlyEmpty,
    /// Skips all empty commits, including ones that were empty before the
    /// rebase.
    /// Will never skip merge commits with multiple non-empty parents.
    AbandonAllEmpty,
}

/// Controls the configuration of a rebase.
// If we wanted to add a flag similar to `git rebase --ignore-date`, then this
// makes it much easier by ensuring that the only changes required are to
// change the RebaseOptions construction in the CLI, and changing the
// rebase_commit function to actually use the flag, and ensure we don't need to
// plumb it in.
#[derive(Clone, Debug, Default)]
pub struct RebaseOptions {
    pub empty: EmptyBehavior,
    pub rewrite_refs: RewriteRefsOptions,
    /// If a merge commit would end up with one parent being an ancestor of the
    /// other, then filter out the ancestor.
    pub simplify_ancestor_merge: bool,
}

/// Configuration for [`MutableRepo::update_rewritten_references()`].
#[derive(Clone, Debug, Default)]
pub struct RewriteRefsOptions {
    /// Whether or not delete bookmarks pointing to the abandoned commits.
    ///
    /// If false, bookmarks will be moved to the parents of the abandoned
    /// commit.
    pub delete_abandoned_bookmarks: bool,
}

pub struct MoveCommitsStats {
    /// The number of commits in the target set which were rebased.
    pub num_rebased_targets: u32,
    /// The number of descendant commits which were rebased.
    pub num_rebased_descendants: u32,
    /// The number of commits for which rebase was skipped, due to the commit
    /// already being in place.
    pub num_skipped_rebases: u32,
    /// The number of commits which were abandoned due to being empty.
    pub num_abandoned_empty: u32,
    /// The rebased commits
    pub rebased_commits: HashMap<CommitId, RebasedCommit>,
}

/// Target and destination commits to be rebased by [`move_commits()`].
#[derive(Clone, Debug)]
pub struct MoveCommitsLocation {
    pub new_parent_ids: Vec<CommitId>,
    pub new_child_ids: Vec<CommitId>,
    pub target: MoveCommitsTarget,
}

#[derive(Clone, Debug)]
pub enum MoveCommitsTarget {
    /// The commits to be moved. Commits should be mutable and in reverse
    /// topological order.
    Commits(Vec<CommitId>),
    /// The root commits to be moved, along with all their descendants.
    Roots(Vec<CommitId>),
}

#[derive(Clone, Debug)]
pub struct ComputedMoveCommits {
    target_commit_ids: IndexSet<CommitId>,
    descendants: Vec<Commit>,
    commit_new_parents_map: HashMap<CommitId, Vec<CommitId>>,
    to_abandon: HashSet<CommitId>,
}

impl ComputedMoveCommits {
    fn empty() -> Self {
        Self {
            target_commit_ids: IndexSet::new(),
            descendants: vec![],
            commit_new_parents_map: HashMap::new(),
            to_abandon: HashSet::new(),
        }
    }

    /// Records a set of commits to abandon while rebasing.
    ///
    /// Abandoning these commits while rebasing ensures that their descendants
    /// are still rebased properly. [`MutableRepo::record_abandoned_commit`] is
    /// similar, but it can lead to issues when abandoning a target commit
    /// before the rebase.
    pub fn record_to_abandon(&mut self, commit_ids: impl IntoIterator<Item = CommitId>) {
        self.to_abandon.extend(commit_ids);
    }

    pub fn apply(
        self,
        mut_repo: &mut MutableRepo,
        options: &RebaseOptions,
    ) -> BackendResult<MoveCommitsStats> {
        apply_move_commits(mut_repo, self, options)
    }
}

/// Moves `loc.target` commits from their current location to a new location in
/// the graph.
///
/// Commits in `target` are rebased onto the new parents given by
/// `new_parent_ids`, while the `new_child_ids` commits are rebased onto the
/// heads of the commits in `targets`. This assumes that commits in `target` and
/// `new_child_ids` can be rewritten, and there will be no cycles in the
/// resulting graph. Commits in `target` should be in reverse topological order.
pub fn move_commits(
    mut_repo: &mut MutableRepo,
    loc: &MoveCommitsLocation,
    options: &RebaseOptions,
) -> BackendResult<MoveCommitsStats> {
    compute_move_commits(mut_repo, loc)?.apply(mut_repo, options)
}

pub fn compute_move_commits(
    repo: &MutableRepo,
    loc: &MoveCommitsLocation,
) -> BackendResult<ComputedMoveCommits> {
    let target_commit_ids: IndexSet<CommitId>;
    let connected_target_commits: Vec<Commit>;
    let connected_target_commits_internal_parents: HashMap<CommitId, IndexSet<CommitId>>;
    let target_roots: HashSet<CommitId>;

    match &loc.target {
        MoveCommitsTarget::Commits(commit_ids) => {
            if commit_ids.is_empty() {
                return Ok(ComputedMoveCommits::empty());
            }

            target_commit_ids = commit_ids.iter().cloned().collect();

            connected_target_commits = RevsetExpression::commits(commit_ids.clone())
                .connected()
                .evaluate(repo)
                .map_err(|err| err.into_backend_error())?
                .iter()
                .commits(repo.store())
                .try_collect()
                .map_err(|err| err.into_backend_error())?;
            connected_target_commits_internal_parents =
                compute_internal_parents_within(&target_commit_ids, &connected_target_commits);

            target_roots = connected_target_commits_internal_parents
                .iter()
                .filter(|&(commit_id, parents)| {
                    target_commit_ids.contains(commit_id) && parents.is_empty()
                })
                .map(|(commit_id, _)| commit_id.clone())
                .collect();
        }
        MoveCommitsTarget::Roots(root_ids) => {
            if root_ids.is_empty() {
                return Ok(ComputedMoveCommits::empty());
            }

            target_commit_ids = RevsetExpression::commits(root_ids.clone())
                .descendants()
                .evaluate(repo)
                .map_err(|err| err.into_backend_error())?
                .iter()
                .try_collect()
                .map_err(|err| err.into_backend_error())?;

            connected_target_commits = target_commit_ids
                .iter()
                .map(|id| repo.store().get_commit(id))
                .try_collect()?;
            // We don't have to compute the internal parents for the connected target set,
            // since the connected target set is the same as the target set.
            connected_target_commits_internal_parents = HashMap::new();
            target_roots = root_ids.iter().cloned().collect();
        }
    }

    // If a commit outside the target set has a commit in the target set as a
    // parent, then - after the transformation - it should have that commit's
    // ancestors which are not in the target set as parents.
    let mut target_commits_external_parents: HashMap<CommitId, IndexSet<CommitId>> = HashMap::new();
    for id in target_commit_ids.iter().rev() {
        let commit = repo.store().get_commit(id)?;
        let mut new_parents = IndexSet::new();
        for old_parent in commit.parent_ids() {
            if let Some(parents) = target_commits_external_parents.get(old_parent) {
                new_parents.extend(parents.iter().cloned());
            } else {
                new_parents.insert(old_parent.clone());
            }
        }
        target_commits_external_parents.insert(commit.id().clone(), new_parents);
    }

    // If the new parents include a commit in the target set, replace it with the
    // commit's ancestors which are outside the set.
    // e.g. `jj rebase -r A --before A`
    let new_parent_ids: Vec<_> = loc
        .new_parent_ids
        .iter()
        .flat_map(|parent_id| {
            if let Some(parent_ids) = target_commits_external_parents.get(parent_id) {
                parent_ids.iter().cloned().collect_vec()
            } else {
                vec![parent_id.clone()]
            }
        })
        .collect();

    // If the new children include a commit in the target set, replace it with the
    // commit's descendants which are outside the set.
    // e.g. `jj rebase -r A --after A`
    let new_children: Vec<_> = if loc
        .new_child_ids
        .iter()
        .any(|id| target_commit_ids.contains(id))
    {
        let target_commits_descendants: Vec<_> =
            RevsetExpression::commits(target_commit_ids.iter().cloned().collect_vec())
                .union(
                    &RevsetExpression::commits(target_commit_ids.iter().cloned().collect_vec())
                        .children(),
                )
                .evaluate(repo)
                .map_err(|err| err.into_backend_error())?
                .iter()
                .commits(repo.store())
                .try_collect()
                .map_err(|err| err.into_backend_error())?;

        // For all commits in the target set, compute its transitive descendant commits
        // which are outside of the target set by up to 1 generation.
        let mut target_commit_external_descendants: HashMap<CommitId, IndexSet<Commit>> =
            HashMap::new();
        // Iterate through all descendants of the target set, going through children
        // before parents.
        for commit in &target_commits_descendants {
            if !target_commit_external_descendants.contains_key(commit.id()) {
                let children = if target_commit_ids.contains(commit.id()) {
                    IndexSet::new()
                } else {
                    IndexSet::from([commit.clone()])
                };
                target_commit_external_descendants.insert(commit.id().clone(), children);
            }

            let children = target_commit_external_descendants
                .get(commit.id())
                .unwrap()
                .iter()
                .cloned()
                .collect_vec();
            for parent_id in commit.parent_ids() {
                if target_commit_ids.contains(parent_id) {
                    if let Some(target_children) =
                        target_commit_external_descendants.get_mut(parent_id)
                    {
                        target_children.extend(children.iter().cloned());
                    } else {
                        target_commit_external_descendants
                            .insert(parent_id.clone(), children.iter().cloned().collect());
                    }
                };
            }
        }

        let mut new_children = Vec::new();
        for id in &loc.new_child_ids {
            if let Some(children) = target_commit_external_descendants.get(id) {
                new_children.extend(children.iter().cloned());
            } else {
                new_children.push(repo.store().get_commit(id)?);
            }
        }
        new_children
    } else {
        loc.new_child_ids
            .iter()
            .map(|id| repo.store().get_commit(id))
            .try_collect()?
    };

    // Compute the parents of the new children, which will include the heads of the
    // target set.
    let new_children_parents: HashMap<_, _> = if !new_children.is_empty() {
        // Compute the heads of the target set, which will be used as the parents of
        // `new_children`.
        let target_heads = compute_commits_heads(&target_commit_ids, &connected_target_commits);

        new_children
            .iter()
            .map(|child_commit| {
                let mut new_child_parent_ids = IndexSet::new();
                for old_child_parent_id in child_commit.parent_ids() {
                    // Replace target commits with their parents outside the target set.
                    let old_child_parent_ids = if let Some(parents) =
                        target_commits_external_parents.get(old_child_parent_id)
                    {
                        parents.iter().collect_vec()
                    } else {
                        vec![old_child_parent_id]
                    };

                    // If the original parents of the new children are the new parents of the
                    // `target_heads`, replace them with the target heads since we are "inserting"
                    // the target commits in between the new parents and the new children.
                    for id in old_child_parent_ids {
                        if new_parent_ids.contains(id) {
                            new_child_parent_ids.extend(target_heads.clone());
                        } else {
                            new_child_parent_ids.insert(id.clone());
                        };
                    }
                }

                // If not already present, add `target_heads` as parents of the new child
                // commit.
                new_child_parent_ids.extend(target_heads.clone());

                (
                    child_commit.id().clone(),
                    new_child_parent_ids.into_iter().collect_vec(),
                )
            })
            .collect()
    } else {
        HashMap::new()
    };

    // Compute the set of commits to visit, which includes the target commits, the
    // new children commits (if any), and their descendants.
    let mut roots = target_roots.iter().cloned().collect_vec();
    roots.extend(new_children.iter().ids().cloned());

    let descendants = repo.find_descendants_for_rebase(roots.clone())?;
    let commit_new_parents_map = descendants
        .iter()
        .map(|commit| -> BackendResult<_> {
            let commit_id = commit.id();
            let new_parent_ids =
                if let Some(new_child_parents) = new_children_parents.get(commit_id) {
                    // New child of the rebased target commits.
                    new_child_parents.clone()
                } else if target_commit_ids.contains(commit_id) {
                    // Commit is in the target set.
                    if target_roots.contains(commit_id) {
                        // If the commit is a root of the target set, it should be rebased onto the
                        // new destination.
                        new_parent_ids.clone()
                    } else {
                        // Otherwise:
                        // 1. Keep parents which are within the target set.
                        // 2. Replace parents which are outside the target set but are part of the
                        //    connected target set with their ancestor commits which are in the
                        //    target set.
                        // 3. Keep other parents outside the target set if they are not descendants
                        //    of the new children of the target set.
                        let mut new_parents = vec![];
                        for parent_id in commit.parent_ids() {
                            if target_commit_ids.contains(parent_id) {
                                new_parents.push(parent_id.clone());
                            } else if let Some(parents) =
                                connected_target_commits_internal_parents.get(parent_id)
                            {
                                new_parents.extend(parents.iter().cloned());
                            } else if !fallible_any(&new_children, |child| {
                                repo.index().is_ancestor(child.id(), parent_id)
                            })
                            // TODO: indexing error shouldn't be a "BackendError"
                            .map_err(|err| BackendError::Other(err.into()))?
                            {
                                new_parents.push(parent_id.clone());
                            }
                        }
                        new_parents
                    }
                } else if commit
                    .parent_ids()
                    .iter()
                    .any(|id| target_commits_external_parents.contains_key(id))
                {
                    // Commits outside the target set should have references to commits inside the
                    // set replaced.
                    let mut new_parents = vec![];
                    for parent in commit.parent_ids() {
                        if let Some(parents) = target_commits_external_parents.get(parent) {
                            new_parents.extend(parents.iter().cloned());
                        } else {
                            new_parents.push(parent.clone());
                        }
                    }
                    new_parents
                } else {
                    commit.parent_ids().iter().cloned().collect_vec()
                };
            Ok((commit.id().clone(), new_parent_ids))
        })
        .try_collect()?;

    Ok(ComputedMoveCommits {
        target_commit_ids,
        descendants,
        commit_new_parents_map,
        to_abandon: HashSet::new(),
    })
}

fn apply_move_commits(
    mut_repo: &mut MutableRepo,
    commits: ComputedMoveCommits,
    options: &RebaseOptions,
) -> BackendResult<MoveCommitsStats> {
    let mut num_rebased_targets = 0;
    let mut num_rebased_descendants = 0;
    let mut num_skipped_rebases = 0;
    let mut num_abandoned_empty = 0;

    // Always keep empty commits when rebasing descendants.
    let rebase_descendant_options = &RebaseOptions {
        empty: EmptyBehavior::Keep,
        rewrite_refs: options.rewrite_refs.clone(),
        simplify_ancestor_merge: options.simplify_ancestor_merge,
    };

    let mut rebased_commits: HashMap<CommitId, RebasedCommit> = HashMap::new();
    mut_repo.transform_commits(
        commits.descendants,
        &commits.commit_new_parents_map,
        &options.rewrite_refs,
        async |rewriter| {
            let old_commit_id = rewriter.old_commit().id().clone();
            if commits.to_abandon.contains(&old_commit_id) {
                rewriter.abandon();
            } else if rewriter.parents_changed() {
                let is_target_commit = commits.target_commit_ids.contains(&old_commit_id);
                let rebased_commit = rebase_commit_with_options(
                    rewriter,
                    if is_target_commit {
                        options
                    } else {
                        rebase_descendant_options
                    },
                )?;
                if let RebasedCommit::Abandoned { .. } = rebased_commit {
                    num_abandoned_empty += 1;
                } else if is_target_commit {
                    num_rebased_targets += 1;
                } else {
                    num_rebased_descendants += 1;
                }
                rebased_commits.insert(old_commit_id, rebased_commit);
            } else {
                num_skipped_rebases += 1;
            }

            Ok(())
        },
    )?;

    Ok(MoveCommitsStats {
        num_rebased_targets,
        num_rebased_descendants,
        num_skipped_rebases,
        num_abandoned_empty,
        rebased_commits,
    })
}

#[derive(Default)]
pub struct DuplicateCommitsStats {
    /// Map of original commit ID to newly duplicated commit.
    pub duplicated_commits: IndexMap<CommitId, Commit>,
    /// The number of descendant commits which were rebased onto the duplicated
    /// commits.
    pub num_rebased: u32,
}

/// Duplicates the given `target_commit_ids` onto a new location in the graph.
///
/// The roots of `target_commit_ids` are duplicated on top of the new
/// `parent_commit_ids`, whilst other commits in `target_commit_ids` are
/// duplicated on top of the newly duplicated commits in the target set. If
/// `children_commit_ids` is not empty, the `children_commit_ids` will be
/// rebased onto the heads of the duplicated target commits.
///
/// If `target_descriptions` is not empty, it will be consulted to retrieve the
/// new descriptions of the target commits, falling back to the original if the
/// map does not contain an entry for a given commit.
///
/// This assumes that commits in `children_commit_ids` can be rewritten. There
/// should also be no cycles in the resulting graph, i.e. `children_commit_ids`
/// should not be ancestors of `parent_commit_ids`. Commits in
/// `target_commit_ids` should be in reverse topological order (children before
/// parents).
pub async fn duplicate_commits(
    mut_repo: &mut MutableRepo,
    target_commit_ids: &[CommitId],
    target_descriptions: &HashMap<CommitId, String>,
    parent_commit_ids: &[CommitId],
    children_commit_ids: &[CommitId],
) -> BackendResult<DuplicateCommitsStats> {
    if target_commit_ids.is_empty() {
        return Ok(DuplicateCommitsStats::default());
    }

    let mut duplicated_old_to_new: IndexMap<CommitId, Commit> = IndexMap::new();
    let mut num_rebased = 0;

    let target_commit_ids: IndexSet<_> = target_commit_ids.iter().cloned().collect();

    let connected_target_commits: Vec<_> =
        RevsetExpression::commits(target_commit_ids.iter().cloned().collect_vec())
            .connected()
            .evaluate(mut_repo)
            .map_err(|err| err.into_backend_error())?
            .iter()
            .commits(mut_repo.store())
            .try_collect()
            .map_err(|err| err.into_backend_error())?;

    // Commits in the target set should only have other commits in the set as
    // parents, except the roots of the set, which persist their original
    // parents.
    // If a commit in the target set has a parent which is not in the set, but has
    // an ancestor which is in the set, then the commit will have that ancestor
    // as a parent instead.
    let target_commits_internal_parents = {
        let mut target_commits_internal_parents =
            compute_internal_parents_within(&target_commit_ids, &connected_target_commits);
        target_commits_internal_parents.retain(|id, _| target_commit_ids.contains(id));
        target_commits_internal_parents
    };

    // Compute the roots of `target_commits`.
    let target_root_ids: HashSet<_> = target_commits_internal_parents
        .iter()
        .filter(|(_, parents)| parents.is_empty())
        .map(|(commit_id, _)| commit_id.clone())
        .collect();

    // Compute the heads of the target set, which will be used as the parents of
    // the children commits.
    let target_head_ids = if !children_commit_ids.is_empty() {
        compute_commits_heads(&target_commit_ids, &connected_target_commits)
    } else {
        vec![]
    };

    // Topological order ensures that any parents of the original commit are
    // either not in `target_commits` or were already duplicated.
    for original_commit_id in target_commit_ids.iter().rev() {
        let original_commit = mut_repo
            .store()
            .get_commit_async(original_commit_id)
            .await?;
        let new_parent_ids = if target_root_ids.contains(original_commit_id) {
            parent_commit_ids.to_vec()
        } else {
            target_commits_internal_parents
                .get(original_commit_id)
                .unwrap()
                .iter()
                // Replace parent IDs with their new IDs if they were duplicated.
                .map(|id| {
                    duplicated_old_to_new
                        .get(id)
                        .map_or(id, |commit| commit.id())
                        .clone()
                })
                .collect()
        };
        let mut new_commit_builder = CommitRewriter::new(mut_repo, original_commit, new_parent_ids)
            .rebase()
            .await?
            .clear_rewrite_source()
            .generate_new_change_id();
        if let Some(desc) = target_descriptions.get(original_commit_id) {
            new_commit_builder = new_commit_builder.set_description(desc);
        }
        duplicated_old_to_new.insert(original_commit_id.clone(), new_commit_builder.write()?);
    }

    // Replace the original commit IDs in `target_head_ids` with the duplicated
    // commit IDs.
    let target_head_ids = target_head_ids
        .into_iter()
        .map(|commit_id| {
            duplicated_old_to_new
                .get(&commit_id)
                .map_or(commit_id, |commit| commit.id().clone())
        })
        .collect_vec();

    // Rebase new children onto the target heads.
    let children_commit_ids_set: HashSet<CommitId> = children_commit_ids.iter().cloned().collect();
    mut_repo.transform_descendants(children_commit_ids.to_vec(), async |mut rewriter| {
        if children_commit_ids_set.contains(rewriter.old_commit().id()) {
            let mut child_new_parent_ids = IndexSet::new();
            for old_parent_id in rewriter.old_commit().parent_ids() {
                // If the original parents of the new children are the new parents of
                // `target_head_ids`, replace them with `target_head_ids` since we are
                // "inserting" the target commits in between the new parents and the new
                // children.
                if parent_commit_ids.contains(old_parent_id) {
                    child_new_parent_ids.extend(target_head_ids.clone());
                } else {
                    child_new_parent_ids.insert(old_parent_id.clone());
                }
            }
            // If not already present, add `target_head_ids` as parents of the new child
            // commit.
            child_new_parent_ids.extend(target_head_ids.clone());
            rewriter.set_new_parents(child_new_parent_ids.into_iter().collect());
        }
        num_rebased += 1;
        rewriter.rebase().await?.write()?;
        Ok(())
    })?;

    Ok(DuplicateCommitsStats {
        duplicated_commits: duplicated_old_to_new,
        num_rebased,
    })
}

/// Duplicates the given `target_commits` onto their original parents or other
/// duplicated commits.
///
/// Commits in `target_commits` should be in reverse topological order (children
/// before parents).
///
/// If `target_descriptions` is not empty, it will be consulted to retrieve the
/// new descriptions of the target commits, falling back to the original if
/// the map does not contain an entry for a given commit.
pub fn duplicate_commits_onto_parents(
    mut_repo: &mut MutableRepo,
    target_commits: &[CommitId],
    target_descriptions: &HashMap<CommitId, String>,
) -> BackendResult<DuplicateCommitsStats> {
    if target_commits.is_empty() {
        return Ok(DuplicateCommitsStats::default());
    }

    let mut duplicated_old_to_new: IndexMap<CommitId, Commit> = IndexMap::new();

    // Topological order ensures that any parents of the original commit are
    // either not in `target_commits` or were already duplicated.
    for original_commit_id in target_commits.iter().rev() {
        let original_commit = mut_repo.store().get_commit(original_commit_id)?;
        let new_parent_ids = original_commit
            .parent_ids()
            .iter()
            .map(|id| {
                duplicated_old_to_new
                    .get(id)
                    .map_or(id, |commit| commit.id())
                    .clone()
            })
            .collect();
        let mut new_commit_builder = mut_repo
            .rewrite_commit(&original_commit)
            .clear_rewrite_source()
            .generate_new_change_id()
            .set_parents(new_parent_ids);
        if let Some(desc) = target_descriptions.get(original_commit_id) {
            new_commit_builder = new_commit_builder.set_description(desc);
        }
        duplicated_old_to_new.insert(original_commit_id.clone(), new_commit_builder.write()?);
    }

    Ok(DuplicateCommitsStats {
        duplicated_commits: duplicated_old_to_new,
        num_rebased: 0,
    })
}

/// Computes the internal parents of all commits in a connected commit graph,
/// allowing only commits in the target set as parents.
///
/// The parents of each commit are identical to the ones found using a preorder
/// DFS of the node's ancestors, starting from the node itself, and avoiding
/// traversing an edge if the parent is in the target set. `graph_commits`
/// should be in reverse topological order.
fn compute_internal_parents_within(
    target_commit_ids: &IndexSet<CommitId>,
    graph_commits: &[Commit],
) -> HashMap<CommitId, IndexSet<CommitId>> {
    let mut internal_parents: HashMap<CommitId, IndexSet<CommitId>> = HashMap::new();
    for commit in graph_commits.iter().rev() {
        // The roots of the set will not have any parents found in `internal_parents`,
        // and will be stored as an empty vector.
        let mut new_parents = IndexSet::new();
        for old_parent in commit.parent_ids() {
            if target_commit_ids.contains(old_parent) {
                new_parents.insert(old_parent.clone());
            } else if let Some(parents) = internal_parents.get(old_parent) {
                new_parents.extend(parents.iter().cloned());
            }
        }
        internal_parents.insert(commit.id().clone(), new_parents);
    }
    internal_parents
}

/// Computes the heads of commits in the target set, given the list of
/// `target_commit_ids` and a connected graph of commits.
///
/// `connected_target_commits` should be in reverse topological order (children
/// before parents).
fn compute_commits_heads(
    target_commit_ids: &IndexSet<CommitId>,
    connected_target_commits: &[Commit],
) -> Vec<CommitId> {
    let mut target_head_ids: HashSet<CommitId> = HashSet::new();
    for commit in connected_target_commits.iter().rev() {
        target_head_ids.insert(commit.id().clone());
        for old_parent in commit.parent_ids() {
            target_head_ids.remove(old_parent);
        }
    }
    connected_target_commits
        .iter()
        .rev()
        .filter(|commit| {
            target_head_ids.contains(commit.id()) && target_commit_ids.contains(commit.id())
        })
        .map(|commit| commit.id().clone())
        .collect_vec()
}

pub struct CommitWithSelection {
    pub commit: Commit,
    pub selected_tree: MergedTree,
    pub parent_tree: MergedTree,
}

impl CommitWithSelection {
    /// Returns true if the selection contains all changes in the commit.
    pub fn is_full_selection(&self) -> bool {
        self.selected_tree.tree_ids() == self.commit.tree_ids()
    }

    /// Returns true if the selection matches the parent tree (contains no
    /// changes from the commit).
    ///
    /// Both `is_full_selection()` and `is_empty_selection()`
    /// can be true if the commit is itself empty.
    pub fn is_empty_selection(&self) -> bool {
        self.selected_tree.tree_ids() == self.parent_tree.tree_ids()
    }
}

/// Resulting commit builder and stats to be returned by [`squash_commits()`].
#[must_use]
pub struct SquashedCommit<'repo> {
    /// New destination commit will be created by this builder.
    pub commit_builder: CommitBuilder<'repo>,
    /// List of abandoned source commits.
    pub abandoned_commits: Vec<Commit>,
}

/// Squash `sources` into `destination` and return a [`SquashedCommit`] for the
/// resulting commit. Caller is responsible for setting the description and
/// finishing the commit.
pub fn squash_commits<'repo>(
    repo: &'repo mut MutableRepo,
    sources: &[CommitWithSelection],
    destination: &Commit,
    keep_emptied: bool,
) -> BackendResult<Option<SquashedCommit<'repo>>> {
    struct SourceCommit<'a> {
        commit: &'a CommitWithSelection,
        abandon: bool,
    }
    let mut source_commits = vec![];
    for source in sources {
        let abandon = !keep_emptied && source.is_full_selection();
        if !abandon && source.is_empty_selection() {
            // Nothing selected from this commit. If it's abandoned (i.e. already empty), we
            // still include it so `jj squash` can be used for abandoning an empty commit in
            // the middle of a stack.
            continue;
        }

        // TODO: Do we want to optimize the case of moving to the parent commit (`jj
        // squash -r`)? The source tree will be unchanged in that case.
        source_commits.push(SourceCommit {
            commit: source,
            abandon,
        });
    }

    if source_commits.is_empty() {
        return Ok(None);
    }

    let mut abandoned_commits = vec![];
    for source in &source_commits {
        if source.abandon {
            repo.record_abandoned_commit(&source.commit.commit);
            abandoned_commits.push(source.commit.commit.clone());
        } else {
            let source_tree = source.commit.commit.tree();
            // Apply the reverse of the selected changes onto the source
            let new_source_tree = source_tree
                .merge(
                    source.commit.selected_tree.clone(),
                    source.commit.parent_tree.clone(),
                )
                .block_on()?;
            repo.rewrite_commit(&source.commit.commit)
                .set_tree(new_source_tree)
                .write()?;
        }
    }

    let mut rewritten_destination = destination.clone();
    if fallible_any(sources, |source| {
        repo.index()
            .is_ancestor(source.commit.id(), destination.id())
    })
    // TODO: indexing error shouldn't be a "BackendError"
    .map_err(|err| BackendError::Other(err.into()))?
    {
        // If we're moving changes to a descendant, first rebase descendants onto the
        // rewritten sources. Otherwise it will likely already have the content
        // changes we're moving, so applying them will have no effect and the
        // changes will disappear.
        let options = RebaseOptions::default();
        repo.rebase_descendants_with_options(&options, |old_commit, rebased_commit| {
            if old_commit.id() != destination.id() {
                return;
            }
            rewritten_destination = match rebased_commit {
                RebasedCommit::Rewritten(commit) => commit,
                RebasedCommit::Abandoned { .. } => panic!("all commits should be kept"),
            };
        })?;
    }
    // Apply the selected changes onto the destination
    let mut destination_tree = rewritten_destination.tree();
    for source in &source_commits {
        destination_tree = destination_tree
            .merge(
                source.commit.parent_tree.clone(),
                source.commit.selected_tree.clone(),
            )
            .block_on()?;
    }
    let mut predecessors = vec![destination.id().clone()];
    predecessors.extend(
        source_commits
            .iter()
            .map(|source| source.commit.commit.id().clone()),
    );

    let commit_builder = repo
        .rewrite_commit(&rewritten_destination)
        .set_tree(destination_tree)
        .set_predecessors(predecessors);
    Ok(Some(SquashedCommit {
        commit_builder,
        abandoned_commits,
    }))
}

/// Find divergent commits from the target that are already present with
/// identical contents in the destination. These commits should be able to be
/// safely abandoned.
pub fn find_duplicate_divergent_commits(
    repo: &dyn Repo,
    new_parent_ids: &[CommitId],
    target: &MoveCommitsTarget,
) -> BackendResult<Vec<Commit>> {
    let target_commits: Vec<Commit> = match target {
        MoveCommitsTarget::Commits(commit_ids) => commit_ids
            .iter()
            .map(|commit_id| repo.store().get_commit(commit_id))
            .try_collect()?,
        MoveCommitsTarget::Roots(root_ids) => RevsetExpression::commits(root_ids.clone())
            .descendants()
            .evaluate(repo)
            .map_err(|err| err.into_backend_error())?
            .iter()
            .commits(repo.store())
            .try_collect()
            .map_err(|err| err.into_backend_error())?,
    };
    let target_commit_ids: HashSet<&CommitId> = target_commits.iter().map(Commit::id).collect();

    // For each divergent change being rebased, we want to find all of the other
    // commits with the same change ID which are not being rebased.
    let divergent_changes: Vec<(&Commit, Vec<CommitId>)> = target_commits
        .iter()
        .map(|target_commit| -> Result<_, BackendError> {
            let mut ancestor_candidates = repo
                .resolve_change_id(target_commit.change_id())
                // TODO: indexing error shouldn't be a "BackendError"
                .map_err(|err| BackendError::Other(err.into()))?
                .unwrap_or_default();
            ancestor_candidates.retain(|commit_id| !target_commit_ids.contains(commit_id));
            Ok((target_commit, ancestor_candidates))
        })
        .filter_ok(|(_, candidates)| !candidates.is_empty())
        .try_collect()?;
    if divergent_changes.is_empty() {
        return Ok(Vec::new());
    }

    let target_root_ids = match target {
        MoveCommitsTarget::Commits(commit_ids) => commit_ids,
        MoveCommitsTarget::Roots(root_ids) => root_ids,
    };

    // We only care about divergent changes which are new ancestors of the rebased
    // commits, not ones which were already ancestors of the rebased commits.
    let is_new_ancestor = RevsetExpression::commits(target_root_ids.clone())
        .range(&RevsetExpression::commits(new_parent_ids.to_owned()))
        .evaluate(repo)
        .map_err(|err| err.into_backend_error())?
        .containing_fn();

    let mut duplicate_divergent = Vec::new();
    // Checking every pair of commits between these two sets could be expensive if
    // there are several commits with the same change ID. However, it should be
    // uncommon to have more than a couple commits with the same change ID being
    // rebased at the same time, so it should be good enough in practice.
    for (target_commit, ancestor_candidates) in divergent_changes {
        for ancestor_candidate_id in ancestor_candidates {
            if !is_new_ancestor(&ancestor_candidate_id).map_err(|err| err.into_backend_error())? {
                continue;
            }

            let ancestor_candidate = repo.store().get_commit(&ancestor_candidate_id)?;
            let new_tree =
                rebase_to_dest_parent(repo, slice::from_ref(target_commit), &ancestor_candidate)?;
            // Check whether the rebased commit would have the same tree as the existing
            // commit if they had the same parents. If so, we can skip this rebased commit.
            if new_tree.tree_ids() == ancestor_candidate.tree_ids() {
                duplicate_divergent.push(target_commit.clone());
                break;
            }
        }
    }
    Ok(duplicate_divergent)
}
