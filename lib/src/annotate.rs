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

//! Methods that allow annotation (attribution and blame) for a file in a
//! repository.
//!
//! TODO: Add support for different blame layers with a trait in the future.
//! Like commit metadata and more.

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::collections::hash_map;
use std::iter;
use std::ops::Range;
use std::sync::Arc;

use bstr::BStr;
use bstr::BString;
use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use itertools::Itertools as _;

use crate::backend::BackendError;
use crate::backend::BackendResult;
use crate::backend::CommitId;
use crate::commit::Commit;
use crate::conflicts::ConflictMarkerStyle;
use crate::conflicts::ConflictMaterializeOptions;
use crate::conflicts::MaterializedTreeValue;
use crate::conflicts::materialize_merge_result_to_bytes;
use crate::conflicts::materialize_tree_value;
use crate::diff::ContentDiff;
use crate::diff::DiffHunkKind;
use crate::files::FileMergeHunkLevel;
use crate::fileset::FilesetExpression;
use crate::graph::GraphEdge;
use crate::graph::GraphNode;
use crate::merge::SameChange;
use crate::merged_tree::MergedTree;
use crate::repo::Repo;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::revset::ResolvedRevsetExpression;
use crate::revset::RevsetEvaluationError;
use crate::revset::RevsetExpression;
use crate::revset::RevsetFilterPredicate;
use crate::store::Store;
use crate::tree_merge::MergeOptions;

/// Annotation results for a specific file
#[derive(Clone, Debug)]
pub struct FileAnnotation {
    line_map: OriginalLineMap,
    text: BString,
}

impl FileAnnotation {
    /// Returns iterator over `(line_origin, line)`s.
    ///
    /// For each line, `Ok(line_origin)` returns information about the
    /// originator commit of the line. If no originator commit was found
    /// within the domain, `Err(line_origin)` should be set. It points to the
    /// commit outside of the domain where the search stopped.
    ///
    /// The `line` includes newline character.
    pub fn line_origins(&self) -> impl Iterator<Item = (Result<&LineOrigin, &LineOrigin>, &BStr)> {
        itertools::zip_eq(&self.line_map, self.text.split_inclusive(|b| *b == b'\n'))
            .map(|(line_origin, line)| (line_origin.as_ref(), line.as_ref()))
    }

    /// Returns iterator over `(commit_id, line)`s.
    ///
    /// For each line, `Ok(commit_id)` points to the originator commit of the
    /// line. If no originator commit was found within the domain,
    /// `Err(commit_id)` should be set. It points to the commit outside of the
    /// domain where the search stopped.
    ///
    /// The `line` includes newline character.
    pub fn lines(&self) -> impl Iterator<Item = (Result<&CommitId, &CommitId>, &BStr)> {
        itertools::zip_eq(
            self.commit_ids(),
            self.text
                .split_inclusive(|b| *b == b'\n')
                .map(AsRef::as_ref),
        )
    }

    /// Returns iterator over `(commit_id, line_range)`s.
    ///
    /// See [`Self::lines()`] for `commit_id`s.
    ///
    /// The `line_range` is a slice range in the file `text`. Consecutive ranges
    /// having the same `commit_id` are not compacted.
    pub fn line_ranges(
        &self,
    ) -> impl Iterator<Item = (Result<&CommitId, &CommitId>, Range<usize>)> {
        let ranges = self
            .text
            .split_inclusive(|b| *b == b'\n')
            .scan(0, |total, line| {
                let start = *total;
                *total += line.len();
                Some(start..*total)
            });
        itertools::zip_eq(self.commit_ids(), ranges)
    }

    /// Returns iterator over compacted `(commit_id, line_range)`s.
    ///
    /// Consecutive ranges having the same `commit_id` are merged into one.
    pub fn compact_line_ranges(
        &self,
    ) -> impl Iterator<Item = (Result<&CommitId, &CommitId>, Range<usize>)> {
        let mut ranges = self.line_ranges();
        let mut acc = ranges.next();
        iter::from_fn(move || {
            let (acc_commit_id, acc_range) = acc.as_mut()?;
            for (cur_commit_id, cur_range) in ranges.by_ref() {
                if *acc_commit_id == cur_commit_id {
                    acc_range.end = cur_range.end;
                } else {
                    return acc.replace((cur_commit_id, cur_range));
                }
            }
            acc.take()
        })
    }

    /// File content at the starting commit.
    pub fn text(&self) -> &BStr {
        self.text.as_ref()
    }

    fn commit_ids(&self) -> impl Iterator<Item = Result<&CommitId, &CommitId>> {
        self.line_map.iter().map(|line_origin| {
            line_origin
                .as_ref()
                .map(|origin| &origin.commit_id)
                .map_err(|origin| &origin.commit_id)
        })
    }
}

