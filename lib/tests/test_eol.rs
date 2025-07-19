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

use std::fs::File;
use std::io::Write as _;
use std::path::Path;

use bstr::ByteSlice as _;
use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigSource;
use jj_lib::conflicts::MaterializedTreeValue;
use jj_lib::files::MergeResult;
use jj_lib::repo::Repo as _;
use jj_lib::repo::StoreFactories;
use jj_lib::rewrite::merge_commit_trees;
use jj_lib::settings::UserSettings;
use jj_lib::working_copy::CheckoutOptions;
use jj_lib::workspace::Workspace;
use jj_lib::workspace::default_working_copy_factories;
use pollster::FutureExt as _;
use test_case::test_case;
use testutils::TestRepoBackend;
use testutils::TestWorkspace;
use testutils::base_user_config;
use testutils::commit_with_tree;
use testutils::repo_path;
use tokio::io::AsyncReadExt as _;

static LF_FILE_CONTENT: &[u8] = b"aaa\nbbbb\nccccc\n";
static CRLF_FILE_CONTENT: &[u8] = b"aaa\r\nbbbb\r\nccccc\r\n";
static MIXED_EOL_FILE_CONTENT: &[u8] = b"aaa\nbbbb\r\nccccc\n";
static BINARY_FILE_CONTENT: &[u8] = b"\0";

struct Config {
    extra_setting: &'static str,
    file_content: &'static [u8],
}

fn base_user_settings_with_extra_configs(extra_settings: &str) -> UserSettings {
    let mut config = base_user_config();
    config.add_layer(
        ConfigLayer::parse(ConfigSource::User, extra_settings)
            .expect("Failed to parse the settings"),
    );
    UserSettings::from_config(config).expect("Failed to create the UserSettings from the config")
}

#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "input-output""#,
    file_content: LF_FILE_CONTENT,
} => LF_FILE_CONTENT; "eol-conversion input-output LF only file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "input-output""#,
    file_content: CRLF_FILE_CONTENT,
} => LF_FILE_CONTENT; "eol-conversion input-output CRLF only file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "input-output""#,
    file_content: MIXED_EOL_FILE_CONTENT,
} => LF_FILE_CONTENT; "eol-conversion input-output mixed EOL file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "input-output""#,
    file_content: BINARY_FILE_CONTENT,
} => BINARY_FILE_CONTENT; "eol-conversion input-output binary file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "input""#,
    file_content: LF_FILE_CONTENT,
} => LF_FILE_CONTENT; "eol-conversion input LF only file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "input""#,
    file_content: CRLF_FILE_CONTENT,
} => LF_FILE_CONTENT; "eol-conversion input CRLF only file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "input""#,
    file_content: MIXED_EOL_FILE_CONTENT,
} => LF_FILE_CONTENT; "eol-conversion input mixed EOL file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "input""#,
    file_content: BINARY_FILE_CONTENT,
} => BINARY_FILE_CONTENT; "eol-conversion input binary file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "none""#,
    file_content: LF_FILE_CONTENT,
} => LF_FILE_CONTENT; "eol-conversion none LF only file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "none""#,
    file_content: CRLF_FILE_CONTENT,
} => CRLF_FILE_CONTENT; "eol-conversion none CRLF only file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "none""#,
    file_content: MIXED_EOL_FILE_CONTENT,
} => MIXED_EOL_FILE_CONTENT; "eol-conversion none mixed EOL file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "none""#,
    file_content: BINARY_FILE_CONTENT,
} => BINARY_FILE_CONTENT; "eol-conversion none binary file")]
fn test_eol_conversion_snapshot(
    Config {
        extra_setting,
        file_content,
    }: Config,
) -> Vec<u8> {
    // This test creates snapshots with different working-copy.eol-conversion
    // configurations, where proper EOL conversion should apply before writing files
    // back to the store. Then files are checked out with
    // working-copy.eol-conversion = "none", which won't touch the EOLs, so that we
    // can tell whether the exact EOLs written to the store are expected.

    let extra_setting = format!("{extra_setting}\n");
    let user_settings = base_user_settings_with_extra_configs(&extra_setting);
    let mut test_workspace =
        TestWorkspace::init_with_backend_and_settings(TestRepoBackend::Git, &user_settings);
    let file_repo_path = repo_path("test-eol-file");
    let file_disk_path = file_repo_path
        .to_fs_path(test_workspace.workspace.workspace_root())
        .unwrap();

    testutils::write_working_copy_file(
        test_workspace.workspace.workspace_root(),
        file_repo_path,
        file_content,
    );
    let tree = test_workspace.snapshot().unwrap();
    let new_tree = test_workspace.snapshot().unwrap();
    assert_eq!(
        new_tree.id(),
        tree.id(),
        "The working copy should be clean."
    );
    let file_added_commit = commit_with_tree(test_workspace.repo.store(), tree.id());

    // Create a commit with the file removed, so that later when we checkout the
    // file_added_commit, the test file is recreated.
    std::fs::remove_file(&file_disk_path).unwrap();
    let tree = test_workspace.snapshot().unwrap();
    let file_removed_commit = commit_with_tree(test_workspace.repo.store(), tree.id());
    let workspace = &mut test_workspace.workspace;
    workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &file_removed_commit,
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();
    assert!(!file_disk_path.exists());

    let user_settings =
        base_user_settings_with_extra_configs("working-copy.eol-conversion = \"none\"\n");
    // Reload the workspace with the new working-copy.eol-conversion = "none"
    // setting to verify the EOL of files previously written to the store.
    let mut workspace = Workspace::load(
        &user_settings,
        test_workspace.workspace.workspace_root(),
        &StoreFactories::default(),
        &default_working_copy_factories(),
    )
    .expect("Failed to reload the workspace");
    // We have to query the Commit again. The Workspace is backed by a different
    // Store from the original Commit.
    let file_added_commit = workspace
        .repo_loader()
        .store()
        .get_commit(file_added_commit.id())
        .expect("Failed to find the commit with the test file");
    workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &file_added_commit,
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();
    assert!(file_disk_path.exists());
    let new_tree = test_workspace.snapshot().unwrap();
    assert_eq!(
        new_tree.id(),
        *file_added_commit.tree_id(),
        "The working copy should be clean."
    );

    std::fs::read(&file_disk_path).expect("Failed to read the checked out test file")
}

