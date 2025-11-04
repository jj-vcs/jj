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

use std::fs::File;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;

use assert_matches::assert_matches;
use indoc::indoc;
use itertools::Itertools as _;
use jj_lib::backend::CopyId;
use jj_lib::backend::TreeId;
use jj_lib::backend::TreeValue;
use jj_lib::file_util::check_symlink_support;
use jj_lib::file_util::try_symlink;
use jj_lib::fsmonitor::FsmonitorSettings;
use jj_lib::gitignore::GitIgnoreFile;
use jj_lib::local_working_copy::LocalWorkingCopy;
use jj_lib::local_working_copy::TreeState;
use jj_lib::local_working_copy::TreeStateSettings;
use jj_lib::merge::Merge;
use jj_lib::merge::MergedTreeValue;
use jj_lib::merged_tree::MergedTree;
use jj_lib::merged_tree::MergedTreeBuilder;
use jj_lib::op_store::OperationId;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::secret_backend::SecretBackend;
use jj_lib::tree_builder::TreeBuilder;
use jj_lib::working_copy::CheckoutError;
use jj_lib::working_copy::CheckoutStats;
use jj_lib::working_copy::SnapshotOptions;
use jj_lib::working_copy::UntrackedReason;
use jj_lib::working_copy::WorkingCopy as _;
use jj_lib::workspace::Workspace;
use jj_lib::workspace::default_working_copy_factories;
use pollster::FutureExt as _;
use test_case::test_case;
use testutils::TestRepo;
use testutils::TestRepoBackend;
use testutils::TestWorkspace;
use testutils::assert_tree_eq;
use testutils::commit_with_tree;
use testutils::create_tree;
use testutils::create_tree_with;
use testutils::empty_snapshot_options;
use testutils::repo_path;
use testutils::repo_path_buf;
use testutils::repo_path_component;
use testutils::write_random_commit;

fn check_icase_fs(dir: &Path) -> bool {
    let test_file = tempfile::Builder::new()
        .prefix("icase-")
        .tempfile_in(dir)
        .unwrap();
    let orig_name = test_file.path().file_name().unwrap().to_str().unwrap();
    let upper_name = orig_name.to_ascii_uppercase();
    assert_ne!(orig_name, upper_name);
    dir.join(upper_name).try_exists().unwrap()
}

/// Returns true if the directory appears to ignore some unicode zero-width
/// characters, as in HFS+.
fn check_hfs_plus(dir: &Path) -> bool {
    let test_file = tempfile::Builder::new()
        .prefix("hfs-plus-\u{200c}-")
        .tempfile_in(dir)
        .unwrap();
    let orig_name = test_file.path().file_name().unwrap().to_str().unwrap();
    let stripped_name = orig_name.replace('\u{200c}', "");
    assert_ne!(orig_name, stripped_name);
    dir.join(stripped_name).try_exists().unwrap()
}

/// Returns true if the directory appears to support Windows short file names.
fn check_vfat(dir: &Path) -> bool {
    let _test_file = tempfile::Builder::new()
        .prefix("vfattest-")
        .tempfile_in(dir)
        .unwrap();
    let short_name = "VFATTE~1";
    dir.join(short_name).try_exists().unwrap()
}

fn to_owned_path_vec(paths: &[&RepoPath]) -> Vec<RepoPathBuf> {
    paths.iter().map(|&path| path.to_owned()).collect()
}

fn tree_entries(tree: &MergedTree) -> Vec<(RepoPathBuf, Option<MergedTreeValue>)> {
    tree.entries()
        .map(|(path, result)| (path, result.ok()))
        .collect_vec()
}

#[test]
fn test_root() {
    // Test that the working copy is clean and empty after init.
    let mut test_workspace = TestWorkspace::init();

    let wc = test_workspace.workspace.working_copy();
    assert_eq!(wc.sparse_patterns().unwrap(), vec![RepoPathBuf::root()]);
    let new_tree = test_workspace.snapshot().unwrap();
    let repo = &test_workspace.repo;
    let wc_commit_id = repo
        .view()
        .get_wc_commit_id(WorkspaceName::DEFAULT)
        .unwrap();
    let wc_commit = repo.store().get_commit(wc_commit_id).unwrap();
    assert_tree_eq!(new_tree, wc_commit.tree());
    assert_tree_eq!(new_tree, repo.store().empty_merged_tree());
}

#[test_case(TestRepoBackend::Simple ; "simple backend")]
#[test_case(TestRepoBackend::Git ; "git backend")]
fn test_checkout_file_transitions(backend: TestRepoBackend) {
    // Tests switching between commits where a certain path is of one type in one
    // commit and another type in the other. Includes a "missing" type, so we cover
    // additions and removals as well.

    let mut test_workspace = TestWorkspace::init_with_backend(backend);
    let repo = &test_workspace.repo;
    let store = repo.store().clone();
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();

    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    enum Kind {
        Missing,
        Normal,
        Executable,
        // Executable, but same content as Normal, to test transition where only the bit changed
        ExecutableNormalContent,
        Conflict,
        // Same content as Executable, to test that transition preserves the executable bit
        ConflictedExecutableContent,
        Symlink,
        Tree,
        GitSubmodule,
    }

    fn write_path(
        repo: &Arc<ReadonlyRepo>,
        tree_builder: &mut MergedTreeBuilder,
        kind: Kind,
        path: &RepoPath,
    ) {
        let store = repo.store();
        let copy_id = CopyId::placeholder();
        let value = match kind {
            Kind::Missing => Merge::absent(),
            Kind::Normal => {
                let id = testutils::write_file(store, path, "normal file contents");
                Merge::normal(TreeValue::File {
                    id,
                    executable: false,
                    copy_id,
                })
            }
            Kind::Executable => {
                let id: jj_lib::backend::FileId =
                    testutils::write_file(store, path, "executable file contents");
                Merge::normal(TreeValue::File {
                    id,
                    executable: true,
                    copy_id,
                })
            }
            Kind::ExecutableNormalContent => {
                let id = testutils::write_file(store, path, "normal file contents");
                Merge::normal(TreeValue::File {
                    id,
                    executable: true,
                    copy_id,
                })
            }
            Kind::Conflict => {
                let base_file_id = testutils::write_file(store, path, "base file contents");
                let left_file_id = testutils::write_file(store, path, "left file contents");
                let right_file_id = testutils::write_file(store, path, "right file contents");
                Merge::from_removes_adds(
                    vec![Some(TreeValue::File {
                        id: base_file_id,
                        executable: false,
                        copy_id: copy_id.clone(),
                    })],
                    vec![
                        Some(TreeValue::File {
                            id: left_file_id,
                            executable: false,
                            copy_id: copy_id.clone(),
                        }),
                        Some(TreeValue::File {
                            id: right_file_id,
                            executable: false,
                            copy_id: copy_id.clone(),
                        }),
                    ],
                )
            }
            Kind::ConflictedExecutableContent => {
                let base_file_id = testutils::write_file(store, path, "executable file contents");
                let left_file_id =
                    testutils::write_file(store, path, "left executable file contents");
                let right_file_id =
                    testutils::write_file(store, path, "right executable file contents");
                Merge::from_removes_adds(
                    vec![Some(TreeValue::File {
                        id: base_file_id,
                        executable: true,
                        copy_id: copy_id.clone(),
                    })],
                    vec![
                        Some(TreeValue::File {
                            id: left_file_id,
                            executable: true,
                            copy_id: copy_id.clone(),
                        }),
                        Some(TreeValue::File {
                            id: right_file_id,
                            executable: true,
                            copy_id: copy_id.clone(),
                        }),
                    ],
                )
            }
            Kind::Symlink => {
                let id = store.write_symlink(path, "target").block_on().unwrap();
                Merge::normal(TreeValue::Symlink(id))
            }
            Kind::Tree => {
                let file_path = path.join(repo_path_component("file"));
                let id = testutils::write_file(store, &file_path, "normal file contents");
                let value = TreeValue::File {
                    id,
                    executable: false,
                    copy_id: copy_id.clone(),
                };
                tree_builder.set_or_remove(file_path, Merge::normal(value));
                return;
            }
            Kind::GitSubmodule => {
                let mut tx = repo.start_transaction();
                let id = write_random_commit(tx.repo_mut()).id().clone();
                tx.commit("test").unwrap();
                Merge::normal(TreeValue::GitSubmodule(id))
            }
        };
        tree_builder.set_or_remove(path.to_owned(), value);
    }

    let mut kinds = vec![
        Kind::Missing,
        Kind::Normal,
        Kind::Executable,
        Kind::ExecutableNormalContent,
        Kind::Conflict,
        Kind::ConflictedExecutableContent,
        Kind::Tree,
    ];
    kinds.push(Kind::Symlink);
    if backend == TestRepoBackend::Git {
        kinds.push(Kind::GitSubmodule);
    }
    let mut left_tree_builder = MergedTreeBuilder::new(store.empty_merged_tree());
    let mut right_tree_builder = MergedTreeBuilder::new(store.empty_merged_tree());
    let mut files = vec![];
    for left_kind in &kinds {
        for right_kind in &kinds {
            let path = repo_path_buf(format!("{left_kind:?}_{right_kind:?}"));
            write_path(repo, &mut left_tree_builder, *left_kind, &path);
            write_path(repo, &mut right_tree_builder, *right_kind, &path);
            files.push((*left_kind, *right_kind, path.clone()));
        }
    }
    let left_tree = left_tree_builder.write_tree().unwrap();
    let right_tree = right_tree_builder.write_tree().unwrap();
    let left_commit = commit_with_tree(&store, left_tree);
    let right_commit = commit_with_tree(&store, right_tree.clone());

    let ws = &mut test_workspace.workspace;
    ws.check_out(repo.op_id().clone(), None, &left_commit)
        .unwrap();
    ws.check_out(repo.op_id().clone(), None, &right_commit)
        .unwrap();

    // Check that the working copy is clean.
    let new_tree = test_workspace.snapshot().unwrap();
    assert_tree_eq!(new_tree, right_tree);

    for (_left_kind, right_kind, path) in &files {
        let wc_path = workspace_root.join(path.as_internal_file_string());
        let maybe_metadata = wc_path.symlink_metadata();
        match right_kind {
            Kind::Missing => {
                assert!(maybe_metadata.is_err(), "{path:?} should not exist");
            }
            Kind::Normal => {
                assert!(maybe_metadata.is_ok(), "{path:?} should exist");
                let metadata = maybe_metadata.unwrap();
                assert!(metadata.is_file(), "{path:?} should be a file");
                #[cfg(unix)]
                assert_eq!(
                    metadata.permissions().mode() & 0o111,
                    0,
                    "{path:?} should not be executable"
                );
            }
            Kind::Executable | Kind::ExecutableNormalContent => {
                assert!(maybe_metadata.is_ok(), "{path:?} should exist");
                let metadata = maybe_metadata.unwrap();
                assert!(metadata.is_file(), "{path:?} should be a file");
                #[cfg(unix)]
                assert_ne!(
                    metadata.permissions().mode() & 0o111,
                    0,
                    "{path:?} should be executable"
                );
            }
            Kind::Conflict => {
                assert!(maybe_metadata.is_ok(), "{path:?} should exist");
                let metadata = maybe_metadata.unwrap();
                assert!(metadata.is_file(), "{path:?} should be a file");
                #[cfg(unix)]
                assert_eq!(
                    metadata.permissions().mode() & 0o111,
                    0,
                    "{path:?} should not be executable"
                );
            }
            Kind::ConflictedExecutableContent => {
                assert!(maybe_metadata.is_ok(), "{path:?} should exist");
                let metadata = maybe_metadata.unwrap();
                assert!(metadata.is_file(), "{path:?} should be a file");
                #[cfg(unix)]
                assert_ne!(
                    metadata.permissions().mode() & 0o111,
                    0,
                    "{path:?} should be executable"
                );
            }
            Kind::Symlink => {
                assert!(maybe_metadata.is_ok(), "{path:?} should exist");
                let metadata = maybe_metadata.unwrap();
                if check_symlink_support().unwrap_or(false) {
                    assert!(
                        metadata.file_type().is_symlink(),
                        "{path:?} should be a symlink"
                    );
                }
            }
            Kind::Tree => {
                assert!(maybe_metadata.is_ok(), "{path:?} should exist");
                let metadata = maybe_metadata.unwrap();
                assert!(metadata.is_dir(), "{path:?} should be a directory");
            }
            Kind::GitSubmodule => {
                // Not supported for now
                assert!(maybe_metadata.is_err(), "{path:?} should not exist");
            }
        };
    }
}