/// Annotation process for a specific file.
#[derive(Clone, Debug)]
pub struct FileAnnotator {
    // If we add copy-tracing support, file_path might be tracked by state.
    file_path: RepoPathBuf,
    starting_text: BString,
    state: AnnotationState,
}

impl FileAnnotator {
    /// Initializes annotator for a specific file in the `starting_commit`.
    ///
    /// If the file is not found, the result would be empty.
    pub async fn from_commit(
        starting_commit: &Commit,
        file_path: &RepoPath,
    ) -> BackendResult<Self> {
        let source = Source::load(starting_commit, file_path).await?;
        Ok(Self::with_source(starting_commit.id(), file_path, source))
    }

    /// Initializes annotator for a specific file path starting with the given
    /// content.
    ///
    /// The file content at the `starting_commit` is set to `starting_text`.
    /// This is typically one of the file contents in the conflict or
    /// merged-parent tree.
    pub fn with_file_content(
        starting_commit_id: &CommitId,
        file_path: &RepoPath,
        starting_text: impl Into<Vec<u8>>,
    ) -> Self {
        let source = Source::new(BString::new(starting_text.into()));
        Self::with_source(starting_commit_id, file_path, source)
    }

    fn with_source(
        starting_commit_id: &CommitId,
        file_path: &RepoPath,
        mut source: Source,
    ) -> Self {
        source.fill_line_map();
        let starting_text = source.text.clone();
        let state = AnnotationState {
            original_line_map: (0..source.line_map.len())
                .map(|line_number| {
                    Err(LineOrigin {
                        commit_id: starting_commit_id.clone(),
                        line_number,
                    })
                })
                .collect(),
            commit_source_map: HashMap::from([(starting_commit_id.clone(), source)]),
            num_unresolved_roots: 0,
        };
        Self {
            file_path: file_path.to_owned(),
            starting_text,
            state,
        }
    }

    /// Computes line-by-line annotation within the `domain`.
    ///
    /// The `domain` expression narrows the range of ancestors to search. It
    /// will be intersected as `domain & ::pending_commits & files(file_path)`.
    /// The `pending_commits` is assumed to be included in the `domain`.
    pub async fn compute(
        &mut self,
        repo: &dyn Repo,
        domain: &Arc<ResolvedRevsetExpression>,
    ) -> Result<(), RevsetEvaluationError> {
        process_commits(repo, &mut self.state, domain, &self.file_path).await
    }

    /// Remaining commit ids to visit from.
    pub fn pending_commits(&self) -> impl Iterator<Item = &CommitId> {
        self.state.commit_source_map.keys()
    }

    /// Returns the current state as line-oriented annotation.
    pub fn to_annotation(&self) -> FileAnnotation {
        // Just clone the line map. We might want to change the underlying data
        // model something akin to interleaved delta in order to get annotation
        // at a certain ancestor commit without recomputing.
        FileAnnotation {
            line_map: self.state.original_line_map.clone(),
            text: self.starting_text.clone(),
        }
    }
}

/// Intermediate state of file annotation.
#[derive(Clone, Debug)]
struct AnnotationState {
    original_line_map: OriginalLineMap,
    /// Commits to file line mappings and contents.
    commit_source_map: HashMap<CommitId, Source>,
    /// Number of unresolved root commits in `commit_source_map`.
    num_unresolved_roots: usize,
}

/// Line mapping and file content at a certain commit.
#[derive(Clone, Debug)]
struct Source {
    /// Mapping of line numbers in the file at the current commit to the
    /// starting file, sorted by the line numbers at the current commit.
    line_map: Vec<(usize, usize)>,
    /// File content at the current commit.
    text: BString,
}

impl Source {
    fn new(text: BString) -> Self {
        Self {
            line_map: Vec::new(),
            text,
        }
    }

    async fn load(commit: &Commit, file_path: &RepoPath) -> Result<Self, BackendError> {
        let text = file_text_at_commit(commit, file_path).await?;
        Ok(Self::new(text))
    }

    fn fill_line_map(&mut self) {
        let lines = self.text.split_inclusive(|b| *b == b'\n');
        self.line_map = lines.enumerate().map(|(i, _)| (i, i)).collect();
    }
}

/// List of origins for each line, indexed by line numbers in the
/// starting file.
type OriginalLineMap = Vec<Result<LineOrigin, LineOrigin>>;

/// Information about the origin of an annotated line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineOrigin {
    /// Commit ID where the line was introduced.
    pub commit_id: CommitId,
    /// 0-based line number of the line in the origin commit.
    pub line_number: usize,
}

