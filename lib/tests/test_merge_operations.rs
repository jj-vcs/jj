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

use std::process::Command;

use jj_lib::op_store;
use jj_lib::op_store::RefTarget;
use jj_lib::operation::Operation;
use jj_lib::ref_name::WorkspaceName;
use pollster::FutureExt as _;
use testutils::CommitBuilderExt as _;
use testutils::TestRepo;
use testutils::TestResult;
use testutils::create_random_commit;
use testutils::write_random_commit;

#[test]
fn test_start_merge_operations() -> TestResult {
    let test_repo = TestRepo::init();
    let repo_loader = test_repo.repo.loader();

    let mut tx = test_repo.repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let repo = tx.commit("add commit").block_on()?;

    // Rewrite the commit and add a bookmark to the old commit concurrently
    let mut tx1 = repo.start_transaction();
    let commit_a2 = tx1
        .repo_mut()
        .rewrite_commit(&commit_a)
        .set_description("rewritten")
        .write_unwrap();
    tx1.repo_mut().rebase_descendants().block_on()?;
    let mut tx2 = repo.start_transaction();
    tx2.repo_mut()
        .set_local_bookmark_target("main".as_ref(), RefTarget::normal(commit_a.id().clone()));
    let op1 = tx1.write("rewrite").block_on()?.leave_unpublished();
    let op2 = tx2.write("bookmark").block_on()?.leave_unpublished();
    let op1 = op1.operation().clone();
    let op2 = op2.operation().clone();

    let (mut tx, num_rebased) = repo_loader
        .start_merge_operations(vec![op1.clone(), op2.clone()], None, Some("merge"), [])
        .block_on()?;
    assert_eq!(num_rebased, 0);
    assert_eq!(tx.parent_ops(), [op1.clone(), op2.clone()]);
    // The bookmark should have been moved to the rewritten commit
    assert_eq!(
        *tx.repo().get_local_bookmark("main".as_ref()),
        RefTarget::normal(commit_a2.id().clone())
    );

    // The caller can make further changes as part of the merge operation
    tx.repo_mut()
        .set_local_bookmark_target("other".as_ref(), RefTarget::normal(commit_a2.id().clone()));
    let merged_repo = tx.write("merge").block_on()?.leave_unpublished();
    assert_eq!(
        merged_repo.operation().parent_ids(),
        [op1.id().clone(), op2.id().clone()]
    );
    assert_eq!(
        *merged_repo.view().get_local_bookmark("other".as_ref()),
        RefTarget::normal(commit_a2.id().clone())
    );
    Ok(())
}

// This test is about checking that we don't run out of stack space.
#[test]
fn test_merge_operations_deep_criss_cross() -> TestResult {
    const CHILD_ENV: &str = "JJ_LIB_TEST_MERGE_OPS_DEEP_CRISS_CROSS_CHILD";
    const LEVELS_ENV: &str = "JJ_LIB_TEST_MERGE_OPS_DEEP_CRISS_CROSS_LEVELS";
    const CHILD_STACK_SIZE: usize = 512 * 1024;
    const DEFAULT_NUM_LEVELS: usize = 14;

    if std::env::var_os(CHILD_ENV).is_some() {
        let num_levels = std::env::var(LEVELS_ENV)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT_NUM_LEVELS);
        return run_merge_operations_deep_criss_cross(num_levels);
    }

    let num_levels = std::env::var(LEVELS_ENV)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_NUM_LEVELS);
    let output = Command::new(std::env::current_exe()?)
        .arg("--exact")
        .arg("test_merge_operations::test_merge_operations_deep_criss_cross")
        .arg("--nocapture")
        .env(CHILD_ENV, "1")
        .env("RUST_MIN_STACK", CHILD_STACK_SIZE.to_string())
        .output()?;
    assert!(
        output.status.success(),
        "deep criss-cross merge failed with {num_levels} levels and {CHILD_STACK_SIZE} byte child \
         stack\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    Ok(())
}

fn run_merge_operations_deep_criss_cross(num_levels: usize) -> TestResult {
    let test_repo = TestRepo::init();
    let repo_loader = test_repo.repo.loader();

    let mut tx_a = test_repo.repo.start_transaction();
    let commit_k = write_random_commit(tx_a.repo_mut());
    tx_a.repo_mut()
        .set_wc_commit(WorkspaceName::DEFAULT.to_owned(), commit_k.id().clone())?;
    let repo_a = tx_a.commit("K").block_on()?;

    let mut tx_b = repo_a.start_transaction();
    let _commit_k2 = tx_b
        .repo_mut()
        .rewrite_commit(&commit_k)
        .set_description("K2")
        .write_unwrap();
    tx_b.repo_mut().rebase_descendants().block_on()?;

    let mut tx_c = repo_a.start_transaction();
    let commit_l = create_random_commit(tx_c.repo_mut())
        .set_description("L")
        .set_parents(vec![commit_k.id().clone()])
        .write_unwrap();
    tx_c.repo_mut()
        .set_wc_commit(WorkspaceName::DEFAULT.to_owned(), commit_l.id().clone())?;

    let repo_b = tx_b.commit("B").block_on()?;
    let repo_c = tx_c.commit("C").block_on()?;

    let mut left_op = repo_b.operation().clone();
    let mut right_op = repo_c.operation().clone();
    let op_store = repo_loader.op_store();
    for _ in 0..num_levels {
        let next_left_op = op_store::Operation {
            view_id: left_op.view_id().clone(),
            metadata: left_op.metadata().clone(),
            parents: vec![left_op.id().clone(), right_op.id().clone()],
            commit_predecessors: None,
        };
        let next_left_op_id = op_store.write_operation(&next_left_op).block_on()?;

        let next_right_op = op_store::Operation {
            view_id: right_op.view_id().clone(),
            metadata: right_op.metadata().clone(),
            parents: vec![left_op.id().clone(), right_op.id().clone()],
            commit_predecessors: None,
        };
        let next_right_op_id = op_store.write_operation(&next_right_op).block_on()?;

        left_op = Operation::new(op_store.clone(), next_left_op_id, next_left_op);
        right_op = Operation::new(op_store.clone(), next_right_op_id, next_right_op);
    }

    let workspace_name = None;
    let transaction_attributes = [];
    let (merged_repo, _num_rebased) = repo_loader
        .merge_operations(
            vec![left_op.clone(), right_op.clone()],
            workspace_name,
            Some("merge deep criss-cross"),
            transaction_attributes,
        )
        .block_on()?;
    assert!(!merged_repo.view().heads().is_empty());
    Ok(())
}