struct ConflictSnapshotTestConfig {
    parent1_contents: &'static str,
    parent2_contents: &'static str,
    contents_to_append: &'static [u8],
    merge_snapshot_setting: &'static str,

    check_eol_is_lf: bool,
    expected_conflict_side1: &'static str,
    expected_conflict_side2: &'static str,
    expected_appended_contents: &'static str,
}

#[test_case(ConflictSnapshotTestConfig {
    parent1_contents: "a\n",
    parent2_contents: "b\n",
    contents_to_append: b"c\r\n",
    merge_snapshot_setting: r#"working-copy.eol-conversion = "input-output""#,
    check_eol_is_lf: true,
    expected_conflict_side1: "a\n",
    expected_conflict_side2: "b\n",
    expected_appended_contents: "c\n",
}; "input output setting with CRLF contents appended")]
#[test_case(ConflictSnapshotTestConfig {
    parent1_contents: "a\n",
    parent2_contents: "b\n",
    contents_to_append: b"c\r\n",
    merge_snapshot_setting: r#"working-copy.eol-conversion = "input""#,
    check_eol_is_lf: true,
    expected_conflict_side1: "a\n",
    expected_conflict_side2: "b\n",
    expected_appended_contents: "c\n",
}; "input setting with CRLF contents appended")]
#[test_case(ConflictSnapshotTestConfig {
    parent1_contents: "a\n",
    parent2_contents: "b\n",
    contents_to_append: b"c\r\n",
    merge_snapshot_setting: r#"working-copy.eol-conversion = "none""#,
    check_eol_is_lf: false,
    expected_conflict_side1: "a\n",
    expected_conflict_side2: "b\n",
    expected_appended_contents: "c\r\n",
}; "none setting with CRLF contents appended")]
#[test_case(ConflictSnapshotTestConfig {
    parent1_contents: "a\n",
    parent2_contents: "b\r\n",
    contents_to_append: b"c\r\nd\n",
    merge_snapshot_setting: r#"working-copy.eol-conversion = "input-output""#,
    check_eol_is_lf: false,
    expected_conflict_side1: "a\n",
    expected_conflict_side2: "b\r\n",
    expected_appended_contents: "c\r\nd\n",
}; "input output setting with CRLF conflicts in store")]
#[test_case(ConflictSnapshotTestConfig {
    parent1_contents: "a\n",
    parent2_contents: "b\r\n",
    contents_to_append: b"c\r\nd\n",
    merge_snapshot_setting: r#"working-copy.eol-conversion = "input""#,
    check_eol_is_lf: false,
    expected_conflict_side1: "a\n",
    expected_conflict_side2: "b\r\n",
    expected_appended_contents: "c\r\nd\n",
}; "input setting with CRLF conflicts in store")]
#[test_case(ConflictSnapshotTestConfig {
    parent1_contents: "a\r\n",
    parent2_contents: "b\r\n",
    contents_to_append: b"c\r\nd\n",
    merge_snapshot_setting: r#"working-copy.eol-conversion = "none""#,
    check_eol_is_lf: false,
    expected_conflict_side1: "a\r\n",
    expected_conflict_side2: "b\r\n",
    expected_appended_contents: "c\r\nd\n",
}; "none setting with CRLF conflicts in store")]
fn test_eol_snapshot_after_editing_conflict(
    ConflictSnapshotTestConfig {
        parent1_contents,
        parent2_contents,
        contents_to_append,
        merge_snapshot_setting,
        check_eol_is_lf,
        expected_conflict_side1,
        expected_conflict_side2,
        expected_appended_contents,
    }: ConflictSnapshotTestConfig,
) {
    // Create a conflict commit with the given contents as is in the Store, and
    // append another line to the conflict file, create a snapshot on the modified
    // merge conflict under the test setting, checkout the snapshot with the given
    // setting, and return the content of the file content in Store.

    let user_settings =
        base_user_settings_with_extra_configs(&format!("{merge_snapshot_setting}\n"));
    let mut test_workspace =
        TestWorkspace::init_with_backend_and_settings(TestRepoBackend::Git, &user_settings);
    let file_repo_path = repo_path("test-eol-file");
    let file_disk_path = file_repo_path
        .to_fs_path(test_workspace.workspace.workspace_root())
        .unwrap();

    // The commit graph:
    // C (conflict)
    // |\
    // A B
    // |/
    // (empty)
    let root_commit = test_workspace.repo.store().root_commit();
    let mut tx = test_workspace.repo.start_transaction();
    let tree = testutils::create_tree(&test_workspace.repo, &[(file_repo_path, parent1_contents)]);
    let parent1_commit = tx
        .repo_mut()
        .new_commit(vec![root_commit.id().clone()], tree.id())
        .write()
        .unwrap();
    let tree = testutils::create_tree(&test_workspace.repo, &[(file_repo_path, parent2_contents)]);
    let parent2_commit = tx
        .repo_mut()
        .new_commit(vec![root_commit.id().clone()], tree.id())
        .write()
        .unwrap();
    tx.commit("commit parents").unwrap();

    // Reload the repo to pick up the new commits.
    test_workspace.repo = test_workspace.repo.reload_at_head().unwrap();
    // Create the merge commit.
    let tree = merge_commit_trees(&*test_workspace.repo, &[parent1_commit, parent2_commit])
        .block_on()
        .unwrap();
    let merge_commit = commit_with_tree(test_workspace.repo.store(), tree.id());

    // Checkout the merge commit.
    test_workspace
        .workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &merge_commit,
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();
    let mut file = File::options().append(true).open(&file_disk_path).unwrap();
    file.write_all(contents_to_append).unwrap();
    drop(file);

    let tree = test_workspace.snapshot().unwrap();
    let new_tree = test_workspace.snapshot().unwrap();
    assert_eq!(
        new_tree.id(),
        tree.id(),
        "The working copy should be clean."
    );
    // Create the new merge commit with the conflict file appended.
    let merge_commit = commit_with_tree(test_workspace.repo.store(), tree.id());

    // Reload the Workspace with the working-copy.eol-conversion = "none" setting to
    // check the EOL of the file written to the store previously.
    let no_eol_conversion_settings =
        base_user_settings_with_extra_configs("working-copy.eol-conversion = \"none\"\n");
    test_workspace.workspace = Workspace::load(
        &no_eol_conversion_settings,
        test_workspace.workspace.workspace_root(),
        &StoreFactories::default(),
        &default_working_copy_factories(),
    )
    .expect("Failed to reload the workspace");
    // Checkout the empty commit to clear the directory, so that the test file will
    // be recreated.
    test_workspace
        .workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &test_workspace.workspace.repo_loader().store().root_commit(),
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();
    // We have to query the Commit again. The Workspace is backed by a different
    // Store from the original Commit.
    let merge_commit = test_workspace
        .workspace
        .repo_loader()
        .store()
        .get_commit(merge_commit.id())
        .expect("Failed to find the commit with the test file");
    test_workspace
        .workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &merge_commit,
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();

    assert!(std::fs::exists(&file_disk_path).unwrap());
    let contents = std::fs::read(&file_disk_path).unwrap();
    let hunks =
        jj_lib::conflicts::parse_conflict(&contents, 2, jj_lib::conflicts::MIN_CONFLICT_MARKER_LEN)
            .unwrap();
    let conflict = hunks[0].clone();
    let conflict_sides = conflict.iter().collect::<Vec<_>>();
    let conflict_side1 = conflict_sides[0].to_str().unwrap();
    let conflict_side2 = conflict_sides[2].to_str().unwrap();
    let appended_contents = hunks[1]
        .clone()
        .into_resolved()
        .expect("The second hunk is the added contents, which should be resolved.");
    let appended_contents = appended_contents.to_str().unwrap();
    assert_eq!(conflict_side1, expected_conflict_side1);
    assert_eq!(conflict_side2, expected_conflict_side2);
    assert_eq!(appended_contents, expected_appended_contents);
    if check_eol_is_lf {
        for line in contents.lines_with_terminator() {
            assert!(
                !line.ends_with(b"\r\n"),
                "{:?} should not end with CRLF",
                &*line.to_str_lossy()
            );
            assert!(
                line.ends_with(b"\n"),
                "{:?} should end with LF",
                &*line.to_str_lossy()
            );
        }
    }
}