/// Concurrently fetches file content at a set of commits, deduplicating
/// concurrent requests for the same commit.
///
/// A commit that's a parent of several branches would otherwise be fetched
/// once per branch; `requested` collapses those into one fetch (see
/// `test_content_prefetcher_dedups_requests`), while up to
/// [`Store::concurrency()`] fetches run in parallel. The dedup lasts only
/// while a request is outstanding -- see the `commit_source_map.remove()`
/// comment in `process_commit()`.
struct ContentPrefetcher {
    store: Arc<Store>,
    file_name: RepoPathBuf,
    /// In-flight fetches. Each reads the file fully into a `BString` so the
    /// slow path (incl. `read_all()`) runs concurrently, not later in
    /// `process_commit()`.
    pending: FuturesUnordered<BoxFuture<'static, (CommitId, BackendResult<BString>)>>,
    /// Commit ids in `pending` or `completed`, to avoid fetching one twice.
    /// Removed on `take()`, so a later request re-fetches.
    requested: HashSet<CommitId>,
    /// Finished fetches not yet consumed by `take()`, keyed by commit id.
    completed: HashMap<CommitId, BString>,
}

impl ContentPrefetcher {
    fn new(store: &Arc<Store>, file_name: &RepoPath) -> Self {
        Self {
            store: store.clone(),
            file_name: file_name.to_owned(),
            pending: FuturesUnordered::new(),
            requested: HashSet::new(),
            completed: HashMap::new(),
        }
    }

    /// Starts fetching `commit_id`'s content, unless already requested.
    ///
    /// Not capped here: the caller's look-ahead bounds how many run at once,
    /// and `store.concurrency()` is only a soft hint, so a few extra is fine.
    fn prefetch(&mut self, commit_id: &CommitId) {
        if !self.requested.insert(commit_id.clone()) {
            return;
        }
        let store = self.store.clone();
        let file_name = self.file_name.clone();
        let commit_id = commit_id.clone();
        self.pending.push(
            async move {
                let result = async {
                    let commit = store.get_commit_async(&commit_id).await?;
                    file_text_at_commit(&commit, &file_name).await
                }
                .await;
                (commit_id, result)
            }
            .boxed(),
        );
    }

    /// Returns the content at `commit_id`, prefetching it first if needed.
    ///
    /// Other fetches may finish while waiting; their results are stashed in
    /// `completed` for later `take()` calls.
    async fn take(&mut self, commit_id: &CommitId) -> BackendResult<BString> {
        self.prefetch(commit_id);
        if let Some(text) = self.completed.remove(commit_id) {
            self.requested.remove(commit_id);
            return Ok(text);
        }
        loop {
            let (id, result) = self
                .pending
                .next()
                .await
                .expect("commit_id is pending since it was just prefetched");
            if id == *commit_id {
                let text = result?;
                self.requested.remove(commit_id);
                return Ok(text);
            }
            // A different prefetched commit finished first. Stash it for a
            // later `take()` -- but only if it's still wanted. `retain()` may
            // have dropped it from `requested` after this fetch was already in
            // flight (its future can't be pulled out of `pending`), in which
            // case we discard the result here, including any error: failing the
            // whole annotation for a commit on a dead branch we'll never visit
            // would be wrong.
            if self.requested.contains(&id) {
                self.completed.insert(id, result?);
            }
        }
    }

    /// Stops tracking any requested commit that no longer satisfies `keep`,
    /// dropping its stashed content and freeing its `requested` slot.
    ///
    /// A fetch already in flight isn't aborted (it can't be removed from
    /// `pending`); its result is discarded by `take()` once it lands, since the
    /// commit is no longer in `requested`. So this reclaims the concurrency
    /// slot and memory, not the in-flight backend request itself.
    fn retain(&mut self, mut keep: impl FnMut(&CommitId) -> bool) {
        self.requested.retain(|id| {
            if keep(id) {
                true
            } else {
                self.completed.remove(id);
                false
            }
        });
    }
}

/// Loads the content of `file_path` in `commit`'s tree.
async fn file_text_at_commit(
    commit: &Commit,
    file_path: &RepoPath,
) -> Result<BString, BackendError> {
    let tree = commit.tree();
    get_file_contents(commit.store(), file_path, &tree).await
}

/// The commits still worth fetching: the live frontier (`commit_source_map`)
/// plus the ancestors reachable from it through the buffered `lookahead`.
///
/// Ancestors of a dead branch (one whose lines dropped to 0, so it's no longer
/// in `commit_source_map`) are excluded, so the caller can stop prefetching
/// them and cancel any fetch already issued.
fn reachable_commits(
    commit_source_map: &HashMap<CommitId, Source>,
    lookahead: &VecDeque<GraphNode<CommitId>>,
) -> HashSet<CommitId> {
    let mut reachable: HashSet<CommitId> = commit_source_map.keys().cloned().collect();
    // `lookahead` is in topological order (children before parents), so a single
    // pass propagates reachability down to every buffered ancestor.
    for (commit_id, edges) in lookahead {
        if reachable.contains(commit_id) {
            reachable.extend(edges.iter().map(|edge| edge.target.clone()));
        }
    }
    reachable
}

