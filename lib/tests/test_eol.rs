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

use bstr::ByteSlice as _;
use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigSource;
use jj_lib::repo::Repo as _;
use jj_lib::repo::StoreFactories;
use jj_lib::rewrite::merge_commit_trees;
use jj_lib::settings::UserSettings;
use jj_lib::workspace::Workspace;
use jj_lib::workspace::default_working_copy_factories;
use pollster::FutureExt as _;
use test_case::test_case;
use testutils::TestRepoBackend;
use testutils::TestWorkspace;
use testutils::assert_tree_eq;
use testutils::base_user_config;
use testutils::commit_with_tree;
use testutils::repo_path;

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
    assert_tree_eq!(new_tree, tree, "The working copy should be clean.");
    let file_added_commit = commit_with_tree(test_workspace.repo.store(), tree);

    // Create a commit with the file removed, so that later when we checkout the
    // file_added_commit, the test file is recreated.
    std::fs::remove_file(&file_disk_path).unwrap();
    let tree = test_workspace.snapshot().unwrap();
    let file_removed_commit = commit_with_tree(test_workspace.repo.store(), tree);
    let workspace = &mut test_workspace.workspace;
    workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &file_removed_commit,
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
        )
        .unwrap();
    assert!(file_disk_path.exists());
    let new_tree = test_workspace.snapshot().unwrap();
    assert_tree_eq!(
        new_tree,
        file_added_commit.tree(),
        "The working copy should be clean."
    );

    std::fs::read(&file_disk_path).expect("Failed to read the checked out test file")
}

// Create a conflict commit in a CRLF EOL file, and append another line with the
// CRLF EOL to the file, create a snapshot on the modified merge conflict,
// checkout the snapshot with the given setting, and return the content of the
// file.
fn create_conflict_snapshot_and_read(extra_setting: &str) -> Vec<u8> {
    // Use the working-copy.eol-conversion = "none" setting to write files to the
    // store as is.
    let no_eol_conversion_settings =
        base_user_settings_with_extra_configs("working-copy.eol-conversion = \"none\"\n");
    let mut test_workspace = TestWorkspace::init_with_backend_and_settings(
        TestRepoBackend::Git,
        &no_eol_conversion_settings,
    );
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
    testutils::write_working_copy_file(
        test_workspace.workspace.workspace_root(),
        file_repo_path,
        "a\r\n",
    );
    let tree = test_workspace.snapshot().unwrap();
    let mut tx = test_workspace.repo.start_transaction();
    let parent1_commit = tx
        .repo_mut()
        .new_commit(vec![root_commit.id().clone()], tree)
        .write()
        .unwrap();
    tx.commit("commit parent1").unwrap();

    test_workspace
        .workspace
        .check_out(test_workspace.repo.op_id().clone(), None, &root_commit)
        .unwrap();
    testutils::write_working_copy_file(
        test_workspace.workspace.workspace_root(),
        file_repo_path,
        "b\r\n",
    );
    let tree = test_workspace.snapshot().unwrap();
    let mut tx = test_workspace.repo.start_transaction();
    let parent2_commit = tx
        .repo_mut()
        .new_commit(vec![root_commit.id().clone()], tree)
        .write()
        .unwrap();
    tx.commit("commit parent2").unwrap();

    // Reload the repo to pick up the new commits.
    test_workspace.repo = test_workspace.repo.reload_at_head().unwrap();
    // Create the merge commit.
    let tree = merge_commit_trees(&*test_workspace.repo, &[parent1_commit, parent2_commit])
        .block_on()
        .unwrap();
    let merge_commit = commit_with_tree(test_workspace.repo.store(), tree);
    // Append new texts to the file with conflicts to make sure the last line is not
    // conflict markers.
    test_workspace
        .workspace
        .check_out(test_workspace.repo.op_id().clone(), None, &merge_commit)
        .unwrap();
    let mut file = File::options().append(true).open(&file_disk_path).unwrap();
    file.write_all(b"c\r\n").unwrap();
    drop(file);

    let extra_setting = format!("{extra_setting}\n");
    let user_settings = base_user_settings_with_extra_configs(&extra_setting);
    // Reload the Workspace to apply the settings under testing.
    test_workspace.workspace = Workspace::load(
        &user_settings,
        test_workspace.workspace.workspace_root(),
        &StoreFactories::default(),
        &default_working_copy_factories(),
    )
    .expect("Failed to reload the workspace");
    let tree = test_workspace.snapshot().unwrap();
    let new_tree = test_workspace.snapshot().unwrap();
    assert_tree_eq!(new_tree, tree, "The working copy should be clean.");
    // Create the new merge commit with the conflict file appended.
    let merge_commit = commit_with_tree(test_workspace.repo.store(), tree);

    // Reload the Workspace with the working-copy.eol-conversion = "none" setting to
    // check the EOL of the file written to the store previously.
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
        .check_out(test_workspace.repo.op_id().clone(), None, &merge_commit)
        .unwrap();

    assert!(std::fs::exists(&file_disk_path).unwrap());
    std::fs::read(&file_disk_path).unwrap()
}