struct UpdateConflictsTestConfig {
    parent1_contents: &'static str,
    parent2_contents: &'static str,
    extra_setting: &'static str,

    expected_eol: Option<&'static str>,
    expected_conflict_side1: &'static str,
    expected_conflict_side2: &'static str,
}

#[test_case(UpdateConflictsTestConfig {
    parent1_contents: "a\n",
    parent2_contents: "b\n",
    extra_setting: r#"working-copy.eol-conversion = "none""#,
    expected_eol: Some("\n"),
    expected_conflict_side1: "a\n",
    expected_conflict_side2: "b\n",
}; "LF parents with none settings")]
#[test_case(UpdateConflictsTestConfig {
    parent1_contents: "a\n",
    parent2_contents: "b\n",
    extra_setting: r#"working-copy.eol-conversion = "input-output""#,
    expected_eol: Some("\r\n"),
    expected_conflict_side1: "a\r\n",
    expected_conflict_side2: "b\r\n",
}; "LF parents with input-output settings")]
#[test_case(UpdateConflictsTestConfig {
    parent1_contents: "a\r\n",
    parent2_contents: "b\r\n",
    extra_setting: r#"working-copy.eol-conversion = "input-output""#,
    expected_eol: None,
    expected_conflict_side1: "a\r\n",
    expected_conflict_side2: "b\r\n",
}; "CRLF parents with input-output settings")]
#[test_case(UpdateConflictsTestConfig {
    parent1_contents: "a\n",
    parent2_contents: "b\r\n",
    extra_setting: r#"working-copy.eol-conversion = "input-output""#,
    expected_eol: None,
    expected_conflict_side1: "a\n",
    expected_conflict_side2: "b\r\n",
}; "Mixed EOL parents with input-output settings #1")]
#[test_case(UpdateConflictsTestConfig {
    parent1_contents: "a\r\n",
    parent2_contents: "b\n",
    extra_setting: r#"working-copy.eol-conversion = "input-output""#,
    expected_eol: None,
    expected_conflict_side1: "a\r\n",
    expected_conflict_side2: "b\n",
}; "Mixed EOL parents with input-output settings #2")]
fn test_eol_conversion_update_conflicts(
    UpdateConflictsTestConfig {
        parent1_contents,
        parent2_contents,
        extra_setting,
        expected_eol,
        expected_conflict_side1,
        expected_conflict_side2,
    }: UpdateConflictsTestConfig,
) {
    // Create a conflict commit with 2 given contents on one file, checkout that
    // conflict with the given EOL conversion settings, and test if the EOL matches.

    let extra_setting = format!("{extra_setting}\n");
    let user_settings = base_user_settings_with_extra_configs(&extra_setting);
    let mut test_workspace =
        TestWorkspace::init_with_backend_and_settings(TestRepoBackend::Git, &user_settings);
    let file_repo_path = repo_path("test-eol-file");
    let file_disk_path = file_repo_path
        .to_fs_path(test_workspace.workspace.workspace_root())
        .unwrap();

    // The commit graph:
    // C (conflict)
    // |\
    // A B
    // |/
    // (empty)
    let root_commit = test_workspace.repo.store().root_commit();
    let mut tx = test_workspace.repo.start_transaction();
    let tree = testutils::create_tree(&test_workspace.repo, &[(file_repo_path, parent1_contents)]);
    let parent1_commit = tx
        .repo_mut()
        .new_commit(vec![root_commit.id().clone()], tree.id())
        .write()
        .unwrap();
    let tree = testutils::create_tree(&test_workspace.repo, &[(file_repo_path, parent2_contents)]);
    let parent2_commit = tx
        .repo_mut()
        .new_commit(vec![root_commit.id().clone()], tree.id())
        .write()
        .unwrap();
    tx.commit("commit parents").unwrap();

    // Reload the repo to pick up the new commits.
    test_workspace.repo = test_workspace.repo.reload_at_head().unwrap();
    // Create the merge commit.
    let tree = merge_commit_trees(&*test_workspace.repo, &[parent1_commit, parent2_commit])
        .block_on()
        .unwrap();
    let merge_commit = commit_with_tree(test_workspace.repo.store(), tree.id());

    // Checkout the merge commit.
    test_workspace
        .workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &merge_commit,
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();
    let contents = std::fs::read(&file_disk_path).unwrap();
    if let Some(expected_eol) = expected_eol {
        for line in contents.lines_with_terminator() {
            assert!(
                line.ends_with_str(expected_eol),
                "{:?} should end with {:?}",
                &*line.to_str_lossy(),
                expected_eol
            );
        }
    }
    let hunks =
        jj_lib::conflicts::parse_conflict(&contents, 2, jj_lib::conflicts::MIN_CONFLICT_MARKER_LEN)
            .unwrap();
    let hunk = &hunks[0];
    assert!(!hunk.is_resolved());
    let sides = hunk.iter().collect::<Vec<_>>();
    assert_eq!(sides[0], expected_conflict_side1);
    assert_eq!(sides[2], expected_conflict_side2);
}

