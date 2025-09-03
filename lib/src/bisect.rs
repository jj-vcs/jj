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

//! Bisect a range of commits.

use std::collections::HashSet;
use std::sync::Arc;

use itertools::Itertools as _;
use thiserror::Error;

use crate::backend::CommitId;
use crate::commit::Commit;
use crate::repo::Repo;
use crate::revset::ResolvedRevsetExpression;
use crate::revset::RevsetEvaluationError;
use crate::revset::RevsetExpression;
use crate::revset::RevsetIteratorExt as _;

/// An error that occurred while bisecting
#[derive(Error, Debug)]
pub enum BisectionError {
    /// Failed to evaluate a revset
    #[error("Failed to evaluate a revset involved in bisection")]
    RevsetEvaluationError(#[from] RevsetEvaluationError),
}

/// Indicates whether a given commit was good, bad, or if it could not be
/// determined.
#[derive(Debug)]
pub enum Evaluation {
    /// The commit was good
    Good,
    /// The commit was bad
    Bad,
    /// It could not be determined whether the commit was good or bad
    Skip,
}

/// Performs bisection to find the first bad commit in a range.
pub struct Bisector<'repo> {
    repo: &'repo dyn Repo,
    input_range: Arc<ResolvedRevsetExpression>,
    good_commits: HashSet<CommitId>,
    bad_commits: HashSet<CommitId>,
    skipped_commits: HashSet<CommitId>,
}

/// The result of bisection.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum BisectionResult {
    /// Found the first bad commit(s). It should be exactly one unless the input
    /// range had multiple disjoint heads.
    Found(Vec<Commit>),
    /// Could not determine the first bad commit because it was in a
    /// skipped range.
    Indeterminate,
}

/// The next bisection step.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum NextStep {
    /// The commit must be evaluated.
    Evaluate(Commit),
    /// Bisection is complete.
    Done(BisectionResult),
}

impl<'repo> Bisector<'repo> {
    /// Create a new bisector. The range's heads are assumed to be bad.
    /// Parents of the range's roots are assumed to be good.
    pub fn new(
        repo: &'repo dyn Repo,
        input_range: Arc<ResolvedRevsetExpression>,
    ) -> Result<Self, BisectionError> {
        let bad_commits = input_range.heads().evaluate(repo)?.iter().try_collect()?;
        Ok(Self {
            repo,
            input_range,
            bad_commits,
            good_commits: HashSet::new(),
            skipped_commits: HashSet::new(),
        })
    }

    /// Mark a commit good.
    pub fn mark_good(&mut self, id: CommitId) {
        assert!(!self.bad_commits.contains(&id));
        assert!(!self.skipped_commits.contains(&id));
        self.good_commits.insert(id);
    }

    /// Mark a commit bad.
    pub fn mark_bad(&mut self, id: CommitId) {
        assert!(!self.good_commits.contains(&id));
        assert!(!self.skipped_commits.contains(&id));
        self.bad_commits.insert(id);
    }

    /// Mark a commit as skipped (cannot be determined if it's good or bad).
    pub fn mark_skipped(&mut self, id: CommitId) {
        assert!(!self.good_commits.contains(&id));
        assert!(!self.bad_commits.contains(&id));
        self.skipped_commits.insert(id);
    }

    /// Mark a commit as good, bad, or skipped, according to the outcome in
    /// `evaluation`.
    pub fn mark(&mut self, id: CommitId, evaluation: Evaluation) {
        match evaluation {
            Evaluation::Good => self.mark_good(id),
            Evaluation::Bad => self.mark_bad(id),
            Evaluation::Skip => self.mark_skipped(id),
        }
    }

    /// The commits that were marked good.
    pub fn good_commits(&self) -> &HashSet<CommitId> {
        &self.good_commits
    }

    /// The commits that were marked bad.
    pub fn bad_commits(&self) -> &HashSet<CommitId> {
        &self.bad_commits
    }

    /// The commits that were skipped.
    pub fn skipped_commits(&self) -> &HashSet<CommitId> {
        &self.skipped_commits
    }

    /// Find the next commit to evaluate, or determine that there are no more
    /// steps.
    pub fn next_step(&mut self) -> Result<NextStep, BisectionError> {
        let good_expr = RevsetExpression::commits(self.good_commits.iter().cloned().collect());
        let bad_expr = RevsetExpression::commits(self.bad_commits.iter().cloned().collect());
        let skipped_expr =
            RevsetExpression::commits(self.skipped_commits.iter().cloned().collect());
        // Intersect the input range with the current bad range and then bisect it to
        // find the next commit to evaluate.
        // Skipped revisions are simply subtracted from the set.
        // TODO: Handle long ranges of skipped revisions better
        let to_evaluate_expr = self
            .input_range
            .intersection(&good_expr.heads().range(&bad_expr.roots()))
            .minus(&bad_expr)
            .minus(&skipped_expr)
            .bisect()
            .latest(1);
        let to_evaluate_set = to_evaluate_expr.evaluate(self.repo)?;
        if let Some(commit) = to_evaluate_set
            .iter()
            .commits(self.repo.store())
            .next()
            .transpose()?
        {
            Ok(NextStep::Evaluate(commit))
        } else {
            let bad_roots = bad_expr.roots().evaluate(self.repo)?;
            let bad_commits: Vec<_> = bad_roots.iter().commits(self.repo.store()).try_collect()?;
            if bad_commits.is_empty() {
                Ok(NextStep::Done(BisectionResult::Indeterminate))
            } else {
                Ok(NextStep::Done(BisectionResult::Found(bad_commits)))
            }
        }
    }
}