#[test]
fn test_checkout_no_op() {
    // Check out another commit with the same tree that's already checked out. The
    // recorded operation should be updated even though the tree is unchanged.
    let mut test_workspace = TestWorkspace::init();
    let repo = test_workspace.repo.clone();

    let file_path = repo_path("file");

    let tree = create_tree(&repo, &[(file_path, "contents")]);
    let commit1 = commit_with_tree(repo.store(), tree.clone());
    let commit2 = commit_with_tree(repo.store(), tree);

    let ws = &mut test_workspace.workspace;
    ws.check_out(repo.op_id().clone(), None, &commit1).unwrap();

    // Test the setup: the file should exist on in the tree state.
    let wc: &LocalWorkingCopy = ws.working_copy().downcast_ref().unwrap();
    assert!(wc.file_states().unwrap().contains_path(file_path));

    // Update to commit2 (same tree as commit1)
    let new_op_id = OperationId::from_bytes(b"whatever");
    let stats = ws.check_out(new_op_id.clone(), None, &commit2).unwrap();
    assert_eq!(stats, CheckoutStats::default());

    // The tree state is unchanged but the recorded operation id is updated.
    let wc: &LocalWorkingCopy = ws.working_copy().downcast_ref().unwrap();
    assert!(wc.file_states().unwrap().contains_path(file_path));
    assert_eq!(*wc.operation_id(), new_op_id);
}

// Test case for issue #2165
#[test]
fn test_conflict_subdirectory() {
    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;

    let path = repo_path("sub/file");
    let empty_tree = create_tree(repo, &[]);
    let tree1 = create_tree(repo, &[(path, "0")]);
    let commit1 = commit_with_tree(repo.store(), tree1.clone());
    let tree2 = create_tree(repo, &[(path, "1")]);
    let merged_tree = tree1.merge(empty_tree, tree2).block_on().unwrap();
    let merged_commit = commit_with_tree(repo.store(), merged_tree);
    let repo = &test_workspace.repo;
    let ws = &mut test_workspace.workspace;
    ws.check_out(repo.op_id().clone(), None, &commit1).unwrap();
    ws.check_out(repo.op_id().clone(), None, &merged_commit)
        .unwrap();
}

#[test]
fn test_acl() {
    let settings = testutils::user_settings();
    let test_workspace =
        TestWorkspace::init_with_backend_and_settings(TestRepoBackend::Git, &settings);
    let repo = &test_workspace.repo;
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();

    let secret_modified_path = repo_path("secret/modified");
    let secret_added_path = repo_path("secret/added");
    let secret_deleted_path = repo_path("secret/deleted");
    let became_secret_path = repo_path("file1");
    let became_public_path = repo_path("file2");
    let tree1 = create_tree(
        repo,
        &[
            (secret_modified_path, "0"),
            (secret_deleted_path, "0"),
            (became_secret_path, "public"),
            (became_public_path, "secret"),
        ],
    );
    let tree2 = create_tree(
        repo,
        &[
            (secret_modified_path, "1"),
            (secret_added_path, "1"),
            (became_secret_path, "secret"),
            (became_public_path, "public"),
        ],
    );
    let commit1 = commit_with_tree(repo.store(), tree1);
    let commit2 = commit_with_tree(repo.store(), tree2);
    SecretBackend::adopt_git_repo(&workspace_root);

    let mut ws = Workspace::load(
        &settings,
        &workspace_root,
        &test_workspace.env.default_store_factories(),
        &default_working_copy_factories(),
    )
    .unwrap();
    // Reload commits from the store associated with the workspace
    let repo = ws.repo_loader().load_at(repo.operation()).unwrap();
    let commit1 = repo.store().get_commit(commit1.id()).unwrap();
    let commit2 = repo.store().get_commit(commit2.id()).unwrap();

    ws.check_out(repo.op_id().clone(), None, &commit1).unwrap();
    assert!(
        !secret_modified_path
            .to_fs_path_unchecked(&workspace_root)
            .is_file()
    );
    assert!(
        !secret_added_path
            .to_fs_path_unchecked(&workspace_root)
            .is_file()
    );
    assert!(
        !secret_deleted_path
            .to_fs_path_unchecked(&workspace_root)
            .is_file()
    );
    assert!(
        became_secret_path
            .to_fs_path_unchecked(&workspace_root)
            .is_file()
    );
    assert!(
        !became_public_path
            .to_fs_path_unchecked(&workspace_root)
            .is_file()
    );
    ws.check_out(repo.op_id().clone(), None, &commit2).unwrap();
    assert!(
        !secret_modified_path
            .to_fs_path_unchecked(&workspace_root)
            .is_file()
    );
    assert!(
        !secret_added_path
            .to_fs_path_unchecked(&workspace_root)
            .is_file()
    );
    assert!(
        !secret_deleted_path
            .to_fs_path_unchecked(&workspace_root)
            .is_file()
    );
    assert!(
        !became_secret_path
            .to_fs_path_unchecked(&workspace_root)
            .is_file()
    );
    assert!(
        became_public_path
            .to_fs_path_unchecked(&workspace_root)
            .is_file()
    );
}

#[test]
fn test_tree_builder_file_directory_transition() {
    let test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let store = repo.store();
    let mut ws = test_workspace.workspace;
    let workspace_root = ws.workspace_root().to_owned();
    let mut check_out_tree = |tree_id: &TreeId| {
        let tree = repo.store().get_tree(RepoPathBuf::root(), tree_id).unwrap();
        let commit = commit_with_tree(
            repo.store(),
            MergedTree::resolved(repo.store().clone(), tree.id().clone()),
        );
        ws.check_out(repo.op_id().clone(), None, &commit).unwrap();
    };

    let parent_path = repo_path("foo/bar");
    let child_path = repo_path("foo/bar/baz");

    // Add file at parent_path
    let mut tree_builder = TreeBuilder::new(store.clone(), store.empty_tree_id().clone());
    tree_builder.set(
        parent_path.to_owned(),
        TreeValue::File {
            id: testutils::write_file(store, parent_path, ""),
            executable: false,
            copy_id: CopyId::placeholder(),
        },
    );
    let tree_id = tree_builder.write_tree().unwrap();
    check_out_tree(&tree_id);
    assert!(parent_path.to_fs_path_unchecked(&workspace_root).is_file());
    assert!(!child_path.to_fs_path_unchecked(&workspace_root).exists());

    // Turn parent_path into directory, add file at child_path
    let mut tree_builder = TreeBuilder::new(store.clone(), tree_id);
    tree_builder.remove(parent_path.to_owned());
    tree_builder.set(
        child_path.to_owned(),
        TreeValue::File {
            id: testutils::write_file(store, child_path, ""),
            executable: false,
            copy_id: CopyId::placeholder(),
        },
    );
    let tree_id = tree_builder.write_tree().unwrap();
    check_out_tree(&tree_id);
    assert!(parent_path.to_fs_path_unchecked(&workspace_root).is_dir());
    assert!(child_path.to_fs_path_unchecked(&workspace_root).is_file());

    // Turn parent_path back to file
    let mut tree_builder = TreeBuilder::new(store.clone(), tree_id);
    tree_builder.remove(child_path.to_owned());
    tree_builder.set(
        parent_path.to_owned(),
        TreeValue::File {
            id: testutils::write_file(store, parent_path, ""),
            executable: false,
            copy_id: CopyId::placeholder(),
        },
    );
    let tree_id = tree_builder.write_tree().unwrap();
    check_out_tree(&tree_id);
    assert!(parent_path.to_fs_path_unchecked(&workspace_root).is_file());
    assert!(!child_path.to_fs_path_unchecked(&workspace_root).exists());
}