#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "input-output""#,
    file_content: LF_FILE_CONTENT,
} => CRLF_FILE_CONTENT; "eol-conversion input-output LF only file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "input-output""#,
    file_content: CRLF_FILE_CONTENT,
} => CRLF_FILE_CONTENT; "eol-conversion input-output CRLF only file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "input-output""#,
    file_content: MIXED_EOL_FILE_CONTENT,
} => MIXED_EOL_FILE_CONTENT; "eol-conversion input-output mixed EOL file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "input-output""#,
    file_content: BINARY_FILE_CONTENT,
} => BINARY_FILE_CONTENT; "eol-conversion input-output binary file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "input""#,
    file_content: LF_FILE_CONTENT,
} => LF_FILE_CONTENT; "eol-conversion input LF only file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "input""#,
    file_content: CRLF_FILE_CONTENT,
} => CRLF_FILE_CONTENT; "eol-conversion input CRLF only file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "input""#,
    file_content: MIXED_EOL_FILE_CONTENT,
} => MIXED_EOL_FILE_CONTENT; "eol-conversion input mixed EOL file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "input""#,
    file_content: BINARY_FILE_CONTENT,
} => BINARY_FILE_CONTENT; "eol-conversion input binary file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "none""#,
    file_content: LF_FILE_CONTENT,
} => LF_FILE_CONTENT; "eol-conversion none LF only file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "none""#,
    file_content: CRLF_FILE_CONTENT,
} => CRLF_FILE_CONTENT; "eol-conversion none CRLF only file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "none""#,
    file_content: MIXED_EOL_FILE_CONTENT,
} => MIXED_EOL_FILE_CONTENT; "eol-conversion none mixed EOL file")]
#[test_case(Config {
    extra_setting: r#"working-copy.eol-conversion = "none""#,
    file_content: BINARY_FILE_CONTENT,
} => BINARY_FILE_CONTENT; "eol-conversion none binary file")]
fn test_eol_conversion_checkout(
    Config {
        extra_setting,
        file_content,
    }: Config,
) -> Vec<u8> {
    // This test checks in files with working-copy.eol-conversion = "none", so that
    // the store stores files as is. Then we use jj to check out those files with
    // different working-copy.eol-conversion configurations to verify if the EOLs
    // are converted as expected.

    let no_eol_conversion_settings =
        base_user_settings_with_extra_configs("working-copy.eol-conversion = \"none\"\n");
    // Use the working-copy.eol-conversion = "none" setting, so that the input files
    // are stored as is.
    let mut test_workspace = TestWorkspace::init_with_backend_and_settings(
        TestRepoBackend::Git,
        &no_eol_conversion_settings,
    );
    let file_repo_path = repo_path("test-eol-file");
    let file_disk_path = file_repo_path
        .to_fs_path(test_workspace.workspace.workspace_root())
        .unwrap();
    testutils::write_working_copy_file(
        test_workspace.workspace.workspace_root(),
        file_repo_path,
        file_content,
    );
    let tree = test_workspace.snapshot().unwrap();
    let commit = commit_with_tree(test_workspace.repo.store(), tree.id());

    // Checkout the empty commit to clear the directory, so that later when we
    // checkout, files are recreated.
    test_workspace
        .workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &test_workspace.workspace.repo_loader().store().root_commit(),
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();
    assert!(!std::fs::exists(&file_disk_path).unwrap());

    let extra_setting = format!("{extra_setting}\n");
    let user_settings = base_user_settings_with_extra_configs(&extra_setting);
    // Change the working-copy.eol-conversion setting to the configuration under
    // testing.
    test_workspace.workspace = Workspace::load(
        &user_settings,
        test_workspace.workspace.workspace_root(),
        &StoreFactories::default(),
        &default_working_copy_factories(),
    )
    .expect("Failed to reload the workspace");
    // We have to query the Commit again. The Workspace is backed by a different
    // Store from the original Commit.
    let commit = test_workspace
        .workspace
        .repo_loader()
        .store()
        .get_commit(commit.id())
        .expect("Failed to find the commit with the test file");
    // Check out the commit with the test file. TreeState::update should update the
    // EOL accordingly.
    test_workspace
        .workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &commit,
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();

    // When we take a snapshot now, the tree may not be clean, because the EOL our
    // snapshot creates may not align with what is currently used in store. e.g.
    // with working-copy.eol-conversion = "input-output", the test-eol-file may have
    // CRLF line endings in the store, but the snapshot will change the EOL to LF,
    // hence the diff.

    assert!(std::fs::exists(&file_disk_path).unwrap());
    std::fs::read(&file_disk_path).unwrap()
}

