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

use std::rc::Rc;

use assert_matches::assert_matches;
use jj_lib::backend::CommitId;
use jj_lib::bisect::BisectionResult;
use jj_lib::bisect::Bisector;
use jj_lib::bisect::Evaluation;
use jj_lib::bisect::NextStep;
use jj_lib::repo::Repo;
use jj_lib::revset::ResolvedRevsetExpression;
use testutils::TestRepo;
use testutils::write_random_commit;
use testutils::write_random_commit_with_parents;

fn test_bisection<'a>(
    repo: &dyn Repo,
    input_range: &Rc<ResolvedRevsetExpression>,
    results: impl IntoIterator<Item = (&'a CommitId, Evaluation)>,
) -> BisectionResult {
    let mut bisector = Bisector::new(repo, input_range.clone()).unwrap();
    let mut iter = results.into_iter().enumerate();
    loop {
        match bisector.next_step().unwrap() {
            NextStep::Evaluate(commit) => {
                let (i, (expected_id, result)) =
                    iter.next().expect("More commits than expected were tested");
                assert_eq!(
                    commit.id(),
                    expected_id,
                    "Attempt to test unexpected commit at iteration {i}"
                );
                bisector.mark(commit.id().clone(), result);
            }
            NextStep::Done(bisection_result) => {
                assert!(iter.next().is_none(), "Finished earlier than expected");
                return bisection_result;
            }
        }
    }
}

#[test]
fn test_bisect_empty_input() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let input_range = ResolvedRevsetExpression::none();
    let expected_tests = [];
    let result = test_bisection(repo.as_ref(), &input_range, expected_tests);
    assert_matches!(result, BisectionResult::Indeterminate);
}

#[test]
fn test_bisect_linear() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let root_commit = repo.store().root_commit();
    let commit1 = write_random_commit(tx.repo_mut());
    let commit2 = write_random_commit_with_parents(tx.repo_mut(), &[&commit1]);
    let commit3 = write_random_commit_with_parents(tx.repo_mut(), &[&commit2]);
    let commit4 = write_random_commit_with_parents(tx.repo_mut(), &[&commit3]);
    let commit5 = write_random_commit_with_parents(tx.repo_mut(), &[&commit4]);
    let commit6 = write_random_commit_with_parents(tx.repo_mut(), &[&commit5]);
    let commit7 = write_random_commit_with_parents(tx.repo_mut(), &[&commit6]);

    let input_range = ResolvedRevsetExpression::commit(commit7.id().clone()).ancestors();

    // Root commit is the first bad commit
    let expected_tests = [
        (commit3.id(), Evaluation::Bad),
        (commit1.id(), Evaluation::Bad),
        (root_commit.id(), Evaluation::Bad),
    ];
    let result = test_bisection(tx.repo(), &input_range, expected_tests);
    assert_eq!(result, BisectionResult::Found(vec![root_commit.clone()]));

    // Commit 1 is the first bad commit
    let expected_tests = [
        (commit3.id(), Evaluation::Bad),
        (commit1.id(), Evaluation::Bad),
        (root_commit.id(), Evaluation::Good),
    ];
    let result = test_bisection(tx.repo(), &input_range, expected_tests);
    assert_eq!(result, BisectionResult::Found(vec![commit1.clone()]));

    // Commit 3 is the first bad commit
    let expected_tests = [
        (commit3.id(), Evaluation::Bad),
        (commit1.id(), Evaluation::Good),
        (commit2.id(), Evaluation::Good),
    ];
    let result = test_bisection(tx.repo(), &input_range, expected_tests);
    assert_eq!(result, BisectionResult::Found(vec![commit3.clone()]));

    // Commit 5 is the first bad commit
    let expected_tests = [
        (commit3.id(), Evaluation::Good),
        (commit5.id(), Evaluation::Bad),
        (commit4.id(), Evaluation::Good),
    ];
    let result = test_bisection(tx.repo(), &input_range, expected_tests);
    assert_eq!(result, BisectionResult::Found(vec![commit5.clone()]));

    // Commit 7 is the first bad commit
    let expected_tests = [
        (commit3.id(), Evaluation::Good),
        (commit5.id(), Evaluation::Good),
        (commit6.id(), Evaluation::Good),
    ];
    let result = test_bisection(tx.repo(), &input_range, expected_tests);
    assert_eq!(result, BisectionResult::Found(vec![commit7.clone()]));

    // Commit 2 is the first bad commit but commit 3 is skipped
    let expected_tests = [
        (commit3.id(), Evaluation::Skip),
        (commit2.id(), Evaluation::Bad),
        (root_commit.id(), Evaluation::Good),
        (commit1.id(), Evaluation::Good),
    ];
    let result = test_bisection(tx.repo(), &input_range, expected_tests);
    assert_eq!(result, BisectionResult::Found(vec![commit2.clone()]));

    // Commit 4 is the first bad commit but commit 3 is skipped
    let expected_tests = [
        (commit3.id(), Evaluation::Skip),
        (commit2.id(), Evaluation::Good),
        (commit5.id(), Evaluation::Bad),
        (commit4.id(), Evaluation::Bad),
    ];
    let result = test_bisection(tx.repo(), &input_range, expected_tests);
    // TODO: Indicate in the result that we're unsure if commit4 was the first bad
    // commit because the commit before it in the set was skipped.
    assert_eq!(result, BisectionResult::Found(vec![commit4.clone()]));

    // Commit 7 is the first bad commit but commits before 6 were skipped
    // TODO: Avoid testing every commit near first skipped commit. Test e.g. commit
    // 1 and commit 5 once we see that commit 3 was indeterminate.
    let expected_tests = [
        (commit3.id(), Evaluation::Skip),
        (commit2.id(), Evaluation::Skip),
        (commit4.id(), Evaluation::Skip),
        (commit1.id(), Evaluation::Skip),
        (commit5.id(), Evaluation::Skip),
        (root_commit.id(), Evaluation::Skip),
        (commit6.id(), Evaluation::Good),
    ];
    let result = test_bisection(tx.repo(), &input_range, expected_tests);
    assert_eq!(result, BisectionResult::Found(vec![commit7.clone()]));

    // Gaps in the input range are allowed
    let input_range = ResolvedRevsetExpression::commits(vec![
        commit7.id().clone(),
        commit4.id().clone(),
        commit2.id().clone(),
        commit1.id().clone(),
    ]);
    // Commit 4 is the first bad commit
    let expected_tests = [
        (commit2.id(), Evaluation::Good),
        (commit4.id(), Evaluation::Bad),
    ];
    let result = test_bisection(tx.repo(), &input_range, expected_tests);
    assert_eq!(result, BisectionResult::Found(vec![commit4.clone()]));
}

