use std::fs;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use jj_lib::backend::MergedTreeId;
use jj_lib::backend::TreeId;
use jj_lib::backend::TreeValue;
use jj_lib::commit::Commit;
#[cfg(unix)]
use jj_lib::local_working_copy::ExecConfig;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathComponent;
use jj_lib::tree::Tree;
use jj_lib::working_copy::SnapshotOptions;
use jj_lib::working_copy::WorkingCopyOptions;
use testutils::TestWorkspace;

// All tests just use a single filename. Make it a global for convenience.
const NAME: &str = "dummy-file-name";

/// Flip the executable bit of a file on the filesystem.
fn flip_file_exec_bit(path: &PathBuf) {
    let file = fs::File::open(path).unwrap();
    #[cfg(windows)]
    let _ = file;
    #[cfg(unix)]
    {
        let new_mode = 0o100 ^ file.metadata().unwrap().mode();
        file.set_permissions(PermissionsExt::from_mode(new_mode))
            .unwrap();
        let mode = file.metadata().unwrap().mode();
        assert_eq!(mode, new_mode); // sanity check
    }
}

/// Returns a new tree id with the executable bit for this path flipped.
fn flip_tree_exec_bit(tree: &Tree, path: &str) -> TreeId {
    let repo_path_component = RepoPathComponent::new(path).unwrap();
    let store = tree.store();
    let mut builder = store.tree_builder(store.empty_tree_id().clone());
    let tree_val = tree.value(repo_path_component).unwrap().clone();
    let TreeValue::File { id, mut executable } = tree_val else {
        panic!()
    };
    executable = !executable;
    builder.set(
        RepoPath::root().join(repo_path_component),
        TreeValue::File { id, executable },
    );
    builder.write_tree().unwrap()
}

/// Assert a file on the filesystem has the expected executable bit (on Unix).
#[track_caller]
fn assert_file_exec_bit(path: &PathBuf, expected: bool) {
    let file = fs::File::open(path).unwrap();
    let perms = file.metadata().unwrap().permissions();
    #[cfg(unix)]
    let actual = (perms.mode() & 0o100) == 0o100;
    #[cfg(windows)]
    let (actual, _) = (expected, perms);
    assert_eq!(actual, expected);
}

/// Assert a stored tree value has the expected executable bit.
#[track_caller]
fn assert_tree_value_exec_bit(tree_val: &TreeValue, expected: bool) {
    match tree_val {
        TreeValue::File { id: _, executable } => {
            assert_eq!(*executable, expected);
        }
        _ => panic!(),
    }
}

/// Checkout a commit with the given working copy options.
fn checkout_with_opts(ws: &TestWorkspace, commit: &Commit, options: &WorkingCopyOptions) {
    let mut locked_wc = ws
        .workspace
        .working_copy()
        .start_mutation(options.clone())
        .unwrap();
    locked_wc.check_out(commit).unwrap();
    let op_id = ws.repo.op_id().clone();
    locked_wc.finish(op_id).unwrap();
}

/// Snapshot the tree with the given working copy options.
fn snapshot_with_opts(ws: &TestWorkspace, options: &WorkingCopyOptions) -> Tree {
    let mut locked_wc = ws
        .workspace
        .working_copy()
        .start_mutation(options.clone())
        .unwrap();
    let (tree_id, _stats) = locked_wc
        .snapshot(&SnapshotOptions::empty_for_test())
        .unwrap();
    let op_id = ws.repo.op_id().clone();
    locked_wc.finish(op_id).unwrap();
    let merged_tree = ws.repo.store().get_root_tree(&tree_id).unwrap();
    merged_tree.take().into_resolved().unwrap()
}

/// Test that snapshotting files stores the correct executable bit in the tree.
#[test]
fn test_exec_bit_snapshot() {
    let ws = TestWorkspace::init();
    let path = &ws.workspace.workspace_root().to_owned().join(NAME);
    let repo_path_component = RepoPathComponent::new(NAME).unwrap();
    let options = &mut WorkingCopyOptions::empty_for_test();

    // > Snapshot tree values when the file is/isn't executable.
    fs::File::create(path).unwrap();
    let tree = snapshot_with_opts(&ws, options);
    let val = tree.value(repo_path_component).unwrap();
    assert_tree_value_exec_bit(val, false);

    flip_file_exec_bit(path);
    let tree = snapshot_with_opts(&ws, options);
    let val = tree.value(repo_path_component).unwrap();
    assert_tree_value_exec_bit(val, cfg!(unix)); // Exec bit only stored on unix
}