#[test_case(b"a\r\n", b"b\r\n", b"a\r\nb\r\n"; "CRLF file appended")]
#[test_case(b"a\r\nb\n", b"c\r\n", b"a\r\nb\nc\r\n"; "Mixed EOL file appended")]
fn test_eol_crlf_files_in_store_should_not_be_modified(
    old_contents_in_store: &[u8],
    contents_to_append: &[u8],
    expected_new_contents_in_store: &[u8],
) {
    // In this test, we create a file with a line that ends with CRLF in Store.
    // Afterwards, we append another line that ends with CRLF to the file, and
    // take a snapshot with the EOL settings under tests. The file should still
    // have CRLF EOL in the Store.
    //
    // See https://github.com/jj-vcs/jj/issues/7010 for details on why we need
    // this test.

    // First we create a file with CRLF EOL in the Store by using the none EOL
    // conversion setting.
    let no_eol_conversion_settings =
        base_user_settings_with_extra_configs("working-copy.eol-conversion = \"none\"\n");
    // Use the working-copy.eol-conversion = "none" setting, so that the input files
    // are stored as is.
    let mut test_workspace = TestWorkspace::init_with_backend_and_settings(
        TestRepoBackend::Git,
        &no_eol_conversion_settings,
    );
    let file_repo_path = repo_path("test-eol-file");
    let file_disk_path = file_repo_path
        .to_fs_path(test_workspace.workspace.workspace_root())
        .unwrap();
    testutils::write_working_copy_file(
        test_workspace.workspace.workspace_root(),
        file_repo_path,
        old_contents_in_store,
    );
    let tree = test_workspace.snapshot().unwrap();
    let old_commit = commit_with_tree(test_workspace.repo.store(), tree.id());

    // Checkout the empty commit to clear the directory, so that later when we
    // checkout, files are recreated.
    test_workspace
        .workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &test_workspace.workspace.repo_loader().store().root_commit(),
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();
    assert!(!std::fs::exists(&file_disk_path).unwrap());

    // Then we append another line to the file with the EOL conversion setting under
    // test.
    let extra_setting = r#"working-copy.eol-conversion = "input-output""#;
    let user_settings = base_user_settings_with_extra_configs(extra_setting);
    // Change the working-copy.eol-conversion setting to the configuration under
    // testing.
    test_workspace.workspace = Workspace::load(
        &user_settings,
        test_workspace.workspace.workspace_root(),
        &StoreFactories::default(),
        &default_working_copy_factories(),
    )
    .expect("Failed to reload the workspace");
    // We have to query the Commit again. The Workspace is backed by a different
    // Store from the original Commit.
    let old_commit = test_workspace
        .workspace
        .repo_loader()
        .store()
        .get_commit(old_commit.id())
        .expect("Failed to find the commit with the test file");
    // Check out the commit with the test file.
    test_workspace
        .workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &old_commit,
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();
    assert!(std::fs::exists(&file_disk_path).unwrap());
    let mut file = File::options().append(true).open(&file_disk_path).unwrap();
    file.write_all(contents_to_append).unwrap();
    drop(file);
    let tree = test_workspace.snapshot().unwrap();
    let new_commit = commit_with_tree(test_workspace.repo.store(), tree.id());

    // Checkout the empty commit to clear the directory, so that later when we
    // checkout, files are recreated.
    test_workspace
        .workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &test_workspace.workspace.repo_loader().store().root_commit(),
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();
    assert!(!std::fs::exists(&file_disk_path).unwrap());

    // We use the working-copy.eol-conversion = "none" setting to checkout the
    // modified file, so that the files are checked out as is. And check if that
    // file still has CRLF EOL in the Store.
    test_workspace.workspace = Workspace::load(
        &no_eol_conversion_settings,
        test_workspace.workspace.workspace_root(),
        &StoreFactories::default(),
        &default_working_copy_factories(),
    )
    .expect("Failed to reload the workspace");
    // We have to query the Commit again. The Workspace is backed by a different
    // Store from the original Commit.
    let new_commit = test_workspace
        .workspace
        .repo_loader()
        .store()
        .get_commit(new_commit.id())
        .expect("Failed to find the commit with the test file");
    // Check out the commit with the test file.
    test_workspace
        .workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &new_commit,
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();
    assert!(std::fs::exists(&file_disk_path).unwrap());
    let contents = std::fs::read(&file_disk_path).unwrap();
    assert_eq!(
        contents,
        expected_new_contents_in_store,
        "Expected {:?}. Actual {:?}.",
        &*expected_new_contents_in_store.to_str_lossy(),
        &*contents.to_str_lossy()
    );
}

