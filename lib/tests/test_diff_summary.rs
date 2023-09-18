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

use jj_lib::matchers::{EverythingMatcher, FilesMatcher};
use jj_lib::merged_tree::DiffSummary;
use jj_lib::repo_path::RepoPath;
use test_case::test_case;
use testutils::{create_tree, TestRepo, TestRepoBackend};

#[test_case(TestRepoBackend::Local ; "local backend")]
#[test_case(TestRepoBackend::Git ; "git backend")]
fn test_types(backend: TestRepoBackend) {
    let test_repo = TestRepo::init_with_backend(backend);
    let repo = &test_repo.repo;

    let clean_path = RepoPath::from_internal_string("clean");
    let modified_path = RepoPath::from_internal_string("modified");
    let added_path = RepoPath::from_internal_string("added");
    let removed_path = RepoPath::from_internal_string("removed");

    let tree1 = create_tree(
        repo,
        &[
            (&clean_path, "clean"),
            (&modified_path, "contents before"),
            (&removed_path, "removed contents"),
        ],
    );

    let tree2 = create_tree(
        repo,
        &[
            (&clean_path, "clean"),
            (&modified_path, "contents after"),
            (&added_path, "added contents"),
        ],
    );

    assert_eq!(
        tree1.diff_summary(&tree2, &EverythingMatcher),
        DiffSummary {
            modified: vec![modified_path],
            added: vec![added_path],
            removed: vec![removed_path]
        }
    );
}

#[test_case(TestRepoBackend::Local ; "local backend")]
#[test_case(TestRepoBackend::Git ; "git backend")]
fn test_tree_file_transition(backend: TestRepoBackend) {
    let test_repo = TestRepo::init_with_backend(backend);
    let repo = &test_repo.repo;

    let dir_file_path = RepoPath::from_internal_string("dir/file");
    let dir_path = RepoPath::from_internal_string("dir");

    let tree1 = create_tree(repo, &[(&dir_file_path, "contents")]);
    let tree2 = create_tree(repo, &[(&dir_path, "contents")]);

    assert_eq!(
        tree1.diff_summary(&tree2, &EverythingMatcher),
        DiffSummary {
            modified: vec![],
            added: vec![dir_path.clone()],
            removed: vec![dir_file_path.clone()]
        }
    );
    assert_eq!(
        tree2.diff_summary(&tree1, &EverythingMatcher),
        DiffSummary {
            modified: vec![],
            added: vec![dir_file_path],
            removed: vec![dir_path]
        }
    );
}

#[test_case(TestRepoBackend::Local ; "local backend")]
#[test_case(TestRepoBackend::Git ; "git backend")]
fn test_sorting(backend: TestRepoBackend) {
    let test_repo = TestRepo::init_with_backend(backend);
    let repo = &test_repo.repo;

    let a_path = RepoPath::from_internal_string("a");
    let b_path = RepoPath::from_internal_string("b");
    let f_a_path = RepoPath::from_internal_string("f/a");
    let f_b_path = RepoPath::from_internal_string("f/b");
    let f_f_a_path = RepoPath::from_internal_string("f/f/a");
    let f_f_b_path = RepoPath::from_internal_string("f/f/b");
    let n_path = RepoPath::from_internal_string("n");
    let s_b_path = RepoPath::from_internal_string("s/b");
    let z_path = RepoPath::from_internal_string("z");

    let tree1 = create_tree(
        repo,
        &[
            (&a_path, "before"),
            (&f_a_path, "before"),
            (&f_f_a_path, "before"),
        ],
    );

    let tree2 = create_tree(
        repo,
        &[
            (&a_path, "after"),
            (&b_path, "after"),
            (&f_a_path, "after"),
            (&f_b_path, "after"),
            (&f_f_a_path, "after"),
            (&f_f_b_path, "after"),
            (&n_path, "after"),
            (&s_b_path, "after"),
            (&z_path, "after"),
        ],
    );

    assert_eq!(
        tree1.diff_summary(&tree2, &EverythingMatcher),
        DiffSummary {
            modified: vec![a_path.clone(), f_a_path.clone(), f_f_a_path.clone()],
            added: vec![
                b_path.clone(),
                f_b_path.clone(),
                f_f_b_path.clone(),
                n_path.clone(),
                s_b_path.clone(),
                z_path.clone(),
            ],
            removed: vec![]
        }
    );
    assert_eq!(
        tree2.diff_summary(&tree1, &EverythingMatcher),
        DiffSummary {
            modified: vec![a_path, f_a_path, f_f_a_path],
            added: vec![],
            removed: vec![b_path, f_b_path, f_f_b_path, n_path, s_b_path, z_path]
        }
    );
}