/// Starting from the source commits, compute changes at that commit relative to
/// its direct parents, updating the mappings as we go.
async fn process_commits(
    repo: &dyn Repo,
    state: &mut AnnotationState,
    domain: &Arc<ResolvedRevsetExpression>,
    file_name: &RepoPath,
) -> Result<(), RevsetEvaluationError> {
    let predicate = RevsetFilterPredicate::File(FilesetExpression::file_path(file_name.to_owned()));
    // TODO: If the domain isn't a contiguous range, changes masked out by it
    // might not be caught by the closest ancestor revision. For example,
    // domain=merges() would pick up almost nothing because merge revisions
    // are usually empty. Perhaps, we want to query `files(file_path,
    // within_sub_graph=domain)`, not `domain & files(file_path)`.
    let heads = RevsetExpression::commits(state.commit_source_map.keys().cloned().collect());
    let revset = heads
        .union(&domain.intersection(&heads.ancestors()).filtered(predicate))
        .evaluate(repo)?;

    state.num_unresolved_roots = 0;
    let concurrency = repo.store().concurrency();
    let mut prefetcher = ContentPrefetcher::new(repo.store(), file_name);
    let mut nodes = revset.stream_graph().fuse();
    // Fill the lookahead while fewer than `concurrency` fetches are
    // outstanding, so roughly that many stay in flight. `lookahead.len()`
    // caps buffering when no parents need prefetching (all already in
    // `commit_source_map`), and `lookahead.is_empty()` ensures we always
    // pull at least one node so the outer loop can make progress.
    let mut lookahead: VecDeque<GraphNode<CommitId>> = VecDeque::new();
    // `reachable` is the set of commits still worth fetching: the live branches
    // (`commit_source_map`) plus the ancestors reachable from them through the
    // buffered lookahead. Once a branch goes dead (its lines drop to 0), its
    // ancestors fall out of `reachable`, and we stop pulling them down.
    //
    // What this saves: we never *issue* fetches for the ancestors of a dead
    // branch beyond the current lookahead window (the gate below skips them),
    // and `retain()` frees the `requested` slots a dead branch was holding so
    // the lookahead can refill with live work and its stashed `completed` texts
    // are dropped. On a long superseded merge side this is the difference
    // between fetching the whole branch and fetching ~one window of it.
    //
    // What this does *not* save: a fetch already in flight when its branch dies
    // still runs to completion -- its future can't be pulled out of
    // `FuturesUnordered`, so we only drop the result when it lands, not the
    // backend request. So we always pay for up to one lookahead window of
    // speculation past each branch's death; we just don't pay for the rest.
    let mut reachable = reachable_commits(&state.commit_source_map, &lookahead);
    loop {
        while (prefetcher.requested.len() < concurrency && lookahead.len() < concurrency)
            || lookahead.is_empty()
        {
            let Some(node) = nodes.try_next().await? else {
                break;
            };
            let (commit_id, edges) = &node;
            // Don't prefetch the parents of a node whose branch is already dead
            // -- they're unreachable. (The node itself is still popped below;
            // `process_commit` early-returns for it.)
            if reachable.contains(commit_id) {
                for edge in edges {
                    // Grow `reachable` as live ancestors enter the frontier.
                    // This inline insert is why the recompute below only has to
                    // handle shrinkage (dead branches), not growth.
                    reachable.insert(edge.target.clone());
                    // Parents already in `commit_source_map` are read from there,
                    // so prefetching them would be wasted.
                    if !state.commit_source_map.contains_key(&edge.target) {
                        prefetcher.prefetch(&edge.target);
                    }
                }
            }
            lookahead.push_back(node);
        }
        let Some((commit_id, edge_list)) = lookahead.pop_front() else {
            break;
        };
        let dropped_parent =
            process_commit(&mut prefetcher, state, &commit_id, &edge_list).await?;
        if state.commit_source_map.len() == state.num_unresolved_roots {
            // No more lines to propagate to ancestors.
            break;
        }

        // `reachable` only ever needs to *shrink*, and only when a branch dies.
        // Growth is already handled inline: the gate above inserts each live
        // node's parents into `reachable` as it buffers them, so new ancestors
        // enter the set without a rebuild. The one thing that inline insert
        // can't do is remove entries, so a full recompute is needed precisely
        // when a branch dies -- i.e. when `process_commit()` just dropped a
        // parent whose lines reached 0. On every other iteration `reachable` is
        // already correct, so we skip the O(commit_source_map) rebuild. In
        // linear history (no branch ever dies) this never runs; in bushy history
        // it runs only on the iterations where a branch actually dies.
        //
        // TODO: even gated, each rebuild is O(commit_source_map). We could
        // instead maintain `reachable` incrementally with per-commit referrer
        // counts, decrementing as branches die, for O(V+E) total -- but that
        // moves the bookkeeping into `AnnotationState` and isn't worth it until
        // a wide-frontier workload shows the gated rebuild on the hot path.
        if dropped_parent {
            reachable = reachable_commits(&state.commit_source_map, &lookahead);
            // Drop any prefetch whose branch just died. This frees `requested`
            // slots so the gate above lets live work back in; the orphaned
            // futures stay in `pending` until they resolve and `take()` discards
            // them, so the gate can briefly undercount the fetches truly in
            // flight.
            prefetcher.retain(|id| reachable.contains(id));
        }
    }
    Ok(())
}