#[tokio::main(flavor = "current_thread")]
#[test_case("a\r\n", "aa\r\n", "aaa\r\n" => "aaa\r\n"; "has CRLF")]
#[test_case("a\r\n", "aa\n", "aaa\r\n" => "aaa\n"; "has only LF")]
async fn test_eol_snapshot_twice(
    original_contents: &str,
    first_snapshot_contents: &str,
    second_snapshot_contents: &str,
) -> String {
    // In this test, we use the input-output EOL conversion setting. We snapshot
    // twice with different contents and verify the file contents of the last
    // snapshot to make sure the first snapshot stores the correct EOL status.

    let user_settings =
        base_user_settings_with_extra_configs("working-copy.eol-conversion=\"input-output\"\n");
    let mut test_workspace =
        TestWorkspace::init_with_backend_and_settings(TestRepoBackend::Git, &user_settings);
    let file_repo_path = repo_path("test-eol-file");
    let file_disk_path = file_repo_path
        .to_fs_path(test_workspace.workspace.workspace_root())
        .unwrap();

    let root_commit = test_workspace.repo.store().root_commit();
    let mut tx = test_workspace.repo.start_transaction();
    let tree = testutils::create_tree(&test_workspace.repo, &[(file_repo_path, original_contents)]);
    let commit = tx
        .repo_mut()
        .new_commit(vec![root_commit.id().clone()], tree.id())
        .write()
        .unwrap();
    tx.commit("commit the initial commit").unwrap();
    test_workspace
        .workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &commit,
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();
    // Change the file to LF only and snapshot. Now the file in the Store has no
    // CRLF.
    assert!(file_disk_path.exists());
    std::fs::write(&file_disk_path, first_snapshot_contents).unwrap();
    test_workspace.snapshot().unwrap();
    // Change the file to contain CRLF and snapshot. Since the file has no CRLF in
    // the Store, the EOL conversion should be applied.
    std::fs::write(&file_disk_path, second_snapshot_contents).unwrap();
    let tree = test_workspace.snapshot().unwrap();
    let file_id = tree
        .path_value(file_repo_path)
        .unwrap()
        .to_file_merge()
        .unwrap()
        .into_resolved()
        .unwrap()
        .unwrap();
    let mut contents = String::new();
    test_workspace
        .repo
        .store()
        .read_file(file_repo_path, &file_id)
        .await
        .unwrap()
        .read_to_string(&mut contents)
        .await
        .unwrap();
    contents
}

struct SnapshotConflictsTwiceTestConfig {
    /// The contents appended after the conflict in the Store.
    original_appended_contents: &'static str,
    /// The contents appended after the conflict on the disk before the first
    /// snapshot.
    first_snapshot_appended_contents: &'static str,
    /// The contents appended after the conflict on the disk before the second
    /// snapshot.
    second_snapshot_appended_contents: &'static str,
}