#[test]
fn test_conflicting_changes_on_disk() {
    let test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let mut ws = test_workspace.workspace;
    let workspace_root = ws.workspace_root().to_owned();

    // file on disk conflicts with file in target commit
    let file_file_path = repo_path("file-file");
    // file on disk conflicts with directory in target commit
    let file_dir_path = repo_path("file-dir");
    // directory on disk conflicts with file in target commit
    let dir_file_path = repo_path("dir-file");
    let tree = create_tree(
        repo,
        &[
            (file_file_path, "committed contents"),
            (
                &file_dir_path.join(repo_path_component("file")),
                "committed contents",
            ),
            (dir_file_path, "committed contents"),
        ],
    );
    let commit = commit_with_tree(repo.store(), tree);

    std::fs::write(
        file_file_path.to_fs_path_unchecked(&workspace_root),
        "contents on disk",
    )
    .unwrap();
    std::fs::write(
        file_dir_path.to_fs_path_unchecked(&workspace_root),
        "contents on disk",
    )
    .unwrap();
    std::fs::create_dir(dir_file_path.to_fs_path_unchecked(&workspace_root)).unwrap();
    std::fs::write(
        dir_file_path
            .to_fs_path_unchecked(&workspace_root)
            .join("file"),
        "contents on disk",
    )
    .unwrap();

    let stats = ws.check_out(repo.op_id().clone(), None, &commit).unwrap();
    assert_eq!(
        stats,
        CheckoutStats {
            updated_files: 0,
            added_files: 3,
            removed_files: 0,
            skipped_files: 3
        }
    );

    assert_eq!(
        std::fs::read_to_string(file_file_path.to_fs_path_unchecked(&workspace_root)).ok(),
        Some("contents on disk".to_string())
    );
    assert_eq!(
        std::fs::read_to_string(file_dir_path.to_fs_path_unchecked(&workspace_root)).ok(),
        Some("contents on disk".to_string())
    );
    assert_eq!(
        std::fs::read_to_string(
            dir_file_path
                .to_fs_path_unchecked(&workspace_root)
                .join("file")
        )
        .ok(),
        Some("contents on disk".to_string())
    );
}

#[test]
fn test_reset() {
    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let op_id = repo.op_id().clone();
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();

    let ignored_path = repo_path("ignored");
    let gitignore_path = repo_path(".gitignore");

    let tree_without_file = create_tree(repo, &[(gitignore_path, "ignored\n")]);
    let commit_without_file = commit_with_tree(repo.store(), tree_without_file.clone());
    let tree_with_file = create_tree(
        repo,
        &[(gitignore_path, "ignored\n"), (ignored_path, "code")],
    );
    let commit_with_file = commit_with_tree(repo.store(), tree_with_file.clone());

    let ws = &mut test_workspace.workspace;
    let commit = commit_with_tree(repo.store(), tree_with_file.clone());
    ws.check_out(repo.op_id().clone(), None, &commit).unwrap();

    // Test the setup: the file should exist on disk and in the tree state.
    assert!(ignored_path.to_fs_path_unchecked(&workspace_root).is_file());
    let wc: &LocalWorkingCopy = ws.working_copy().downcast_ref().unwrap();
    assert!(wc.file_states().unwrap().contains_path(ignored_path));

    // After we reset to the commit without the file, it should still exist on disk,
    // but it should not be in the tree state, and it should not get added when we
    // commit the working copy (because it's ignored).
    let mut locked_ws = ws.start_working_copy_mutation().unwrap();
    locked_ws
        .locked_wc()
        .reset(&commit_without_file)
        .block_on()
        .unwrap();
    locked_ws.finish(op_id.clone()).unwrap();
    assert!(ignored_path.to_fs_path_unchecked(&workspace_root).is_file());
    let wc: &LocalWorkingCopy = ws.working_copy().downcast_ref().unwrap();
    assert!(!wc.file_states().unwrap().contains_path(ignored_path));
    let new_tree = test_workspace.snapshot().unwrap();
    assert_tree_eq!(new_tree, tree_without_file);

    // Now test the opposite direction: resetting to a commit where the file is
    // tracked. The file should become tracked (even though it's ignored).
    let ws = &mut test_workspace.workspace;
    let mut locked_ws = ws.start_working_copy_mutation().unwrap();
    locked_ws
        .locked_wc()
        .reset(&commit_with_file)
        .block_on()
        .unwrap();
    locked_ws.finish(op_id.clone()).unwrap();
    assert!(ignored_path.to_fs_path_unchecked(&workspace_root).is_file());
    let wc: &LocalWorkingCopy = ws.working_copy().downcast_ref().unwrap();
    assert!(wc.file_states().unwrap().contains_path(ignored_path));
    let new_tree = test_workspace.snapshot().unwrap();
    assert_tree_eq!(new_tree, tree_with_file);
}

#[test]
fn test_checkout_discard() {
    // Start a mutation, do a checkout, and then discard the mutation. The working
    // copy files should remain changed, but the state files should not be
    // written.
    let mut test_workspace = TestWorkspace::init();
    let repo = test_workspace.repo.clone();
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();

    let file1_path = repo_path("file1");
    let file2_path = repo_path("file2");

    let store = repo.store();
    let tree1 = create_tree(&repo, &[(file1_path, "contents")]);
    let tree2 = create_tree(&repo, &[(file2_path, "contents")]);
    let commit1 = commit_with_tree(repo.store(), tree1);
    let commit2 = commit_with_tree(repo.store(), tree2);

    let ws = &mut test_workspace.workspace;
    ws.check_out(repo.op_id().clone(), None, &commit1).unwrap();
    let wc: &LocalWorkingCopy = ws.working_copy().downcast_ref().unwrap();
    let state_path = wc.state_path().to_path_buf();

    // Test the setup: the file should exist on disk and in the tree state.
    assert!(file1_path.to_fs_path_unchecked(&workspace_root).is_file());
    let wc: &LocalWorkingCopy = ws.working_copy().downcast_ref().unwrap();
    assert!(wc.file_states().unwrap().contains_path(file1_path));

    // Start a checkout
    let mut locked_ws = ws.start_working_copy_mutation().unwrap();
    locked_ws
        .locked_wc()
        .check_out(&commit2)
        .block_on()
        .unwrap();
    // The change should be reflected in the working copy but not saved
    assert!(!file1_path.to_fs_path_unchecked(&workspace_root).is_file());
    assert!(file2_path.to_fs_path_unchecked(&workspace_root).is_file());
    let reloaded_wc = LocalWorkingCopy::load(
        store.clone(),
        workspace_root.clone(),
        state_path.clone(),
        repo.settings(),
    )
    .unwrap();
    assert!(reloaded_wc.file_states().unwrap().contains_path(file1_path));
    assert!(!reloaded_wc.file_states().unwrap().contains_path(file2_path));
    drop(locked_ws);

    // The change should remain in the working copy, but not in memory and not saved
    let wc: &LocalWorkingCopy = ws.working_copy().downcast_ref().unwrap();
    assert!(wc.file_states().unwrap().contains_path(file1_path));
    assert!(!wc.file_states().unwrap().contains_path(file2_path));
    assert!(!file1_path.to_fs_path_unchecked(&workspace_root).is_file());
    assert!(file2_path.to_fs_path_unchecked(&workspace_root).is_file());
    let reloaded_wc =
        LocalWorkingCopy::load(store.clone(), workspace_root, state_path, repo.settings()).unwrap();
    assert!(reloaded_wc.file_states().unwrap().contains_path(file1_path));
    assert!(!reloaded_wc.file_states().unwrap().contains_path(file2_path));
}

#[test]
fn test_snapshot_file_directory_transition() {
    let mut test_workspace = TestWorkspace::init();
    let repo = test_workspace.repo.clone();
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();
    let to_ws_path = |path: &RepoPath| path.to_fs_path(&workspace_root).unwrap();

    // file <-> directory transition at root and sub directories
    let file1_path = repo_path("foo/bar");
    let file2_path = repo_path("sub/bar/baz");
    let file1p_path = file1_path.parent().unwrap();
    let file2p_path = file2_path.parent().unwrap();

    let tree1 = create_tree(&repo, &[(file1p_path, "1p"), (file2p_path, "2p")]);
    let tree2 = create_tree(&repo, &[(file1_path, "1"), (file2_path, "2")]);
    let commit1 = commit_with_tree(repo.store(), tree1.clone());
    let commit2 = commit_with_tree(repo.store(), tree2.clone());

    let ws = &mut test_workspace.workspace;
    ws.check_out(repo.op_id().clone(), None, &commit1).unwrap();

    // file -> directory
    std::fs::remove_file(to_ws_path(file1p_path)).unwrap();
    std::fs::remove_file(to_ws_path(file2p_path)).unwrap();
    std::fs::create_dir(to_ws_path(file1p_path)).unwrap();
    std::fs::create_dir(to_ws_path(file2p_path)).unwrap();
    std::fs::write(to_ws_path(file1_path), "1").unwrap();
    std::fs::write(to_ws_path(file2_path), "2").unwrap();
    let new_tree = test_workspace.snapshot().unwrap();
    assert_tree_eq!(new_tree, tree2);

    let ws = &mut test_workspace.workspace;
    ws.check_out(repo.op_id().clone(), None, &commit2).unwrap();

    // directory -> file
    std::fs::remove_file(to_ws_path(file1_path)).unwrap();
    std::fs::remove_file(to_ws_path(file2_path)).unwrap();
    std::fs::remove_dir(to_ws_path(file1p_path)).unwrap();
    std::fs::remove_dir(to_ws_path(file2p_path)).unwrap();
    std::fs::write(to_ws_path(file1p_path), "1p").unwrap();
    std::fs::write(to_ws_path(file2p_path), "2p").unwrap();
    let new_tree = test_workspace.snapshot().unwrap();
    assert_tree_eq!(new_tree, tree1);
}