/// For a given commit, for each parent, we compare the version in the parent
/// tree with the current version, updating the mappings for any lines in
/// common. If the parent doesn't have the file, we skip it.
/// Returns `true` if a parent was dropped from `state.commit_source_map`
/// because it had no lines left to propagate. That's the only event that can
/// shrink the set of commits still worth fetching, so the caller uses it to
/// decide whether to recompute reachability (see `process_commits()`).
async fn process_commit(
    prefetcher: &mut ContentPrefetcher,
    state: &mut AnnotationState,
    current_commit_id: &CommitId,
    edges: &[GraphEdge<CommitId>],
) -> BackendResult<bool> {
    let Some(mut current_source) = state.commit_source_map.remove(current_commit_id) else {
        return Ok(false);
    };

    let mut dropped_parent = false;

    for parent_edge in edges {
        let parent_commit_id = &parent_edge.target;
        let parent_source = match state.commit_source_map.entry(parent_commit_id.clone()) {
            // Content was already loaded by a different branch that visited
            // this parent earlier. The look-ahead in `process_commits()` skips
            // prefetching parents already in `commit_source_map`, so the
            // prefetcher holds nothing for this commit to release here.
            hash_map::Entry::Occupied(entry) => entry.into_mut(),
            hash_map::Entry::Vacant(entry) => {
                let text = prefetcher.take(entry.key()).await?;
                entry.insert(Source::new(text))
            }
        };

        // For two versions of the same file, for all the lines in common,
        // overwrite the new mapping in the results for the new commit. Let's
        // say I have a file in commit A and commit B. We know that according to
        // local line_map, in commit A, line 3 corresponds to line 7 of the
        // starting file. Now, line 3 in Commit A corresponds to line 6 in
        // commit B. Then, we update local line_map to say that "Commit B line 6
        // goes to line 7 of the starting file". We repeat this for all lines in
        // common in the two commits.
        let mut current_lines = current_source.line_map.iter().copied().peekable();
        let mut new_current_line_map = Vec::new();
        let mut new_parent_line_map = Vec::new();
        copy_same_lines_with(
            &current_source.text,
            &parent_source.text,
            |current_start, parent_start, count| {
                new_current_line_map
                    .extend(current_lines.peeking_take_while(|&(cur, _)| cur < current_start));
                while let Some((current, starting)) =
                    current_lines.next_if(|&(cur, _)| cur < current_start + count)
                {
                    let parent = parent_start + (current - current_start);
                    new_parent_line_map.push((parent, starting));
                }
            },
        );
        new_current_line_map.extend(current_lines);
        current_source.line_map = new_current_line_map;
        parent_source.line_map = if parent_source.line_map.is_empty() {
            new_parent_line_map
        } else {
            itertools::merge(parent_source.line_map.iter().copied(), new_parent_line_map).collect()
        };
        if parent_source.line_map.is_empty() {
            // This parent has no lines left to propagate, so drop it rather
            // than pin its content. If a later commit shares this parent, it
            // sees a `Vacant` entry and re-fetches -- the prefetcher dedups
            // only concurrent requests, not this sequential re-fetch. That
            // re-fetch is the intended trade-off, not a missed dedup. (A dead
            // branch is different: `process_commits()` drops its ancestors from
            // `reachable`, so they're never prefetched at all.)
            state.commit_source_map.remove(parent_commit_id);
            dropped_parent = true;
        } else if parent_edge.is_missing() {
            // If an omitted parent had the file, leave these lines unresolved.
            // The origin of the unresolved lines is represented as
            // Err(LineOrigin { parent_commit_id, parent_line_number }).
            for &(parent_line_number, starting_line_number) in &parent_source.line_map {
                state.original_line_map[starting_line_number] = Err(LineOrigin {
                    commit_id: parent_commit_id.clone(),
                    line_number: parent_line_number,
                });
            }
            state.num_unresolved_roots += 1;
        }
    }

    // Once we've looked at all parents of a commit, any leftover lines must be
    // original to the current commit, so we save this information in
    // original_line_map.
    for (current_line_number, starting_line_number) in current_source.line_map {
        state.original_line_map[starting_line_number] = Ok(LineOrigin {
            commit_id: current_commit_id.clone(),
            line_number: current_line_number,
        });
    }

    Ok(dropped_parent)
}