#[tokio::main(flavor = "current_thread")]
#[test_case(SnapshotConflictsTwiceTestConfig {
    original_appended_contents: "a\r\n",
    first_snapshot_appended_contents: "aa\n",
    second_snapshot_appended_contents: "aaa\r\n",
} => "aaa\n"; "CRLF removed in the first snapshot")]
#[test_case(SnapshotConflictsTwiceTestConfig {
    original_appended_contents: "a\r\n",
    first_snapshot_appended_contents: "aa\r\n",
    second_snapshot_appended_contents: "aaa\r\n",
} => "aaa\r\n"; "CRLF is kept in the first snapshot")]
async fn test_eol_snapshot_conflicts_twice(
    SnapshotConflictsTwiceTestConfig {
        original_appended_contents,
        first_snapshot_appended_contents,
        second_snapshot_appended_contents,
    }: SnapshotConflictsTwiceTestConfig,
) -> String {
    // Similar to `test_eol_snapshot_twice`, but the file to snapshot contains
    // conflicts. The conflicts contain only LF EOLs. We change the EOL of the
    // contents that follow the conflicts for testing. We check the contents that
    // follow the conflicts to verify the EOL conversion behavior is correct.

    // We first use the none EOL conversion setting so that we can write to the
    // Store with the original EOL.
    let user_settings =
        base_user_settings_with_extra_configs("working-copy.eol-conversion=\"none\"\n");
    let mut test_workspace =
        TestWorkspace::init_with_backend_and_settings(TestRepoBackend::Git, &user_settings);
    let file_repo_path = repo_path("test-eol-file");
    let file_disk_path = file_repo_path
        .to_fs_path(test_workspace.workspace.workspace_root())
        .unwrap();

    // The commit graph:
    // E (modified conflict)
    // |
    // D (conflict)
    // |\
    // B C
    // |/
    // A

    let root_commit = test_workspace.repo.store().root_commit();
    let mut tx = test_workspace.repo.start_transaction();
    // We must prepare a non-empty base commit, so that when we update the conflict files, it won't be considered to edit the empty placeholder for an absent side and resolve the conflict on snapshot. See https://github.com/jj-vcs/jj/issues/7156 for details.
    let tree = testutils::create_tree(&test_workspace.repo, &[(file_repo_path, "a\n")]);
    let base_commit = tx
        .repo_mut()
        .new_commit(vec![root_commit.id().clone()], tree.id())
        .write()
        .unwrap();
    let tree = testutils::create_tree(&test_workspace.repo, &[(file_repo_path, "b\n")]);
    let parent1_commit = tx
        .repo_mut()
        .new_commit(vec![base_commit.id().clone()], tree.id())
        .write()
        .unwrap();
    let tree = testutils::create_tree(&test_workspace.repo, &[(file_repo_path, "c\n")]);
    let parent2_commit = tx
        .repo_mut()
        .new_commit(vec![base_commit.id().clone()], tree.id())
        .write()
        .unwrap();
    tx.commit("commit parents").unwrap();

    // Reload the repo to pick up the new commits.
    test_workspace.repo = test_workspace.repo.reload_at_head().unwrap();
    // Create the merge commit.
    let tree = merge_commit_trees(&*test_workspace.repo, &[parent1_commit, parent2_commit])
        .await
        .unwrap();
    let merge_commit = commit_with_tree(test_workspace.repo.store(), tree.id());

    // Checkout the merge commit.
    test_workspace
        .workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &merge_commit,
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();
    // We use the following CONFLICT_END_MARKERS to find the end of conflicts, so
    // that it's easier to modify the contents following the conflict hunk.
    const CONFLICT_END_MARKERS: &str = "========== <end of conflicts> ==========\n";
    // Modify the contents and snapshot for the original commit.
    let mut file = File::options().append(true).open(&file_disk_path).unwrap();
    file.write_all(CONFLICT_END_MARKERS.as_bytes()).unwrap();
    file.write_all(original_appended_contents.as_bytes())
        .unwrap();
    drop(file);
    let tree = test_workspace.snapshot().unwrap();
    let modified_commit = commit_with_tree(test_workspace.repo.store(), tree.id());

    // Clean the folder, change the EOL settings, reload the workspace, and checkout
    // the commit again.
    test_workspace
        .workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &root_commit,
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();
    let user_settings =
        base_user_settings_with_extra_configs("working-copy.eol-conversion=\"input-output\"\n");
    // Reload the workspace with the new working-copy.eol-conversion =
    // "input-output" setting.
    test_workspace.workspace = Workspace::load(
        &user_settings,
        test_workspace.workspace.workspace_root(),
        &StoreFactories::default(),
        &default_working_copy_factories(),
    )
    .expect("Failed to reload the workspace");
    // We have to query the Commit again. The Workspace is backed by a different
    // Store from the original Commit.
    let modified_commit = test_workspace
        .workspace
        .repo_loader()
        .store()
        .get_commit(modified_commit.id())
        .expect("Failed to find the commit with the test file");
    test_workspace
        .workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &modified_commit,
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();

    fn change_contents_appended_after_conflicts_to(
        path: &Path,
        contents_to_append: &(impl AsRef<[u8]> + ?Sized),
    ) {
        let original_contents = std::fs::read(path).unwrap();
        let mut new_contents: Vec<u8> = vec![];
        for line in original_contents.lines_with_terminator() {
            new_contents.extend(line);
            if line.starts_with_str(CONFLICT_END_MARKERS) {
                break;
            }
        }
        new_contents.extend(contents_to_append.as_ref());
        std::fs::write(path, new_contents).unwrap();
    }

    // Modify the contents and snapshot for the first time.
    change_contents_appended_after_conflicts_to(&file_disk_path, first_snapshot_appended_contents);
    test_workspace.snapshot().unwrap();

    // Modify the contents and snapshot for the second time.
    change_contents_appended_after_conflicts_to(&file_disk_path, second_snapshot_appended_contents);
    let tree = test_workspace.snapshot().unwrap();

    // Obtain the file contents in store.
    let tree_value = tree.path_value(file_repo_path).unwrap();
    assert!(!tree_value.is_resolved());
    let materialize_tree_value = jj_lib::conflicts::materialize_tree_value(
        test_workspace.workspace.repo_loader().store(),
        file_repo_path,
        tree_value,
    )
    .await
    .unwrap();
    let MaterializedTreeValue::FileConflict(materialize_file_value) = materialize_tree_value else {
        panic!("The tree entry should be a file.");
    };
    let MergeResult::Conflict(mut hunks) =
        jj_lib::files::merge_hunks(&materialize_file_value.contents)
    else {
        panic!("There should be conflicts.");
    };
    let appended_contents = hunks.remove(1).into_resolved().unwrap();
    assert!(appended_contents.starts_with_str(CONFLICT_END_MARKERS));
    let appended_contents = &appended_contents[CONFLICT_END_MARKERS.len()..];
    String::from_utf8(appended_contents.to_vec()).unwrap()
}

struct ResetAndSnapshotTestConfig {
    /// The original contents of the test file in Store.
    original_contents: &'static str,
    /// The contents of the test file on the disk before snapshot.
    new_contents_to_snapshot: &'static str,
}

#[tokio::main(flavor = "current_thread")]
#[test_case(ResetAndSnapshotTestConfig {
    original_contents: "a\r\n",
    new_contents_to_snapshot: "a\nb\r\n",
} => "a\nb\r\n"; "old contents have CRLF")]
#[test_case(ResetAndSnapshotTestConfig {
    original_contents: "a\n",
    new_contents_to_snapshot: "a\nb\r\n",
} => "a\nb\n"; "old contents don't have CRLF")]
async fn test_eol_reset_and_snapshot(
    ResetAndSnapshotTestConfig {
        original_contents,
        new_contents_to_snapshot,
    }: ResetAndSnapshotTestConfig,
) -> String {
    // In this test, we create a commit with a file in the Store. Then we reset the
    // working copy to the root commit and reset back to the commit with a file. To
    // verify whether the CRLF file state is correctly set, we modify the file,
    // snapshot, and check if the file contents in the Store is as expected.

    let user_settings =
        base_user_settings_with_extra_configs("working-copy.eol-conversion=\"input-output\"\n");
    let mut test_workspace =
        TestWorkspace::init_with_backend_and_settings(TestRepoBackend::Git, &user_settings);
    let file_repo_path = repo_path("test-eol-file");
    let file_disk_path = file_repo_path
        .to_fs_path(test_workspace.workspace.workspace_root())
        .unwrap();

    let root_commit = test_workspace.repo.store().root_commit();
    let mut tx = test_workspace.repo.start_transaction();
    let tree = testutils::create_tree(&test_workspace.repo, &[(file_repo_path, original_contents)]);
    let commit = tx
        .repo_mut()
        .new_commit(vec![root_commit.id().clone()], tree.id())
        .write()
        .unwrap();
    tx.commit("commit the initial commit").unwrap();
    test_workspace
        .workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &commit,
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();

    let mut working_copy = test_workspace
        .workspace
        .working_copy()
        .start_mutation()
        .unwrap();
    // Reset the working copy to the root commit.
    working_copy.recover(&root_commit).unwrap();
    working_copy
        .finish(test_workspace.repo.op_id().clone())
        .unwrap();
    let mut working_copy = test_workspace
        .workspace
        .working_copy()
        .start_mutation()
        .unwrap();
    // Reset the working copy to the original commit.
    working_copy.recover(&commit).unwrap();
    working_copy
        .finish(test_workspace.repo.op_id().clone())
        .unwrap();

    // Modify the test file and snapshot to test if the current CRLF file state is
    // correct.
    std::fs::write(&file_disk_path, new_contents_to_snapshot).unwrap();
    let tree = test_workspace.snapshot().unwrap();

    // Read the contents directly from the Store.
    let file_id = tree
        .path_value_async(file_repo_path)
        .await
        .unwrap()
        .to_file_merge()
        .unwrap()
        .into_resolved()
        .unwrap()
        .unwrap();
    let mut contents = String::new();
    test_workspace
        .repo
        .store()
        .read_file(file_repo_path, &file_id)
        .await
        .unwrap()
        .read_to_string(&mut contents)
        .await
        .unwrap();
    contents
}

