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

use jj_lib::conflict_labels::ConflictLabels;
use jj_lib::merge::Merge;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::Repo as _;
use jj_lib::treeset;
use pollster::FutureExt as _;
use testutils::TestRepo;
use testutils::TestResult;
use testutils::assert_tree_eq;
use testutils::create_single_tree;
use testutils::create_tree;
use testutils::repo_path;

#[test]
fn test_conflict_side() -> TestResult {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let conflicted = repo_path("conflicted");
    let modify_delete = repo_path("modify-delete");
    let only_side1 = repo_path("only-side1");
    let only_side2 = repo_path("only-side2");
    let added_side2 = repo_path("dir/added-side2");
    let base = create_single_tree(
        repo,
        &[
            (conflicted, "base"),
            (modify_delete, "base"),
            (only_side1, "base"),
            (only_side2, "base"),
        ],
    );
    let side1 = create_single_tree(
        repo,
        &[
            (conflicted, "side 1"),
            (only_side1, "side 1"),
            (only_side2, "base"),
        ],
    );
    let side2 = create_single_tree(
        repo,
        &[
            (conflicted, "side 2"),
            (modify_delete, "side 2"),
            (only_side1, "base"),
            (only_side2, "side 2"),
            (added_side2, "side 2"),
        ],
    );
    let tree = MergedTree::new(
        repo.store().clone(),
        Merge::from_vec(vec![
            side1.id().clone(),
            base.id().clone(),
            side2.id().clone(),
        ]),
        ConflictLabels::from_vec(vec!["side 1".into(), "base".into(), "side 2".into()]),
    );

    let expected_side1 = create_tree(
        repo,
        &[
            (conflicted, "side 1"),
            (only_side1, "side 1"),
            (only_side2, "side 2"),
            (added_side2, "side 2"),
        ],
    );
    let expected_side2 = create_tree(
        repo,
        &[
            (conflicted, "side 2"),
            (modify_delete, "side 2"),
            (only_side1, "side 1"),
            (only_side2, "side 2"),
            (added_side2, "side 2"),
        ],
    );
    assert_tree_eq!(
        treeset::conflict_side(&tree, 0).block_on()?.unwrap(),
        expected_side1
    );
    assert_tree_eq!(
        treeset::conflict_side(&tree, 1).block_on()?.unwrap(),
        expected_side2
    );
    assert!(treeset::conflict_side(&tree, 2).block_on()?.is_none());

    // A resolved tree has a single side
    assert_tree_eq!(
        treeset::conflict_side(&expected_side1, 0)
            .block_on()?
            .unwrap(),
        expected_side1
    );
    assert!(
        treeset::conflict_side(&expected_side1, 1)
            .block_on()?
            .is_none()
    );
    Ok(())
}