#[test_case(TestRepoBackend::Local ; "local backend")]
#[test_case(TestRepoBackend::Git ; "git backend")]
fn test_matcher_dir_file_transition(backend: TestRepoBackend) {
    let test_repo = TestRepo::init_with_backend(backend);
    let repo = &test_repo.repo;

    let a_path = RepoPath::from_internal_string("a");
    let a_a_path = RepoPath::from_internal_string("a/a");

    let tree1 = create_tree(repo, &[(&a_path, "before")]);
    let tree2 = create_tree(repo, &[(&a_a_path, "after")]);

    let matcher = FilesMatcher::new(&[a_path.clone()]);
    assert_eq!(
        tree1.diff_summary(&tree2, &matcher),
        DiffSummary {
            modified: vec![],
            added: vec![],
            removed: vec![a_path.clone()]
        }
    );
    assert_eq!(
        tree2.diff_summary(&tree1, &matcher),
        DiffSummary {
            modified: vec![],
            added: vec![a_path.clone()],
            removed: vec![]
        }
    );

    let matcher = FilesMatcher::new(&[a_a_path.clone()]);
    assert_eq!(
        tree1.diff_summary(&tree2, &matcher),
        DiffSummary {
            modified: vec![],
            added: vec![a_a_path.clone()],
            removed: vec![]
        }
    );
    assert_eq!(
        tree2.diff_summary(&tree1, &matcher),
        DiffSummary {
            modified: vec![],
            added: vec![],
            removed: vec![a_a_path.clone()]
        }
    );

    let matcher = FilesMatcher::new(&[a_path.clone(), a_a_path.clone()]);
    assert_eq!(
        tree1.diff_summary(&tree2, &matcher),
        DiffSummary {
            modified: vec![],
            added: vec![a_a_path.clone()],
            removed: vec![a_path.clone()]
        }
    );
    assert_eq!(
        tree2.diff_summary(&tree1, &matcher),
        DiffSummary {
            modified: vec![],
            added: vec![a_path],
            removed: vec![a_a_path]
        }
    );
}

#[test_case(TestRepoBackend::Local ; "local backend")]
#[test_case(TestRepoBackend::Git ; "git backend")]
fn test_matcher_normal_cases(backend: TestRepoBackend) {
    let test_repo = TestRepo::init_with_backend(backend);
    let repo = &test_repo.repo;

    let a_path = RepoPath::from_internal_string("a");
    let dir1_a_path = RepoPath::from_internal_string("dir1/a");
    let dir2_b_path = RepoPath::from_internal_string("dir2/b");
    let z_path = RepoPath::from_internal_string("z");

    let tree1 = create_tree(repo, &[(&a_path, "before"), (&dir1_a_path, "before")]);
    // File "a" gets modified
    // File "dir1/a" gets modified
    // File "dir2/b" gets created
    // File "z" gets created
    let tree2 = create_tree(
        repo,
        &[
            (&a_path, "after"),
            (&dir1_a_path, "after"),
            (&dir2_b_path, "after"),
            (&z_path, "after"),
        ],
    );

    let matcher = FilesMatcher::new(&[a_path.clone(), z_path.clone()]);
    assert_eq!(
        tree1.diff_summary(&tree2, &matcher),
        DiffSummary {
            modified: vec![a_path.clone()],
            added: vec![z_path.clone()],
            removed: vec![]
        }
    );
    assert_eq!(
        tree2.diff_summary(&tree1, &matcher),
        DiffSummary {
            modified: vec![a_path],
            added: vec![],
            removed: vec![z_path]
        }
    );

    let matcher = FilesMatcher::new(&[dir1_a_path.clone(), dir2_b_path.clone()]);
    assert_eq!(
        tree1.diff_summary(&tree2, &matcher),
        DiffSummary {
            modified: vec![dir1_a_path.clone()],
            added: vec![dir2_b_path.clone()],
            removed: vec![]
        }
    );
    assert_eq!(
        tree2.diff_summary(&tree1, &matcher),
        DiffSummary {
            modified: vec![dir1_a_path],
            added: vec![],
            removed: vec![dir2_b_path]
        }
    );
}