struct ResetAndSnapshotConflictTestConfig {
    /// The original contents of the 2 sides of the conflicts of the test file
    /// in Store.
    original_contents: (&'static str, &'static str),
    /// The contents of the test file on the disk before snapshot.
    new_contents_to_snapshot: &'static str,
}

#[tokio::main(flavor = "current_thread")]
#[test_case(ResetAndSnapshotConflictTestConfig {
    original_contents: ("a\r\n", "b\n"),
    new_contents_to_snapshot: "aa\r\n",
} => "aa\r\n"; "old contents have CRLF #1")]
#[test_case(ResetAndSnapshotConflictTestConfig {
    original_contents: ("a\n", "b\r\n"),
    new_contents_to_snapshot: "aa\r\n",
} => "aa\r\n"; "old contents have CRLF #2")]
#[test_case(ResetAndSnapshotConflictTestConfig {
    original_contents: ("a\n", "b\n"),
    new_contents_to_snapshot: "aa\r\n",
} => "aa\n"; "old contents don't have CRLF")]
async fn test_eol_reset_and_snapshot_conflict(
    ResetAndSnapshotConflictTestConfig {
        original_contents: (original_side1_contents, original_side2_contents),
        new_contents_to_snapshot,
    }: ResetAndSnapshotConflictTestConfig,
) -> String {
    // Similar to test_eol_reset_and_snapshot, but the test file contains conflicts.

    let user_settings =
        base_user_settings_with_extra_configs("working-copy.eol-conversion=\"input-output\"\n");
    let mut test_workspace =
        TestWorkspace::init_with_backend_and_settings(TestRepoBackend::Git, &user_settings);
    let file_repo_path = repo_path("test-eol-file");
    let file_disk_path = file_repo_path
        .to_fs_path(test_workspace.workspace.workspace_root())
        .unwrap();

    // The commit graph:
    // E (modified conflict)
    // |
    // D (conflict)
    // |\
    // B C
    // |/
    // A

    let root_commit = test_workspace.repo.store().root_commit();
    let mut tx = test_workspace.repo.start_transaction();
    // We must prepare a non-empty base commit, so that when we update the conflict files, it won't be considered to edit the empty placeholder for an absent side and resolve the conflict on snapshot. See https://github.com/jj-vcs/jj/issues/7156 for details.
    let tree = testutils::create_tree(&test_workspace.repo, &[(file_repo_path, "\n")]);
    let base_commit = tx
        .repo_mut()
        .new_commit(vec![root_commit.id().clone()], tree.id())
        .write()
        .unwrap();
    let tree = testutils::create_tree(
        &test_workspace.repo,
        &[(file_repo_path, original_side1_contents)],
    );
    let parent1_commit = tx
        .repo_mut()
        .new_commit(vec![base_commit.id().clone()], tree.id())
        .write()
        .unwrap();
    let tree = testutils::create_tree(
        &test_workspace.repo,
        &[(file_repo_path, original_side2_contents)],
    );
    let parent2_commit = tx
        .repo_mut()
        .new_commit(vec![base_commit.id().clone()], tree.id())
        .write()
        .unwrap();
    tx.commit("commit parents").unwrap();

    // Reload the repo to pick up the new commits.
    test_workspace.repo = test_workspace.repo.reload_at_head().unwrap();
    // Create the merge commit.
    let tree = merge_commit_trees(&*test_workspace.repo, &[parent1_commit, parent2_commit])
        .await
        .unwrap();
    let merge_commit = commit_with_tree(test_workspace.repo.store(), tree.id());
    test_workspace
        .workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &merge_commit,
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();

    let mut working_copy = test_workspace
        .workspace
        .working_copy()
        .start_mutation()
        .unwrap();
    // Reset the working copy to the root commit.
    working_copy.recover(&root_commit).unwrap();
    working_copy
        .finish(test_workspace.repo.op_id().clone())
        .unwrap();
    let mut working_copy = test_workspace
        .workspace
        .working_copy()
        .start_mutation()
        .unwrap();
    // Reset the working copy to the original commit.
    working_copy.recover(&merge_commit).unwrap();
    working_copy
        .finish(test_workspace.repo.op_id().clone())
        .unwrap();

    // Modify the test file and snapshot to test if the current CRLF file state is
    // correct.
    std::fs::write(&file_disk_path, new_contents_to_snapshot).unwrap();
    let tree = test_workspace.snapshot().unwrap();

    // Read the contents directly from the Store.
    let file_id = tree
        .path_value_async(file_repo_path)
        .await
        .unwrap()
        .to_file_merge()
        .unwrap()
        .into_resolved()
        .unwrap()
        .unwrap();
    let mut contents = String::new();
    test_workspace
        .repo
        .store()
        .read_file(file_repo_path, &file_id)
        .await
        .unwrap()
        .read_to_string(&mut contents)
        .await
        .unwrap();
    contents
}
