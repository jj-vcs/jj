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

use itertools::Itertools as _;
use jj_core::op_store;
use jj_core::operation::Operation;
use jj_core::ref_name::WorkspaceName;
use jj_lib::repo::Repo as _;
use pollster::FutureExt as _;
use testutils::CommitBuilderExt as _;
use testutils::TestRepo;
use testutils::TestResult;
use testutils::commit_transactions;
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

#[test]
fn test_concurrent_rebases_form_rewrite_cycle() -> TestResult {
    let test_repo = TestRepo::init();
    let mut tx = test_repo.repo.start_transaction();
    // Y
    // |
    // X  Z
    let commit_x = create_random_commit(tx.repo_mut())
        .set_description("X")
        .write_unwrap();
    let commit_y = create_random_commit(tx.repo_mut())
        .set_description("Y")
        .set_parents(vec![commit_x.id().clone()])
        .write_unwrap();
    let commit_z = create_random_commit(tx.repo_mut())
        .set_description("Z")
        .write_unwrap();
    let base = tx.commit("base").block_on()?;

    // Concurrently rebase X and its descendant onto Z, and Z onto Y.
    let mut tx1 = base.start_transaction();
    let commit_x1 = tx1
        .repo_mut()
        .rewrite_commit(&commit_x)
        .set_parents(vec![commit_z.id().clone()])
        .write_unwrap();
    tx1.repo_mut().rebase_descendants().block_on()?;

    let mut tx2 = base.start_transaction();
    tx2.repo_mut()
        .rewrite_commit(&commit_z)
        .set_parents(vec![commit_y.id().clone()])
        .write_unwrap();
    tx2.repo_mut().rebase_descendants().block_on()?;

    //     Z''
    //     |
    // Y'  Y''
    //  \ /
    //   X'
    //   |
    //   Z
    let repo = commit_transactions(vec![tx1, tx2]);
    assert_eq!(repo.view().heads().len(), 2);
    let head_commits: Vec<_> = repo
        .view()
        .heads()
        .iter()
        .map(|id| repo.store().get_commit(id))
        .try_collect()?;
    let commit_z2 = head_commits
        .iter()
        .find(|c| c.change_id() == commit_z.change_id())
        .unwrap();
    let commit_y1 = head_commits
        .iter()
        .find(|c| c.change_id() == commit_y.change_id())
        .unwrap();
    assert_eq!(commit_z2.parent_ids().len(), 1);
    let commit_y2 = commit_z2.parents().block_on()?[0].clone();
    assert_eq!(commit_y2.change_id(), commit_y.change_id());
    assert_eq!(commit_y2.parent_ids(), &[commit_x1.id().clone()]);
    assert_eq!(commit_y1.parent_ids(), &[commit_x1.id().clone()]);
    assert_eq!(commit_x1.parent_ids(), &[commit_z.id().clone()]);
    Ok(())
}