/// Test that checking out a tree writes the correct executable bit to the
/// filesystem.
#[test]
fn test_exec_bit_checkout() {
    let ws = TestWorkspace::init();
    let path = &ws.workspace.workspace_root().to_owned().join(NAME);
    let repo_path = RepoPath::from_internal_string(NAME).unwrap();

    // > Build two commits that write the executable bit as true/false.
    let tree = testutils::create_single_tree(&ws.repo, &[(repo_path, "")]);
    let tree_no_exec = tree.id().clone();
    let tree_exec = flip_tree_exec_bit(&tree, NAME);
    assert_ne!(tree_no_exec, tree_exec);
    let commit_with_tree_id =
        |id: TreeId| testutils::commit_with_tree(ws.repo.store(), MergedTreeId::resolved(id));
    let commit_exec = &commit_with_tree_id(tree_exec);
    let commit_no_exec = &commit_with_tree_id(tree_no_exec);
    assert_ne!(commit_exec, commit_no_exec);

    // > Checkout commits and ensure the filesystem is updated correctly.
    assert!(!fs::exists(path).unwrap());
    let options = &WorkingCopyOptions::empty_for_test();
    checkout_with_opts(&ws, commit_exec, options);
    assert_file_exec_bit(path, true);

    checkout_with_opts(&ws, commit_no_exec, options);
    assert_file_exec_bit(path, false);

    checkout_with_opts(&ws, commit_exec, options);
    assert_file_exec_bit(path, true);
}

/// Test that changing the executable bit with differing configuration on unix.
#[test]
#[cfg(unix)]
fn test_exec_bit_config() {
    let ws = TestWorkspace::init();
    let path = &ws.workspace.workspace_root().join(NAME);
    let repo_path = RepoPath::from_internal_string(NAME).unwrap();
    let repo_path_component = RepoPathComponent::new(NAME).unwrap();

    // > Build two commits that write the executable bit as true/false.
    let tree = testutils::create_single_tree(&ws.repo, &[(repo_path, "")]);
    let tree_no_exec = tree.id().clone();
    let tree_exec = flip_tree_exec_bit(&tree, NAME);
    assert_ne!(tree_no_exec, tree_exec);
    let commit_with_tree_id =
        |id: TreeId| testutils::commit_with_tree(ws.repo.store(), MergedTreeId::resolved(id));
    let commit_exec = &commit_with_tree_id(tree_exec);
    let commit_no_exec = &commit_with_tree_id(tree_no_exec);
    assert_ne!(commit_exec, commit_no_exec);

    // > Checkout commits and ensure the filesystem is updated correctly.
    assert!(!fs::exists(path).unwrap());
    let options = &mut WorkingCopyOptions::empty_for_test();
    options.exec_config = Some(ExecConfig::Respect);
    checkout_with_opts(&ws, commit_no_exec, options);
    assert_file_exec_bit(path, false);

    checkout_with_opts(&ws, commit_exec, options);
    assert_file_exec_bit(path, true);

    options.exec_config = Some(ExecConfig::Ignore);
    checkout_with_opts(&ws, commit_no_exec, options);
    assert_file_exec_bit(path, true);

    // > Snapshot tree values when the file is/isn't executable.
    let tree = snapshot_with_opts(&ws, options);
    let val = tree.value(repo_path_component).unwrap();
    assert_tree_value_exec_bit(val, false);
    assert_file_exec_bit(path, true);
    // Tree value exec bit should not change on snapshot when ignored

    flip_file_exec_bit(path);
    let tree = snapshot_with_opts(&ws, options);
    let val = tree.value(repo_path_component).unwrap();
    assert_tree_value_exec_bit(val, false);
    assert_file_exec_bit(path, false);

    flip_file_exec_bit(path);
    options.exec_config = Some(ExecConfig::Respect);
    let tree = snapshot_with_opts(&ws, options);
    let val = tree.value(repo_path_component).unwrap();
    assert_tree_value_exec_bit(val, true);
    assert_file_exec_bit(path, true);
}