/// For two files, calls `copy(current_start, parent_start, count)` for each
/// range of contiguous lines in common (e.g. line 8-10 maps to line 9-11.)
fn copy_same_lines_with(
    current_contents: &[u8],
    parent_contents: &[u8],
    mut copy: impl FnMut(usize, usize, usize),
) {
    let diff = ContentDiff::by_line([current_contents, parent_contents]);
    let mut current_line_counter: usize = 0;
    let mut parent_line_counter: usize = 0;
    for hunk in diff.hunks() {
        match hunk.kind {
            DiffHunkKind::Matching => {
                let count = hunk.contents[0].split_inclusive(|b| *b == b'\n').count();
                copy(current_line_counter, parent_line_counter, count);
                current_line_counter += count;
                parent_line_counter += count;
            }
            DiffHunkKind::Different => {
                let current_output = hunk.contents[0];
                let parent_output = hunk.contents[1];
                current_line_counter += current_output.split_inclusive(|b| *b == b'\n').count();
                parent_line_counter += parent_output.split_inclusive(|b| *b == b'\n').count();
            }
        }
    }
}

async fn get_file_contents(
    store: &Store,
    path: &RepoPath,
    tree: &MergedTree,
) -> Result<BString, BackendError> {
    let file_value = tree.path_value(path).await?;
    let effective_file_value =
        materialize_tree_value(store, path, file_value, tree.labels()).await?;
    match effective_file_value {
        MaterializedTreeValue::File(mut file) => Ok(file.read_all(path).await?.into()),
        MaterializedTreeValue::FileConflict(file) => {
            // TODO: track line origins without materializing
            let options = ConflictMaterializeOptions {
                marker_style: ConflictMarkerStyle::Diff,
                marker_len: None,
                merge: MergeOptions {
                    hunk_level: FileMergeHunkLevel::Line,
                    same_change: SameChange::Accept,
                },
            };
            Ok(materialize_merge_result_to_bytes(
                &file.contents,
                &file.labels,
                &options,
            ))
        }
        _ => Ok(BString::default()),
    }
}

#[cfg(test)]
mod tests {
    use futures::FutureExt as _;
    use pollster::FutureExt as _;
    use tempfile::TempDir;

    use super::*;
    use crate::config::StackedConfig;
    use crate::settings::UserSettings;
    use crate::signing::Signer;
    use crate::simple_backend::SimpleBackend;

    fn make_line_origin(commit_id: &CommitId, line_number: usize) -> LineOrigin {
        LineOrigin {
            commit_id: commit_id.clone(),
            line_number,
        }
    }

    /// An empty `SimpleBackend`-backed `Store` for exercising
    /// `ContentPrefetcher` and `process_commit()` directly. Owns the
    /// backing `TempDir`, so it stays alive exactly as long as the `Store`
    /// (cf. `testutils::TestRepo`).
    ///
    /// Avoids `testutils` (which depends on `jj-lib`) to dodge a dependency
    /// cycle that would link two incompatible `jj-lib` instances; see
    /// `git_backend.rs`'s `user_settings()`. The tests never poll a real fetch
    /// future, so an empty store suffices.
    struct TestStore {
        _temp_dir: TempDir,
        store: Arc<Store>,
    }

    impl TestStore {
        fn init() -> Self {
            let temp_dir = crate::tests::new_temp_dir();
            let backend = SimpleBackend::init(temp_dir.path());
            let settings = UserSettings::from_config(StackedConfig::with_defaults()).unwrap();
            let signer = Signer::from_settings(&settings).unwrap();
            let merge_options = MergeOptions::from_settings(&settings).unwrap();
            let store = Store::new(Box::new(backend), signer, merge_options);
            Self {
                _temp_dir: temp_dir,
                store,
            }
        }
    }

    fn make_source(text: &str) -> Source {
        let mut source = Source::new(text.into());
        source.fill_line_map();
        source
    }

    #[test]
    fn test_lines_iterator_empty() {
        let annotation = FileAnnotation {
            line_map: vec![],
            text: "".into(),
        };
        assert_eq!(annotation.line_origins().collect_vec(), vec![]);
        assert_eq!(annotation.lines().collect_vec(), vec![]);
        assert_eq!(annotation.line_ranges().collect_vec(), vec![]);
        assert_eq!(annotation.compact_line_ranges().collect_vec(), vec![]);
    }

