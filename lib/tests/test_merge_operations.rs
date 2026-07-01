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
use std::time::Duration;

use jj_lib::ref_name::WorkspaceName;
use jj_lib::transaction::Transaction;
use pollster::FutureExt as _;
use testutils::CommitBuilderExt as _;
use testutils::TestRepo;
use testutils::TestResult;
use testutils::create_random_commit;
use testutils::write_random_commit;

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
    std::thread::sleep(Duration::from_millis(1));
    let repo_c = tx_c.commit("C").block_on()?;

    let mut left_repo = repo_b;
    let mut right_repo = repo_c;
    let workspace_name = None;
    let transaction_attributes = [];
    for _ in 0..num_levels {
        let left_op = left_repo.operation().clone();
        let right_op = right_repo.operation().clone();

        let (next_left_repo, _num_rebased) = Transaction::merge_operations(
            left_repo.loader(),
            vec![left_op.clone(), right_op.clone()],
            workspace_name,
            Some("merge left"),
            transaction_attributes.clone(),
        )
        .block_on()?;

        std::thread::sleep(Duration::from_millis(1));

        let (next_right_repo, _num_rebased) = Transaction::merge_operations(
            left_repo.loader(),
            vec![left_op, right_op],
            workspace_name,
            Some("merge right"),
            transaction_attributes.clone(),
        )
        .block_on()?;

        left_repo = next_left_repo;
        right_repo = next_right_repo;
    }

    let (merged_repo, _num_rebased) = Transaction::merge_operations(
        left_repo.loader(),
        vec![
            left_repo.operation().clone(),
            right_repo.operation().clone(),
        ],
        workspace_name,
        Some("merge deep criss-cross"),
        transaction_attributes,
    )
    .block_on()?;
    assert!(!merged_repo.view().heads().is_empty());
    Ok(())
}
