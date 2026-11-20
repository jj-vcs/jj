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

use jj_core::op_store;
use jj_core::operation::Operation;
use jj_core::ref_name::WorkspaceName;
use jj_lib::repo::RepoLoaderError;
use jj_lib::rewrite::rebase_commit;
use pollster::FutureExt as _;
use testutils::CommitBuilderExt as _;
use testutils::TestRepo;
use testutils::TestResult;
use testutils::create_random_commit;
use testutils::write_random_commit;

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

// Regression test for https://github.com/jj-vcs/jj/issues/10474: merging
// concurrent operations that rebase commits onto each other's descendants
// produced a cyclic rewrite mapping and panicked in
// `order_commits_for_rebase()`.
#[test]
fn test_merge_operations_cyclic_rebase() -> TestResult {
    let test_repo = TestRepo::init();
    let repo_loader = test_repo.repo.loader();

    let mut tx = test_repo.repo.start_transaction();
    let commit_x = write_random_commit(tx.repo_mut());
    let commit_y = create_random_commit(tx.repo_mut())
        .set_parents(vec![commit_x.id().clone()])
        .write_unwrap();
    let commit_z = write_random_commit(tx.repo_mut());
    let base_repo = tx.commit("base").block_on()?;

    // Op A rebases the X-Y chain onto Z.
    let mut tx_a = base_repo.start_transaction();
    rebase_commit(
        tx_a.repo_mut(),
        commit_x.clone(),
        vec![commit_z.id().clone()],
    )
    .block_on()?;
    tx_a.repo_mut().rebase_descendants().block_on()?;
    let repo_a = tx_a.commit("A").block_on()?;

    // Op B rebases Z onto Y.
    let mut tx_b = base_repo.start_transaction();
    rebase_commit(
        tx_b.repo_mut(),
        commit_z.clone(),
        vec![commit_y.id().clone()],
    )
    .block_on()?;
    tx_b.repo_mut().rebase_descendants().block_on()?;
    let repo_b = tx_b.commit("B").block_on()?;

    // Merging the operations creates a rewrite cycle X -> Z -> Y -> X'. It
    // should surface as an error rather than panic.
    let workspace_name = None;
    let transaction_attributes = [];
    let result = repo_loader
        .merge_operations(
            vec![repo_a.operation().clone(), repo_b.operation().clone()],
            workspace_name,
            Some("merge cyclic rebases"),
            transaction_attributes,
        )
        .block_on();
    assert!(
        matches!(result, Err(RepoLoaderError::Backend(ref err))
            if err.to_string().contains("Cycle between rewritten commits")),
        "{result:?}"
    );
    Ok(())
}