    #[test]
    fn test_lines_iterator_with_content() {
        let commit_id1 = CommitId::from_hex("111111");
        let commit_id2 = CommitId::from_hex("222222");
        let commit_id3 = CommitId::from_hex("333333");
        let annotation = FileAnnotation {
            line_map: vec![
                Ok(make_line_origin(&commit_id1, 0)),
                Ok(make_line_origin(&commit_id2, 1)),
                Ok(make_line_origin(&commit_id3, 2)),
            ],
            text: "foo\n\nbar\n".into(),
        };
        assert_eq!(
            annotation.line_origins().collect_vec(),
            vec![
                (Ok(&make_line_origin(&commit_id1, 0)), "foo\n".as_ref()),
                (Ok(&make_line_origin(&commit_id2, 1)), "\n".as_ref()),
                (Ok(&make_line_origin(&commit_id3, 2)), "bar\n".as_ref()),
            ]
        );
        assert_eq!(
            annotation.lines().collect_vec(),
            vec![
                (Ok(&commit_id1), "foo\n".as_ref()),
                (Ok(&commit_id2), "\n".as_ref()),
                (Ok(&commit_id3), "bar\n".as_ref()),
            ]
        );
        assert_eq!(
            annotation.line_ranges().collect_vec(),
            vec![
                (Ok(&commit_id1), 0..4),
                (Ok(&commit_id2), 4..5),
                (Ok(&commit_id3), 5..9),
            ]
        );
        assert_eq!(
            annotation.compact_line_ranges().collect_vec(),
            vec![
                (Ok(&commit_id1), 0..4),
                (Ok(&commit_id2), 4..5),
                (Ok(&commit_id3), 5..9),
            ]
        );
    }

    #[test]
    fn test_lines_iterator_compaction() {
        let commit_id1 = CommitId::from_hex("111111");
        let commit_id2 = CommitId::from_hex("222222");
        let commit_id3 = CommitId::from_hex("333333");
        let annotation = FileAnnotation {
            line_map: vec![
                Ok(make_line_origin(&commit_id1, 0)),
                Ok(make_line_origin(&commit_id1, 1)),
                Ok(make_line_origin(&commit_id2, 2)),
                Ok(make_line_origin(&commit_id1, 3)),
                Ok(make_line_origin(&commit_id3, 4)),
                Ok(make_line_origin(&commit_id3, 5)),
                Ok(make_line_origin(&commit_id3, 6)),
            ],
            text: "\n".repeat(7).into(),
        };
        assert_eq!(
            annotation.compact_line_ranges().collect_vec(),
            vec![
                (Ok(&commit_id1), 0..2),
                (Ok(&commit_id2), 2..3),
                (Ok(&commit_id1), 3..4),
                (Ok(&commit_id3), 4..7),
            ]
        );
    }

    /// A commit that's an edge target of two graph nodes (e.g. a parent of two
    /// merged branches) must be `prefetch()`-ed only once, so concurrent
    /// duplicate fetches don't waste backend I/O on merge-heavy repos.
    ///
    /// This tests `ContentPrefetcher`'s bookkeeping directly rather than going
    /// through `FileAnnotator`: the line-mapping algorithm re-visits a shared
    /// ancestor once per branch regardless, so the dedup is only observable on
    /// the prefetcher's internal state.
    #[test]
    fn test_content_prefetcher_dedups_requests() {
        let test_store = TestStore::init();
        let file_path = RepoPath::from_internal_string("file").unwrap();
        let commit_id = CommitId::from_hex("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");

        let mut prefetcher = ContentPrefetcher::new(&test_store.store, file_path);

        // Simulate two graph edges that both point at `commit_id` (as would
        // happen if it were a parent of two different branches), discovered
        // during the same look-ahead pass, before either is consumed by
        // `take()`.
        prefetcher.prefetch(&commit_id);
        prefetcher.prefetch(&commit_id);

        // The second `prefetch()` must not register a second fetch task.
        assert_eq!(prefetcher.requested.len(), 1);
        assert_eq!(prefetcher.pending.len(), 1);
    }