#[test]
fn test_bisect_nonlinear() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // 7
    // |\
    // 5 6
    // | |
    // 3 4
    // | |
    // 1 2
    // |/
    // 0
    let mut tx = repo.start_transaction();
    let root_commit = repo.store().root_commit();
    let commit1 = write_random_commit(tx.repo_mut());
    let commit2 = write_random_commit(tx.repo_mut());
    let commit3 = write_random_commit_with_parents(tx.repo_mut(), &[&commit1]);
    let commit4 = write_random_commit_with_parents(tx.repo_mut(), &[&commit2]);
    let commit5 = write_random_commit_with_parents(tx.repo_mut(), &[&commit3]);
    let commit6 = write_random_commit_with_parents(tx.repo_mut(), &[&commit4]);
    let commit7 = write_random_commit_with_parents(tx.repo_mut(), &[&commit5, &commit6]);

    let input_range = ResolvedRevsetExpression::commit(commit7.id().clone()).ancestors();

    // Root commit is the first bad commit
    let expected_tests = [
        (commit3.id(), Evaluation::Bad),
        (root_commit.id(), Evaluation::Bad),
    ];
    let result = test_bisection(tx.repo(), &input_range, expected_tests);
    assert_eq!(result, BisectionResult::Found(vec![root_commit.clone()]));

    // Commit 3 is the first bad commit
    let expected_tests = [
        (commit3.id(), Evaluation::Bad),
        (root_commit.id(), Evaluation::Good),
        (commit1.id(), Evaluation::Good),
    ];
    let result = test_bisection(tx.repo(), &input_range, expected_tests);
    assert_eq!(result, BisectionResult::Found(vec![commit3.clone()]));

    // Commit 4 is the first bad commit
    let expected_tests = [
        (commit3.id(), Evaluation::Good),
        (commit4.id(), Evaluation::Bad),
        (commit2.id(), Evaluation::Good),
    ];
    let result = test_bisection(tx.repo(), &input_range, expected_tests);
    assert_eq!(result, BisectionResult::Found(vec![commit4.clone()]));

    // Commit 6 is the first bad commit
    let expected_tests = [
        (commit3.id(), Evaluation::Good),
        (commit4.id(), Evaluation::Good),
        (commit5.id(), Evaluation::Good),
        (commit6.id(), Evaluation::Bad),
    ];
    let result = test_bisection(tx.repo(), &input_range, expected_tests);
    assert_eq!(result, BisectionResult::Found(vec![commit6.clone()]));
}

#[test]
fn test_bisect_disjoint_sets() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    // 1 2
    // |/
    // 0
    let mut tx = repo.start_transaction();
    let commit1 = write_random_commit(tx.repo_mut());
    let commit2 = write_random_commit(tx.repo_mut());

    let input_range =
        ResolvedRevsetExpression::commits(vec![commit1.id().clone(), commit2.id().clone()]);

    // Both commit 1 and commit 2 are (implicitly) the first bad commits
    let expected_tests = [];
    let result = test_bisection(tx.repo(), &input_range, expected_tests);
    assert_eq!(
        result,
        BisectionResult::Found(vec![commit2.clone(), commit1.clone()])
    );
}