#[test]
fn test_materialize_snapshot_conflicted_files() {
    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo.clone();
    let ws = &mut test_workspace.workspace;
    let workspace_root = ws.workspace_root().to_owned();

    // Create tree with 3-sided conflict, with file1 and file2 having different
    // conflicts:
    // file1: A - A + A - B + C
    // file2: A - B + C - D + D
    let file1_path = repo_path("file1");
    let file2_path = repo_path("file2");
    let side1_tree = create_tree(repo, &[(file1_path, "a\n"), (file2_path, "1\n")]);
    let base1_tree = create_tree(repo, &[(file1_path, "a\n"), (file2_path, "2\n")]);
    let side2_tree = create_tree(repo, &[(file1_path, "a\n"), (file2_path, "4\n")]);
    let base2_tree = create_tree(repo, &[(file1_path, "b\n"), (file2_path, "3\n")]);
    let side3_tree = create_tree(repo, &[(file1_path, "c\n"), (file2_path, "3\n")]);
    let merged_tree = side1_tree
        .merge(base1_tree, side2_tree)
        .block_on()
        .unwrap()
        .merge(base2_tree, side3_tree)
        .block_on()
        .unwrap();
    let commit = commit_with_tree(repo.store(), merged_tree.clone());

    let stats = ws.check_out(repo.op_id().clone(), None, &commit).unwrap();
    assert_eq!(
        stats,
        CheckoutStats {
            updated_files: 0,
            added_files: 2,
            removed_files: 0,
            skipped_files: 0
        }
    );

    // Even though the tree-level conflict is a 3-sided conflict, each file is
    // materialized as a 2-sided conflict.
    let file1_value = merged_tree.path_value(file1_path).unwrap();
    let file2_value = merged_tree.path_value(file2_path).unwrap();
    assert_eq!(file1_value.num_sides(), 3);
    assert_eq!(file2_value.num_sides(), 3);
    insta::assert_snapshot!(
        std::fs::read_to_string(file1_path.to_fs_path_unchecked(&workspace_root)).ok().unwrap(),
        @r"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    -b
    +a
    +++++++ Contents of side #2
    c
    >>>>>>> Conflict 1 of 1 ends
    ");
    insta::assert_snapshot!(
        std::fs::read_to_string(file2_path.to_fs_path_unchecked(&workspace_root)).ok().unwrap(),
        @r"
    <<<<<<< Conflict 1 of 1
    %%%%%%% Changes from base to side #1
    -2
    +1
    +++++++ Contents of side #2
    4
    >>>>>>> Conflict 1 of 1 ends
    ");

    // Editing a conflicted file should correctly propagate updates to each of
    // the conflicting trees.
    testutils::write_working_copy_file(
        &workspace_root,
        file1_path,
        indoc! {"
            <<<<<<< Conflict 1 of 1
            %%%%%%% Changes from base to side #1
            -b_edited
            +a_edited
            +++++++ Contents of side #2
            c_edited
            >>>>>>> Conflict 1 of 1 ends
        "},
    );

    let edited_tree = test_workspace.snapshot().unwrap();
    let edited_file_value = edited_tree.path_value(file1_path).unwrap();
    let edited_file_values = edited_file_value.iter().flatten().collect_vec();
    assert_eq!(edited_file_values.len(), 5);

    let get_file_id = |value: &TreeValue| match value {
        TreeValue::File { id, .. } => id.clone(),
        _ => panic!("unexpected value: {value:#?}"),
    };
    // The file IDs with indices 0 and 1 are the original unedited file values
    // which were simplified.
    let edited_file_file_id_0 = get_file_id(edited_file_values[0]);
    assert_eq!(
        testutils::read_file(repo.store(), file1_path, &edited_file_file_id_0),
        b"a\n"
    );
    assert_eq!(edited_file_values[0], edited_file_values[1]);
    let edited_file_file_id_2 = get_file_id(edited_file_values[2]);
    assert_eq!(
        testutils::read_file(repo.store(), file1_path, &edited_file_file_id_2),
        b"a_edited\n"
    );
    let edited_file_file_id_3 = get_file_id(edited_file_values[3]);
    assert_eq!(
        testutils::read_file(repo.store(), file1_path, &edited_file_file_id_3),
        b"b_edited\n"
    );
    let edited_file_file_id_4 = get_file_id(edited_file_values[4]);
    assert_eq!(
        testutils::read_file(repo.store(), file1_path, &edited_file_file_id_4),
        b"c_edited\n"
    );
}

#[test]
fn test_materialize_snapshot_unchanged_conflicts() {
    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();

    // Both sides change "line 3" differently, right side deletes "line 5".
    let base_content = indoc! {"
        line 1
        line 2
        line 3
        line 4
        line 5
    "};
    let left_content = indoc! {"
        line 1
        line 2
        left 3.1
        left 3.2
        left 3.3
        line 4
        line 5
    "};
    let right_content = indoc! {"
        line 1
        line 2
        right 3.1
        line 4
    "};
    let file_path = repo_path("file");
    let base_tree = create_tree(repo, &[(file_path, base_content)]);
    let left_tree = create_tree(repo, &[(file_path, left_content)]);
    let right_tree = create_tree(repo, &[(file_path, right_content)]);
    let merged_tree = left_tree.merge(base_tree, right_tree).block_on().unwrap();
    let commit = commit_with_tree(repo.store(), merged_tree.clone());

    test_workspace
        .workspace
        .check_out(repo.op_id().clone(), None, &commit)
        .unwrap();

    // "line 5" should be deleted from the checked-out content.
    let disk_path = file_path.to_fs_path_unchecked(&workspace_root);
    let materialized_content = std::fs::read_to_string(&disk_path).unwrap();
    insta::assert_snapshot!(materialized_content, @r"
    line 1
    line 2
    <<<<<<< Conflict 1 of 1
    +++++++ Contents of side #1
    left 3.1
    left 3.2
    left 3.3
    %%%%%%% Changes from base to side #2
    -line 3
    +right 3.1
    >>>>>>> Conflict 1 of 1 ends
    line 4
    ");

    // Update mtime to bypass file state comparison.
    let file = File::options().write(true).open(&disk_path).unwrap();
    file.set_modified(SystemTime::now() + Duration::from_secs(1))
        .unwrap();
    drop(file);

    // Unchanged snapshot should be identical to the original even if "line 5"
    // could be deleted from all sides.
    let snapshotted_tree = test_workspace.snapshot().unwrap();
    assert_eq!(tree_entries(&snapshotted_tree), tree_entries(&merged_tree));
}

#[test]
fn test_snapshot_racy_timestamps() {
    // Tests that file modifications are detected even if they happen the same
    // millisecond as the updated working copy state.
    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();

    let file_path = workspace_root.join("file");
    let mut previous_tree = repo.store().empty_merged_tree();
    for i in 0..100 {
        std::fs::write(&file_path, format!("contents {i}").as_bytes()).unwrap();
        let mut locked_ws = test_workspace
            .workspace
            .start_working_copy_mutation()
            .unwrap();
        let (new_tree, _stats) = locked_ws
            .locked_wc()
            .snapshot(&empty_snapshot_options())
            .block_on()
            .unwrap();
        assert_ne!(new_tree.tree_ids(), previous_tree.tree_ids());
        previous_tree = new_tree;
    }
}

#[cfg(unix)]
#[test]
fn test_snapshot_special_file() {
    // Tests that we ignore when special files (such as sockets and pipes) exist on
    // disk.
    let mut test_workspace = TestWorkspace::init();
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();
    let ws = &mut test_workspace.workspace;

    let file1_path = repo_path("file1");
    let file1_disk_path = file1_path.to_fs_path_unchecked(&workspace_root);
    std::fs::write(&file1_disk_path, "contents".as_bytes()).unwrap();
    let file2_path = repo_path("file2");
    let file2_disk_path = file2_path.to_fs_path_unchecked(&workspace_root);
    std::fs::write(file2_disk_path, "contents".as_bytes()).unwrap();

    let fifo_disk_path = workspace_root.join("fifo");
    nix::unistd::mkfifo(&fifo_disk_path, nix::sys::stat::Mode::S_IRWXU).unwrap();
    assert!(fifo_disk_path.exists());
    assert!(!fifo_disk_path.is_file());

    // Snapshot the working copy with the socket file
    let mut locked_ws = ws.start_working_copy_mutation().unwrap();
    let (tree, _stats) = locked_ws
        .locked_wc()
        .snapshot(&empty_snapshot_options())
        .block_on()
        .unwrap();
    locked_ws.finish(OperationId::from_hex("abc123")).unwrap();
    // Only the regular files should be in the tree
    assert_eq!(
        tree.entries().map(|(path, _value)| path).collect_vec(),
        to_owned_path_vec(&[file1_path, file2_path])
    );
    let wc: &LocalWorkingCopy = ws.working_copy().downcast_ref().unwrap();
    assert_eq!(
        wc.file_states().unwrap().paths().collect_vec(),
        vec![file1_path, file2_path]
    );

    // Replace a regular file by a socket and snapshot the working copy again
    std::fs::remove_file(&file1_disk_path).unwrap();
    nix::unistd::mkfifo(&file1_disk_path, nix::sys::stat::Mode::S_IRWXU).unwrap();
    let tree = test_workspace.snapshot().unwrap();
    // Only the regular file should be in the tree
    assert_eq!(
        tree.entries().map(|(path, _value)| path).collect_vec(),
        to_owned_path_vec(&[file2_path])
    );
    let ws = &mut test_workspace.workspace;
    let wc: &LocalWorkingCopy = ws.working_copy().downcast_ref().unwrap();
    assert_eq!(
        wc.file_states().unwrap().paths().collect_vec(),
        vec![file2_path]
    );
}

#[test]
fn test_gitignores() {
    // Tests that .gitignore files are respected.

    let mut test_workspace = TestWorkspace::init();
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();

    let gitignore_path = repo_path(".gitignore");
    let added_path = repo_path("added");
    let modified_path = repo_path("modified");
    let removed_path = repo_path("removed");
    let ignored_path = repo_path("ignored");
    let subdir_modified_path = repo_path("dir/modified");
    let subdir_ignored_path = repo_path("dir/ignored");

    testutils::write_working_copy_file(&workspace_root, gitignore_path, "ignored\n");
    testutils::write_working_copy_file(&workspace_root, modified_path, "1");
    testutils::write_working_copy_file(&workspace_root, removed_path, "1");
    std::fs::create_dir(workspace_root.join("dir")).unwrap();
    testutils::write_working_copy_file(&workspace_root, subdir_modified_path, "1");

    let tree1 = test_workspace.snapshot().unwrap();
    let files1 = tree1.entries().map(|(name, _value)| name).collect_vec();
    assert_eq!(
        files1,
        to_owned_path_vec(&[
            gitignore_path,
            subdir_modified_path,
            modified_path,
            removed_path,
        ])
    );

    testutils::write_working_copy_file(
        &workspace_root,
        gitignore_path,
        "ignored\nmodified\nremoved\n",
    );
    testutils::write_working_copy_file(&workspace_root, added_path, "2");
    testutils::write_working_copy_file(&workspace_root, modified_path, "2");
    std::fs::remove_file(removed_path.to_fs_path_unchecked(&workspace_root)).unwrap();
    testutils::write_working_copy_file(&workspace_root, ignored_path, "2");
    testutils::write_working_copy_file(&workspace_root, subdir_modified_path, "2");
    testutils::write_working_copy_file(&workspace_root, subdir_ignored_path, "2");

    let tree2 = test_workspace.snapshot().unwrap();
    let files2 = tree2.entries().map(|(name, _value)| name).collect_vec();
    assert_eq!(
        files2,
        to_owned_path_vec(&[
            gitignore_path,
            added_path,
            subdir_modified_path,
            modified_path,
        ])
    );
}

#[test]
fn test_gitignores_in_ignored_dir() {
    // Tests that .gitignore files in an ignored directory are ignored, i.e. that
    // they cannot override the ignores from the parent

    let mut test_workspace = TestWorkspace::init();
    let op_id = test_workspace.repo.op_id().clone();
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();

    let gitignore_path = repo_path(".gitignore");
    let nested_gitignore_path = repo_path("ignored/.gitignore");
    let ignored_path = repo_path("ignored/file");

    let tree1 = create_tree(&test_workspace.repo, &[(gitignore_path, "ignored\n")]);
    let commit1 = commit_with_tree(test_workspace.repo.store(), tree1.clone());
    let ws = &mut test_workspace.workspace;
    ws.check_out(op_id.clone(), None, &commit1).unwrap();

    testutils::write_working_copy_file(&workspace_root, nested_gitignore_path, "!file\n");
    testutils::write_working_copy_file(&workspace_root, ignored_path, "contents");

    let new_tree = test_workspace.snapshot().unwrap();
    assert_eq!(tree_entries(&new_tree), tree_entries(&tree1));

    // The nested .gitignore is ignored even if it's tracked
    let tree2 = create_tree(
        &test_workspace.repo,
        &[
            (gitignore_path, "ignored\n"),
            (nested_gitignore_path, "!file\n"),
        ],
    );
    let commit2 = commit_with_tree(test_workspace.repo.store(), tree2.clone());
    let mut locked_ws = test_workspace
        .workspace
        .start_working_copy_mutation()
        .unwrap();
    locked_ws.locked_wc().reset(&commit2).block_on().unwrap();
    locked_ws.finish(OperationId::from_hex("abc123")).unwrap();

    let new_tree = test_workspace.snapshot().unwrap();
    assert_eq!(tree_entries(&new_tree), tree_entries(&tree2));
}

#[test]
fn test_gitignores_checkout_never_overwrites_ignored() {
    // Tests that a .gitignore'd file doesn't get overwritten if check out a commit
    // where the file is tracked.

    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();

    // Write an ignored file called "modified" to disk
    let gitignore_path = repo_path(".gitignore");
    testutils::write_working_copy_file(&workspace_root, gitignore_path, "modified\n");
    let modified_path = repo_path("modified");
    testutils::write_working_copy_file(&workspace_root, modified_path, "garbage");

    // Create a tree that adds the same file but with different contents
    let tree = create_tree(repo, &[(modified_path, "contents")]);
    let commit = commit_with_tree(repo.store(), tree);

    // Now check out the tree that adds the file "modified" with contents
    // "contents". The exiting contents ("garbage") shouldn't be replaced in the
    // working copy.
    let ws = &mut test_workspace.workspace;
    assert!(ws.check_out(repo.op_id().clone(), None, &commit,).is_ok());

    // Check that the old contents are in the working copy
    let path = workspace_root.join("modified");
    assert!(path.is_file());
    assert_eq!(std::fs::read(&path).unwrap(), b"garbage");
}

#[test]
fn test_gitignores_ignored_directory_already_tracked() {
    // Tests that a .gitignore'd directory that already has a tracked file in it
    // does not get removed when snapshotting the working directory.

    let mut test_workspace = TestWorkspace::init();
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();
    let repo = test_workspace.repo.clone();

    let gitignore_path = repo_path(".gitignore");
    let unchanged_normal_path = repo_path("ignored/unchanged_normal");
    let modified_normal_path = repo_path("ignored/modified_normal");
    let deleted_normal_path = repo_path("ignored/deleted_normal");
    let unchanged_executable_path = repo_path("ignored/unchanged_executable");
    let modified_executable_path = repo_path("ignored/modified_executable");
    let deleted_executable_path = repo_path("ignored/deleted_executable");
    let unchanged_symlink_path = repo_path("ignored/unchanged_symlink");
    let modified_symlink_path = repo_path("ignored/modified_symlink");
    let deleted_symlink_path = repo_path("ignored/deleted_symlink");
    let tree = create_tree_with(&repo, |builder| {
        builder.file(gitignore_path, "/ignored/\n");
        builder.file(unchanged_normal_path, "contents");
        builder.file(modified_normal_path, "contents");
        builder.file(deleted_normal_path, "contents");
        builder
            .file(unchanged_executable_path, "contents")
            .executable(true);
        builder
            .file(modified_executable_path, "contents")
            .executable(true);
        builder
            .file(deleted_executable_path, "contents")
            .executable(true);
        builder.symlink(unchanged_symlink_path, "contents");
        builder.symlink(modified_symlink_path, "contents");
        builder.symlink(deleted_symlink_path, "contents");
    });
    let commit = commit_with_tree(repo.store(), tree);

    // Check out the tree with the files in `ignored/`
    let ws = &mut test_workspace.workspace;
    ws.check_out(repo.op_id().clone(), None, &commit).unwrap();

    // Make some changes inside the ignored directory and check that they are
    // detected when we snapshot. The files that are still there should not be
    // deleted from the resulting tree.
    std::fs::write(
        modified_normal_path.to_fs_path_unchecked(&workspace_root),
        "modified",
    )
    .unwrap();
    std::fs::remove_file(deleted_normal_path.to_fs_path_unchecked(&workspace_root)).unwrap();
    std::fs::write(
        modified_executable_path.to_fs_path_unchecked(&workspace_root),
        "modified",
    )
    .unwrap();
    std::fs::remove_file(deleted_executable_path.to_fs_path_unchecked(&workspace_root)).unwrap();
    let fs_path = modified_symlink_path.to_fs_path_unchecked(&workspace_root);
    std::fs::remove_file(&fs_path).unwrap();
    if check_symlink_support().unwrap_or(false) {
        try_symlink("modified", &fs_path).unwrap();
    } else {
        std::fs::write(fs_path, "modified").unwrap();
    }
    std::fs::remove_file(deleted_symlink_path.to_fs_path_unchecked(&workspace_root)).unwrap();
    let new_tree = test_workspace.snapshot().unwrap();
    let expected_tree = create_tree_with(&repo, |builder| {
        builder.file(gitignore_path, "/ignored/\n");
        builder.file(unchanged_normal_path, "contents");
        builder.file(modified_normal_path, "modified");
        builder
            .file(unchanged_executable_path, "contents")
            .executable(true);
        builder
            .file(modified_executable_path, "modified")
            .executable(true);
        builder.symlink(unchanged_symlink_path, "contents");
        builder.symlink(modified_symlink_path, "modified");
    });
    assert_eq!(tree_entries(&new_tree), tree_entries(&expected_tree));
}

#[test]
fn test_dotgit_ignored() {
    // Tests that .git directories and files are always ignored (we could accept
    // them if the backend is not git).

    let mut test_workspace = TestWorkspace::init();
    let store = test_workspace.repo.store().clone();
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();

    // Test with a .git/ directory (with a file in, since we don't write empty
    // trees)
    let dotgit_path = workspace_root.join(".git");
    std::fs::create_dir(&dotgit_path).unwrap();
    testutils::write_working_copy_file(&workspace_root, repo_path(".git/file"), "contents");
    let new_tree = test_workspace.snapshot().unwrap();
    let empty_tree = store.empty_merged_tree();
    assert_tree_eq!(new_tree, empty_tree);
    std::fs::remove_dir_all(&dotgit_path).unwrap();

    // Test with a .git file
    testutils::write_working_copy_file(&workspace_root, repo_path(".git"), "contents");
    let new_tree = test_workspace.snapshot().unwrap();
    assert_tree_eq!(new_tree, empty_tree);
}

#[test_case(""; "ignore nothing")]
#[test_case("/*\n"; "ignore all")]
fn test_git_submodule(gitignore_content: &str) {
    // Tests that git submodules are ignored.

    let mut test_workspace = TestWorkspace::init_with_backend(TestRepoBackend::Git);
    let repo = test_workspace.repo.clone();
    let store = repo.store().clone();
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();
    let base_ignores = GitIgnoreFile::empty()
        .chain("", Path::new(""), gitignore_content.as_bytes())
        .unwrap();
    let snapshot_options = SnapshotOptions {
        base_ignores,
        ..empty_snapshot_options()
    };
    let mut tx = repo.start_transaction();

    // Add files in sub directory. Sub directories are traversed differently
    // depending on .gitignore. #5246
    let added_path = repo_path("sub/added");
    let submodule_path = repo_path("sub/module");
    let added_submodule_path = repo_path("sub/module/added");

    let mut tree_builder = MergedTreeBuilder::new(store.empty_merged_tree());

    tree_builder.set_or_remove(
        added_path.to_owned(),
        Merge::normal(TreeValue::File {
            id: testutils::write_file(repo.store(), added_path, "added\n"),
            executable: false,
            copy_id: CopyId::new(vec![]),
        }),
    );

    let submodule_id1 = write_random_commit(tx.repo_mut()).id().clone();

    tree_builder.set_or_remove(
        submodule_path.to_owned(),
        Merge::normal(TreeValue::GitSubmodule(submodule_id1)),
    );

    let tree_id1 = tree_builder.write_tree().unwrap();
    let commit1 = commit_with_tree(repo.store(), tree_id1.clone());

    let mut tree_builder = MergedTreeBuilder::new(tree_id1.clone());
    let submodule_id2 = write_random_commit(tx.repo_mut()).id().clone();
    tree_builder.set_or_remove(
        submodule_path.to_owned(),
        Merge::normal(TreeValue::GitSubmodule(submodule_id2)),
    );
    let tree_id2 = tree_builder.write_tree().unwrap();
    let commit2 = commit_with_tree(repo.store(), tree_id2.clone());

    let ws = &mut test_workspace.workspace;
    ws.check_out(repo.op_id().clone(), None, &commit1).unwrap();

    std::fs::create_dir(submodule_path.to_fs_path_unchecked(&workspace_root)).unwrap();

    testutils::write_working_copy_file(
        &workspace_root,
        added_submodule_path,
        "i am a file in a submodule\n",
    );

    // Check that the files present in the submodule are not tracked
    // when we snapshot
    let (new_tree, _stats) = test_workspace
        .snapshot_with_options(&snapshot_options)
        .unwrap();
    assert_tree_eq!(new_tree, tree_id1);

    // Check that the files in the submodule are not deleted
    let file_in_submodule_path = added_submodule_path.to_fs_path_unchecked(&workspace_root);
    assert!(
        file_in_submodule_path.metadata().is_ok(),
        "{file_in_submodule_path:?} should exist"
    );

    // Check out new commit updating the submodule, which shouldn't fail because
    // of existing submodule files
    let ws = &mut test_workspace.workspace;
    ws.check_out(repo.op_id().clone(), None, &commit2).unwrap();

    // Check that the files in the submodule are not deleted
    let file_in_submodule_path = added_submodule_path.to_fs_path_unchecked(&workspace_root);
    assert!(
        file_in_submodule_path.metadata().is_ok(),
        "{file_in_submodule_path:?} should exist"
    );

    // Check that the files present in the submodule are not tracked
    // when we snapshot
    let (new_tree, _stats) = test_workspace
        .snapshot_with_options(&snapshot_options)
        .unwrap();
    assert_tree_eq!(new_tree, tree_id2);

    // Check out the empty tree, which shouldn't fail
    let ws = &mut test_workspace.workspace;
    let stats = ws
        .check_out(repo.op_id().clone(), None, &store.root_commit())
        .unwrap();
    assert_eq!(stats.skipped_files, 1);
}

#[test]
fn test_check_out_existing_file_cannot_be_removed() {
    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();

    let file_path = repo_path("file");
    let tree1 = create_tree(repo, &[(file_path, "0")]);
    let tree2 = create_tree(repo, &[(file_path, "1")]);
    let commit1 = commit_with_tree(repo.store(), tree1);
    let commit2 = commit_with_tree(repo.store(), tree2);

    let ws = &mut test_workspace.workspace;
    ws.check_out(repo.op_id().clone(), None, &commit1).unwrap();

    // Make the parent directory readonly.
    let writable_dir_perm = workspace_root.symlink_metadata().unwrap().permissions();
    let mut readonly_dir_perm = writable_dir_perm.clone();
    readonly_dir_perm.set_readonly(true);

    std::fs::set_permissions(&workspace_root, readonly_dir_perm).unwrap();
    let result = ws.check_out(repo.op_id().clone(), None, &commit2);
    std::fs::set_permissions(&workspace_root, writable_dir_perm).unwrap();

    // TODO: find a way to trigger the error on Windows
    if !cfg!(windows) {
        assert_matches!(
            result,
            Err(CheckoutError::Other { message, .. }) if message.contains("Failed to remove")
        );
    }
}

#[test]
fn test_check_out_existing_file_replaced_with_directory() {
    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();

    let file_path = repo_path("file");
    let tree1 = create_tree(repo, &[(file_path, "0")]);
    let tree2 = create_tree(repo, &[(file_path, "1")]);
    let commit1 = commit_with_tree(repo.store(), tree1);
    let commit2 = commit_with_tree(repo.store(), tree2);

    let ws = &mut test_workspace.workspace;
    ws.check_out(repo.op_id().clone(), None, &commit1).unwrap();

    std::fs::remove_file(file_path.to_fs_path_unchecked(&workspace_root)).unwrap();
    std::fs::create_dir(file_path.to_fs_path_unchecked(&workspace_root)).unwrap();

    // Checkout doesn't fail, but the file should be skipped.
    let stats = ws.check_out(repo.op_id().clone(), None, &commit2).unwrap();
    assert_eq!(stats.skipped_files, 1);
    assert!(file_path.to_fs_path_unchecked(&workspace_root).is_dir());
}

#[test]
fn test_check_out_existing_directory_symlink() {
    if !check_symlink_support().unwrap() {
        eprintln!("Skipping test because symlink isn't supported");
        return;
    }

    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();

    // Creates a symlink in working directory, and a tree that will add a file
    // under the symlinked directory.
    try_symlink("..", workspace_root.join("parent")).unwrap();

    let file_path = repo_path("parent/escaped");
    let tree = create_tree(repo, &[(file_path, "contents")]);
    let commit = commit_with_tree(repo.store(), tree);

    // Checkout doesn't fail, but the file should be skipped.
    let ws = &mut test_workspace.workspace;
    let stats = ws.check_out(repo.op_id().clone(), None, &commit).unwrap();
    assert_eq!(stats.skipped_files, 1);

    // Therefore, "../escaped" shouldn't be created.
    assert!(!workspace_root.parent().unwrap().join("escaped").exists());
}

#[test]
fn test_check_out_existing_directory_symlink_icase_fs() {
    if !check_symlink_support().unwrap() {
        eprintln!("Skipping test because symlink isn't supported");
        return;
    }

    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();
    let is_icase_fs = check_icase_fs(&workspace_root);

    // Creates a symlink in working directory, and a tree that will add a file
    // under the symlinked directory.
    try_symlink("..", workspace_root.join("parent")).unwrap();

    let file_path = repo_path("PARENT/escaped");
    let tree = create_tree(repo, &[(file_path, "contents")]);
    let commit = commit_with_tree(repo.store(), tree);

    // Checkout doesn't fail, but the file should be skipped on icase fs.
    let ws = &mut test_workspace.workspace;
    let stats = ws.check_out(repo.op_id().clone(), None, &commit).unwrap();
    if is_icase_fs {
        assert_eq!(stats.skipped_files, 1);
    } else {
        assert_eq!(stats.skipped_files, 0);
    }

    // Therefore, "../escaped" shouldn't be created.
    assert!(!workspace_root.parent().unwrap().join("escaped").exists());
}

#[test_case(false; "symlink target does not exist")]
#[test_case(true; "symlink target exists")]
fn test_check_out_existing_file_symlink_icase_fs(victim_exists: bool) {
    if !check_symlink_support().unwrap() {
        eprintln!("Skipping test because symlink isn't supported");
        return;
    }

    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();
    let is_icase_fs = check_icase_fs(&workspace_root);

    // Creates a symlink in working directory, and a tree that will overwrite
    // the symlink content.
    try_symlink(
        PathBuf::from_iter(["..", "pwned"]),
        workspace_root.join("parent"),
    )
    .unwrap();
    let victim_file_path = workspace_root.parent().unwrap().join("pwned");
    if victim_exists {
        std::fs::write(&victim_file_path, "old").unwrap();
    }
    assert_eq!(workspace_root.join("parent").exists(), victim_exists);

    let file_path = repo_path("PARENT");
    let tree = create_tree(repo, &[(file_path, "bad")]);
    let commit = commit_with_tree(repo.store(), tree);

    // Checkout doesn't fail, but the file should be skipped on icase fs.
    let ws = &mut test_workspace.workspace;
    let stats = ws.check_out(repo.op_id().clone(), None, &commit).unwrap();
    if is_icase_fs {
        assert_eq!(stats.skipped_files, 1);
    } else {
        assert_eq!(stats.skipped_files, 0);
    }

    // Therefore, "../pwned" shouldn't be updated.
    if victim_exists {
        assert_eq!(std::fs::read(&victim_file_path).unwrap(), b"old");
    } else {
        assert!(!victim_file_path.exists());
    }
}

#[test]
fn test_check_out_file_removal_over_existing_directory_symlink() {
    if !check_symlink_support().unwrap() {
        eprintln!("Skipping test because symlink isn't supported");
        return;
    }

    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();

    let file_path = repo_path("parent/escaped");
    let tree1 = create_tree(repo, &[(file_path, "contents")]);
    let tree2 = create_tree(repo, &[]);
    let commit1 = commit_with_tree(repo.store(), tree1);
    let commit2 = commit_with_tree(repo.store(), tree2);

    // Check out "parent/escaped".
    let ws = &mut test_workspace.workspace;
    ws.check_out(repo.op_id().clone(), None, &commit1).unwrap();

    // Pretend that "parent" was a symlink, which might be created by
    // e.g. checking out "PARENT" on case-insensitive fs. The file
    // "parent/escaped" would be skipped in that case.
    std::fs::remove_file(file_path.to_fs_path_unchecked(&workspace_root)).unwrap();
    std::fs::remove_dir(workspace_root.join("parent")).unwrap();
    try_symlink("..", workspace_root.join("parent")).unwrap();
    let victim_file_path = workspace_root.parent().unwrap().join("escaped");
    std::fs::write(&victim_file_path, "").unwrap();
    assert!(file_path.to_fs_path_unchecked(&workspace_root).exists());

    // Check out empty tree, which tries to remove "parent/escaped".
    let stats = ws.check_out(repo.op_id().clone(), None, &commit2).unwrap();
    assert_eq!(stats.skipped_files, 1);

    // "../escaped" shouldn't be removed.
    assert!(victim_file_path.exists());
}

#[test_case("../pwned"; "escape from root")]
#[test_case("sub/../../pwned"; "escape from sub dir")]
fn test_check_out_malformed_file_path(file_path_str: &str) {
    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();

    let file_path = repo_path(file_path_str);
    let tree = create_tree(repo, &[(file_path, "contents")]);
    let commit = commit_with_tree(repo.store(), tree);

    // Checkout should fail
    let ws = &mut test_workspace.workspace;
    let result = ws.check_out(repo.op_id().clone(), None, &commit);
    assert_matches!(result, Err(CheckoutError::InvalidRepoPath(_)));

    // Therefore, "pwned" file shouldn't be created.
    assert!(!workspace_root.join(file_path_str).exists());
    assert!(!workspace_root.parent().unwrap().join("pwned").exists());
}

#[test_case(r"sub\..\../pwned"; "path separator")]
#[test_case("d:/pwned"; "drive letter")]
fn test_check_out_malformed_file_path_windows(file_path_str: &str) {
    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();

    let file_path = repo_path(file_path_str);
    let tree = create_tree(repo, &[(file_path, "contents")]);
    let commit = commit_with_tree(repo.store(), tree);

    // Checkout should fail on Windows
    let ws = &mut test_workspace.workspace;
    let result = ws.check_out(repo.op_id().clone(), None, &commit);
    if cfg!(windows) {
        assert_matches!(result, Err(CheckoutError::InvalidRepoPath(_)));
    } else {
        assert_matches!(result, Ok(_));
    }

    // Therefore, "pwned" file shouldn't be created.
    if cfg!(windows) {
        assert!(!workspace_root.join(file_path_str).exists());
    }
    assert!(!workspace_root.parent().unwrap().join("pwned").exists());
}

#[test_case(".git"; "root .git file")]
#[test_case(".jj"; "root .jj file")]
#[test_case(".git/pwned"; "root .git dir")]
#[test_case(".jj/pwned"; "root .jj dir")]
#[test_case("sub/.git"; "sub .git file")]
#[test_case("sub/.jj"; "sub .jj file")]
#[test_case("sub/.git/pwned"; "sub .git dir")]
#[test_case("sub/.jj/pwned"; "sub .jj dir")]
fn test_check_out_reserved_file_path(file_path_str: &str) {
    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();
    std::fs::create_dir(workspace_root.join(".git")).unwrap();

    let file_path = repo_path(file_path_str);
    let disk_path = file_path.to_fs_path_unchecked(&workspace_root);
    let tree1 = create_tree(repo, &[(file_path, "contents")]);
    let tree2 = create_tree(repo, &[]);
    let commit1 = commit_with_tree(repo.store(), tree1);
    let commit2 = commit_with_tree(repo.store(), tree2);

    // Checkout should fail.
    let ws = &mut test_workspace.workspace;
    let result = ws.check_out(repo.op_id().clone(), None, &commit1);
    assert_matches!(result, Err(CheckoutError::ReservedPathComponent { .. }));

    // Therefore, "pwned" file shouldn't be created.
    if ![".git", ".jj"].contains(&file_path_str) {
        assert!(!disk_path.exists());
    }
    assert!(!workspace_root.join(".git").join("pwned").exists());
    assert!(!workspace_root.join(".jj").join("pwned").exists());
    assert!(!workspace_root.join("sub").join(".git").exists());
    assert!(!workspace_root.join("sub").join(".jj").exists());

    // Pretend that the checkout somehow succeeded.
    let mut locked_ws = ws.start_working_copy_mutation().unwrap();
    locked_ws.locked_wc().reset(&commit1).block_on().unwrap();
    locked_ws.finish(repo.op_id().clone()).unwrap();
    if ![".git", ".jj"].contains(&file_path_str) {
        std::fs::create_dir_all(disk_path.parent().unwrap()).unwrap();
        std::fs::write(&disk_path, "").unwrap();
    }

    // Check out empty tree, which tries to remove the file.
    let result = ws.check_out(repo.op_id().clone(), None, &commit2);
    assert_matches!(result, Err(CheckoutError::ReservedPathComponent { .. }));

    // The existing file shouldn't be removed.
    assert!(disk_path.exists());
}

#[test_case(".Git/pwned"; "root .git dir")]
#[test_case(".jJ/pwned"; "root .jj dir")]
#[test_case("sub/.GIt"; "sub .git file")]
#[test_case("sub/.JJ"; "sub .jj file")]
#[test_case("sub/.gIT/pwned"; "sub .git dir")]
#[test_case("sub/.Jj/pwned"; "sub .jj dir")]
fn test_check_out_reserved_file_path_icase_fs(file_path_str: &str) {
    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();
    std::fs::create_dir(workspace_root.join(".git")).unwrap();
    let is_icase_fs = check_icase_fs(&workspace_root);

    let file_path = repo_path(file_path_str);
    let disk_path = file_path.to_fs_path_unchecked(&workspace_root);
    let tree1 = create_tree(repo, &[(file_path, "contents")]);
    let tree2 = create_tree(repo, &[]);
    let commit1 = commit_with_tree(repo.store(), tree1);
    let commit2 = commit_with_tree(repo.store(), tree2);

    // Checkout should fail on icase fs.
    let ws = &mut test_workspace.workspace;
    let result = ws.check_out(repo.op_id().clone(), None, &commit1);
    if is_icase_fs {
        assert_matches!(result, Err(CheckoutError::ReservedPathComponent { .. }));
    } else {
        assert_matches!(result, Ok(_));
    }

    // Therefore, "pwned" file shouldn't be created.
    if is_icase_fs {
        assert!(!disk_path.exists());
    }
    assert!(!workspace_root.join(".git").join("pwned").exists());
    assert!(!workspace_root.join(".jj").join("pwned").exists());
    assert!(!workspace_root.join("sub").join(".git").exists());
    assert!(!workspace_root.join("sub").join(".jj").exists());

    // Pretend that the checkout somehow succeeded.
    let mut locked_ws = ws.start_working_copy_mutation().unwrap();
    locked_ws.locked_wc().reset(&commit1).block_on().unwrap();
    locked_ws.finish(repo.op_id().clone()).unwrap();
    std::fs::create_dir_all(disk_path.parent().unwrap()).unwrap();
    std::fs::write(&disk_path, "").unwrap();

    // Check out empty tree, which tries to remove the file.
    let result = ws.check_out(repo.op_id().clone(), None, &commit2);
    if is_icase_fs {
        assert_matches!(result, Err(CheckoutError::ReservedPathComponent { .. }));
    } else {
        assert_matches!(result, Ok(_));
    }

    // The existing file shouldn't be removed on icase fs.
    if is_icase_fs {
        assert!(disk_path.exists());
    }
}

// Here we don't test ignored characters exhaustively because our implementation
// isn't using deny list.
#[test_case("\u{200c}.git/pwned"; "root .git dir")]
#[test_case(".\u{200d}jj/pwned"; "root .jj dir")]
#[test_case("sub/.g\u{200c}it"; "sub .git file")]
#[test_case("sub/.jj\u{200d}"; "sub .jj file")]
#[test_case("sub/.gi\u{200e}t/pwned"; "sub .git dir")]
#[test_case("sub/.jj\u{200f}/pwned"; "sub .jj dir")]
fn test_check_out_reserved_file_path_hfs_plus(file_path_str: &str) {
    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();
    std::fs::create_dir(workspace_root.join(".git")).unwrap();
    let is_hfs_plus = check_hfs_plus(&workspace_root);

    let file_path = repo_path(file_path_str);
    let disk_path = file_path.to_fs_path_unchecked(&workspace_root);
    let tree1 = create_tree(repo, &[(file_path, "contents")]);
    let tree2 = create_tree(repo, &[]);
    let commit1 = commit_with_tree(repo.store(), tree1);
    let commit2 = commit_with_tree(repo.store(), tree2);

    // Checkout should fail on HFS+-like fs.
    let ws = &mut test_workspace.workspace;
    let result = ws.check_out(repo.op_id().clone(), None, &commit1);
    if is_hfs_plus {
        assert_matches!(result, Err(CheckoutError::ReservedPathComponent { .. }));
    } else {
        assert_matches!(result, Ok(_));
    }

    // Therefore, "pwned" file shouldn't be created.
    if is_hfs_plus {
        assert!(!disk_path.exists());
    }
    assert!(!workspace_root.join(".git").join("pwned").exists());
    assert!(!workspace_root.join(".jj").join("pwned").exists());
    assert!(!workspace_root.join("sub").join(".git").exists());
    assert!(!workspace_root.join("sub").join(".jj").exists());

    // Pretend that the checkout somehow succeeded.
    let mut locked_ws = ws.start_working_copy_mutation().unwrap();
    locked_ws.locked_wc().reset(&commit1).block_on().unwrap();
    locked_ws.finish(repo.op_id().clone()).unwrap();
    std::fs::create_dir_all(disk_path.parent().unwrap()).unwrap();
    std::fs::write(&disk_path, "").unwrap();

    // Check out empty tree, which tries to remove the file.
    let result = ws.check_out(repo.op_id().clone(), None, &commit2);
    if is_hfs_plus {
        assert_matches!(result, Err(CheckoutError::ReservedPathComponent { .. }));
    } else {
        assert_matches!(result, Ok(_));
    }

    // The existing file shouldn't be removed on HFS+-like fs.
    if is_hfs_plus {
        assert!(disk_path.exists());
    }
}

#[test_case(".git/pwned", &["GIT~1/pwned", "GI2837~1/pwned"]; "root .git dir short name")]
#[test_case(".jj/pwned", &["JJ~1/pwned", "JJ2E09~1/pwned"]; "root .jj dir short name")]
#[test_case(".git/pwned", &[".GIT./pwned"]; "root .git dir trailing dots")]
#[test_case(".jj/pwned", &[".JJ../pwned"]; "root .jj dir trailing dots")]
#[test_case("sub/.git", &["sub/.GIT.."]; "sub .git file trailing dots")]
#[test_case("sub/.jj", &["sub/.JJ."]; "sub .jj file trailing dots")]
// TODO: Add more weird patterns?
// - https://en.wikipedia.org/wiki/8.3_filename
// - See is_ntfs_dotgit() of Git and pathauditor of Mercurial
fn test_check_out_reserved_file_path_vfat(vfat_path_str: &str, file_path_strs: &[&str]) {
    let mut test_workspace = TestWorkspace::init();
    let repo = &test_workspace.repo;
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();
    std::fs::create_dir(workspace_root.join(".git")).unwrap();
    let is_vfat = check_vfat(&workspace_root);

    let vfat_disk_path = workspace_root.join(vfat_path_str);
    let file_paths = file_path_strs.iter().map(|&s| repo_path(s)).collect_vec();
    let tree1 = create_tree_with(repo, |builder| {
        for path in file_paths {
            builder.file(path, "contents");
        }
    });
    let tree2 = create_tree(repo, &[]);
    let commit1 = commit_with_tree(repo.store(), tree1);
    let commit2 = commit_with_tree(repo.store(), tree2);

    // Checkout should fail on VFAT-like fs.
    let ws = &mut test_workspace.workspace;
    let result = ws.check_out(repo.op_id().clone(), None, &commit1);
    if is_vfat {
        assert_matches!(result, Err(CheckoutError::ReservedPathComponent { .. }));
    } else {
        assert_matches!(result, Ok(_));
    }

    // Therefore, "pwned" file shouldn't be created.
    if is_vfat {
        assert!(!vfat_disk_path.exists());
    }
    assert!(!workspace_root.join(".git").join("pwned").exists());
    assert!(!workspace_root.join(".jj").join("pwned").exists());
    assert!(!workspace_root.join("sub").join(".git").exists());
    assert!(!workspace_root.join("sub").join(".jj").exists());

    // Pretend that the checkout somehow succeeded.
    let mut locked_ws = ws.start_working_copy_mutation().unwrap();
    locked_ws.locked_wc().reset(&commit1).block_on().unwrap();
    locked_ws.finish(repo.op_id().clone()).unwrap();
    if is_vfat {
        std::fs::create_dir_all(vfat_disk_path.parent().unwrap()).unwrap();
        std::fs::write(&vfat_disk_path, "").unwrap();
    }

    // Check out empty tree, which tries to remove the file.
    let result = ws.check_out(repo.op_id().clone(), None, &commit2);
    if is_vfat {
        assert_matches!(result, Err(CheckoutError::ReservedPathComponent { .. }));
    } else {
        assert_matches!(result, Ok(_));
    }

    // The existing file shouldn't be removed on VFAT-like fs.
    if is_vfat {
        assert!(vfat_disk_path.exists());
    }
}

#[test]
fn test_fsmonitor() {
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;
    let workspace_root = test_repo.env.root().join("workspace");
    let state_path = test_repo.env.root().join("state");
    std::fs::create_dir(&workspace_root).unwrap();
    std::fs::create_dir(&state_path).unwrap();
    let tree_state_settings = TreeStateSettings::try_from_user_settings(repo.settings()).unwrap();
    TreeState::init(
        repo.store().clone(),
        workspace_root.clone(),
        state_path.clone(),
        &tree_state_settings,
    )
    .unwrap();

    let foo_path = repo_path("foo");
    let bar_path = repo_path("bar");
    let nested_path = repo_path("path/to/nested");
    testutils::write_working_copy_file(&workspace_root, foo_path, "foo\n");
    testutils::write_working_copy_file(&workspace_root, bar_path, "bar\n");
    testutils::write_working_copy_file(&workspace_root, nested_path, "nested\n");

    let ignored_path = repo_path("path/to/ignored");
    let gitignore_path = repo_path("path/.gitignore");
    testutils::write_working_copy_file(&workspace_root, ignored_path, "ignored\n");
    testutils::write_working_copy_file(&workspace_root, gitignore_path, "to/ignored\n");

    let snapshot = |paths: &[&RepoPath]| {
        let changed_files = paths
            .iter()
            .map(|p| p.to_fs_path_unchecked(Path::new("")))
            .collect();
        let settings = TreeStateSettings {
            fsmonitor_settings: FsmonitorSettings::Test { changed_files },
            ..tree_state_settings.clone()
        };
        let mut tree_state = TreeState::load(
            repo.store().clone(),
            workspace_root.clone(),
            state_path.clone(),
            &settings,
        )
        .unwrap();
        tree_state.snapshot(&empty_snapshot_options()).unwrap();
        tree_state
    };

    let tree_state = snapshot(&[]);
    assert_tree_eq!(*tree_state.current_tree(), repo.store().empty_merged_tree());

    let tree_state = snapshot(&[foo_path]);
    insta::assert_snapshot!(testutils::dump_tree(tree_state.current_tree()), @r#"
    tree 2a5341b103917cfdb48a
      file "foo" (e99c2057c15160add351): "foo\n"
    "#);

    let mut tree_state = snapshot(&[foo_path, bar_path, nested_path, ignored_path]);
    insta::assert_snapshot!(testutils::dump_tree(tree_state.current_tree()), @r#"
    tree 1c5c336421714b1df7bb
      file "bar" (94cc973e7e1aefb7eff6): "bar\n"
      file "foo" (e99c2057c15160add351): "foo\n"
      file "path/to/nested" (6209060941cd770c8d46): "nested\n"
    "#);
    tree_state.save().unwrap();

    testutils::write_working_copy_file(&workspace_root, foo_path, "updated foo\n");
    testutils::write_working_copy_file(&workspace_root, bar_path, "updated bar\n");
    let tree_state = snapshot(&[foo_path]);
    insta::assert_snapshot!(testutils::dump_tree(tree_state.current_tree()), @r#"
    tree f653dfa18d0b025bdb9e
      file "bar" (94cc973e7e1aefb7eff6): "bar\n"
      file "foo" (e0fbd106147cc04ccd05): "updated foo\n"
      file "path/to/nested" (6209060941cd770c8d46): "nested\n"
    "#);

    std::fs::remove_file(foo_path.to_fs_path_unchecked(&workspace_root)).unwrap();
    let mut tree_state = snapshot(&[foo_path]);
    insta::assert_snapshot!(testutils::dump_tree(tree_state.current_tree()), @r#"
    tree b7416fc248a038b920c3
      file "bar" (94cc973e7e1aefb7eff6): "bar\n"
      file "path/to/nested" (6209060941cd770c8d46): "nested\n"
    "#);
    tree_state.save().unwrap();
}

#[test]
fn test_snapshot_max_new_file_size() {
    let mut test_workspace = TestWorkspace::init();
    let workspace_root = test_workspace.workspace.workspace_root().to_owned();
    let small_path = repo_path("small");
    let large_path = repo_path("large");
    let limit: usize = 1024;
    std::fs::write(
        small_path.to_fs_path_unchecked(&workspace_root),
        vec![0; limit],
    )
    .unwrap();
    let options = SnapshotOptions {
        max_new_file_size: limit as u64,
        ..empty_snapshot_options()
    };
    test_workspace
        .snapshot_with_options(&options)
        .expect("files exactly matching the size limit should succeed");
    std::fs::write(
        small_path.to_fs_path_unchecked(&workspace_root),
        vec![0; limit + 1],
    )
    .unwrap();
    let (old_tree, _stats) = test_workspace
        .snapshot_with_options(&options)
        .expect("existing files may grow beyond the size limit");

    // A new file of 1KiB + 1 bytes should be left untracked
    std::fs::write(
        large_path.to_fs_path_unchecked(&workspace_root),
        vec![0; limit + 1],
    )
    .unwrap();
    let (new_tree, stats) = test_workspace
        .snapshot_with_options(&options)
        .expect("snapshot should not fail because of new files beyond the size limit");
    assert_tree_eq!(new_tree, old_tree);
    assert_eq!(
        stats
            .untracked_paths
            .keys()
            .map(AsRef::as_ref)
            .collect_vec(),
        [large_path]
    );
    assert_matches!(
        stats.untracked_paths.values().next().unwrap(),
        UntrackedReason::FileTooLarge { size, .. } if *size == (limit as u64) + 1
    );

    // A file in sub directory should also be caught
    let sub_large_path = repo_path("sub/large");
    std::fs::create_dir(
        sub_large_path
            .parent()
            .unwrap()
            .to_fs_path_unchecked(&workspace_root),
    )
    .unwrap();
    std::fs::rename(
        large_path.to_fs_path_unchecked(&workspace_root),
        sub_large_path.to_fs_path_unchecked(&workspace_root),
    )
    .unwrap();
    let (new_tree, stats) = test_workspace
        .snapshot_with_options(&options)
        .expect("snapshot should not fail because of new files beyond the size limit");
    assert_tree_eq!(new_tree, old_tree);
    assert_eq!(
        stats
            .untracked_paths
            .keys()
            .map(AsRef::as_ref)
            .collect_vec(),
        [sub_large_path]
    );
    assert_matches!(
        stats.untracked_paths.values().next().unwrap(),
        UntrackedReason::FileTooLarge { .. }
    );
}