    /// After `process_commit()` runs, the prefetcher holds nothing for the
    /// parents it visited:
    ///
    /// - a parent fetched via the `Vacant` branch is consumed by `take()`,
    ///   which removes it from `requested` and `completed`;
    /// - a parent already in `commit_source_map` (the `Occupied` branch) was
    ///   never prefetched, since the look-ahead in `process_commits()` skips
    ///   parents already tracked there.
    ///
    /// This is the invariant that lets the `Occupied` branch omit any explicit
    /// prefetcher cleanup -- there is simply nothing to release. It also
    /// exercises the `!commit_source_map.contains_key()` look-ahead guard's
    /// intent: an already-tracked parent stays out of the prefetcher.
    #[test]
    fn test_process_commit_drains_visited_parents() {
        let test_store = TestStore::init();
        let file_path = RepoPath::from_internal_string("file").unwrap();

        let child = CommitId::from_hex("cccccccccccccccccccccccccccccccccccccccc");
        let visited_parent = CommitId::from_hex("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let new_parent = CommitId::from_hex("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

        let mut prefetcher = ContentPrefetcher::new(&test_store.store, file_path);
        // Stand in for a look-ahead fetch of `new_parent` that already
        // finished, so `take()` returns from `completed` without touching the
        // backend. `visited_parent` is deliberately absent: the look-ahead
        // never prefetches a parent that's already in `commit_source_map`.
        prefetcher.requested.insert(new_parent.clone());
        prefetcher
            .completed
            .insert(new_parent.clone(), "a\n".into());

        let mut state = AnnotationState {
            original_line_map: vec![
                Err(make_line_origin(&child, 0)),
                Err(make_line_origin(&child, 1)),
            ],
            commit_source_map: HashMap::from([
                (child.clone(), make_source("a\nb\n")),
                (visited_parent.clone(), make_source("a\n")),
            ]),
            num_unresolved_roots: 0,
        };

        let edges = [
            GraphEdge::direct(visited_parent.clone()),
            GraphEdge::direct(new_parent.clone()),
        ];
        process_commit(&mut prefetcher, &mut state, &child, &edges)
            .block_on()
            .unwrap();

        // `new_parent` was consumed by `take()`; `visited_parent` was read from
        // `commit_source_map` and never entered the prefetcher. Nothing is left
        // holding file content.
        assert!(prefetcher.requested.is_empty());
        assert!(prefetcher.completed.is_empty());
        assert_eq!(prefetcher.pending.len(), 0);
    }

    /// `ContentPrefetcher::retain` correctly cleans up both pending requests
    /// and completed fetches for commits that are no longer reachable.
    #[test]
    fn test_content_prefetcher_retains_reachable_commits() {
        let test_store = TestStore::init();
        let file_path = RepoPath::from_internal_string("file").unwrap();
        let mut prefetcher = ContentPrefetcher::new(&test_store.store, file_path);

        let reachable = CommitId::from_hex("1111111111111111111111111111111111111111");
        let unreachable1 = CommitId::from_hex("2222222222222222222222222222222222222222");
        let unreachable2 = CommitId::from_hex("3333333333333333333333333333333333333333");

        prefetcher.prefetch(&reachable);
        prefetcher.prefetch(&unreachable1);
        prefetcher.prefetch(&unreachable2);

        // Simulate one unreachable fetch finishing before cleanup
        prefetcher
            .completed
            .insert(unreachable1.clone(), "foo\n".into());

        assert_eq!(prefetcher.requested.len(), 3);
        assert_eq!(prefetcher.completed.len(), 1);

        prefetcher.retain(|id| id == &reachable);

        // Both unreachables (one pending, one completed) are removed.
        assert_eq!(prefetcher.requested.len(), 1);
        assert!(prefetcher.requested.contains(&reachable));
        assert_eq!(prefetcher.completed.len(), 0);
    }

    /// A fetch that `retain()` dropped while it was still in flight must not
    /// fail the annotation when it later resolves to an error: `take()` only
    /// propagates results for commits still in `requested`. Without that guard,
    /// an I/O error on a commit we abandoned (a dead branch) would abort the
    /// whole annotation.
    #[test]
    fn test_take_ignores_error_from_dropped_fetch() {
        let test_store = TestStore::init();
        let file_path = RepoPath::from_internal_string("file").unwrap();
        let mut prefetcher = ContentPrefetcher::new(&test_store.store, file_path);

        let wanted = CommitId::from_hex("1111111111111111111111111111111111111111");
        let dropped = CommitId::from_hex("2222222222222222222222222222222222222222");

        // `dropped` was prefetched, then dropped by `retain()` (removed from
        // `requested`) while its fetch is still pending. The fetch errors,
        // since the commit doesn't exist in the backend.
        prefetcher.prefetch(&dropped);
        prefetcher.requested.remove(&dropped);

        // `wanted` is requested but not yet stashed, so `take()` has to drain
        // `pending` -- hitting the erroring `dropped` fetch -- to reach it.
        let expected: BString = "wanted\n".into();
        prefetcher.requested.insert(wanted.clone());
        let wanted_for_future = wanted.clone();
        let expected_for_future = expected.clone();
        prefetcher
            .pending
            .push(async move { (wanted_for_future, Ok(expected_for_future)) }.boxed());

        let text = prefetcher.take(&wanted).block_on().unwrap();
        assert_eq!(text, expected);
    }
}