#[test]
fn test_eol_conversion_input_output_snapshot_conflicts() {
    let contents =
        create_conflict_snapshot_and_read(r#"working-copy.eol-conversion = "input-output""#);
    for line in contents.lines_with_terminator() {
        assert!(
            !line.ends_with(b"\r\n"),
            "{:?} should not end with CRLF",
            line.to_str_lossy().as_ref()
        );
    }
}

#[test]
fn test_eol_conversion_input_snapshot_conflicts() {
    let contents = create_conflict_snapshot_and_read(r#"working-copy.eol-conversion = "input""#);
    for line in contents.lines_with_terminator() {
        assert!(
            !line.ends_with(b"\r\n"),
            "{:?} should not end with CRLF",
            line.to_str_lossy().as_ref()
        );
    }
}

#[test]
fn test_eol_conversion_none_snapshot_conflicts() {
    let contents = create_conflict_snapshot_and_read(r#"working-copy.eol-conversion = "none""#);
    // We only check the last line, because it is only guaranteed that the last line
    // is not the conflict markers. The conflict markers in the store are supposed
    // to use the LF EOL.
    let line = contents.lines_with_terminator().next_back().unwrap();
    assert!(
        line.ends_with(b"\r\n"),
        "{:?} should end with CRLF",
        line.to_str_lossy().as_ref()
    );
}

struct UpdateConflictsTestConfig {
    parent1_contents: &'static str,
    parent2_contents: &'static str,
    extra_setting: &'static str,

    expected_eol: &'static str,
    expected_conflict_side1: &'static str,
    expected_conflict_side2: &'static str,
}

#[test_case(UpdateConflictsTestConfig {
    parent1_contents: "a\n",
    parent2_contents: "b\n",
    extra_setting: r#"working-copy.eol-conversion = "none""#,
    expected_eol: "\n",
    expected_conflict_side1: "a\n",
    expected_conflict_side2: "b\n",
}; "LF parents with none settings")]
#[test_case(UpdateConflictsTestConfig {
    parent1_contents: "a\n",
    parent2_contents: "b\n",
    extra_setting: r#"working-copy.eol-conversion = "input-output""#,
    expected_eol: "\r\n",
    expected_conflict_side1: "a\r\n",
    expected_conflict_side2: "b\r\n",
}; "LF parents with input-output settings")]
#[test_case(UpdateConflictsTestConfig {
    parent1_contents: "a\r\n",
    parent2_contents: "b\r\n",
    extra_setting: r#"working-copy.eol-conversion = "input-output""#,
    expected_eol: "\r\n",
    expected_conflict_side1: "a\r\n",
    expected_conflict_side2: "b\r\n",
}; "CRLF parents with input-output settings")]
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
        .new_commit(vec![root_commit.id().clone()], tree)
        .write()
        .unwrap();
    let tree = testutils::create_tree(&test_workspace.repo, &[(file_repo_path, parent2_contents)]);
    let parent2_commit = tx
        .repo_mut()
        .new_commit(vec![root_commit.id().clone()], tree)
        .write()
        .unwrap();
    tx.commit("commit parent 2").unwrap();

    // Reload the repo to pick up the new commits.
    test_workspace.repo = test_workspace.repo.reload_at_head().unwrap();
    // Create the merge commit.
    let tree = merge_commit_trees(&*test_workspace.repo, &[parent1_commit, parent2_commit])
        .block_on()
        .unwrap();
    let merge_commit = commit_with_tree(test_workspace.repo.store(), tree);

    // Checkout the merge commit.
    test_workspace
        .workspace
        .check_out(test_workspace.repo.op_id().clone(), None, &merge_commit)
        .unwrap();
    let contents = std::fs::read(&file_disk_path).unwrap();
    for line in contents.lines_with_terminator() {
        assert!(
            line.ends_with_str(expected_eol),
            "{:?} should end with {:?}",
            &*line.to_str_lossy(),
            expected_eol
        );
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
} => CRLF_FILE_CONTENT; "eol-conversion input-output mixed EOL file")]
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
    let commit = commit_with_tree(test_workspace.repo.store(), tree);

    // Checkout the empty commit to clear the directory, so that later when we
    // checkout, files are recreated.
    test_workspace
        .workspace
        .check_out(
            test_workspace.repo.op_id().clone(),
            None,
            &test_workspace.workspace.repo_loader().store().root_commit(),
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
        .check_out(test_workspace.repo.op_id().clone(), None, &commit)
        .unwrap();

    // When we take a snapshot now, the tree may not be clean, because the EOL our
    // snapshot creates may not align with what is currently used in store. e.g.
    // with working-copy.eol-conversion = "input-output", the test-eol-file may have
    // CRLF line endings in the store, but the snapshot will change the EOL to LF,
    // hence the diff.

    assert!(std::fs::exists(&file_disk_path).unwrap());
    std::fs::read(&file_disk_path).unwrap()
}
