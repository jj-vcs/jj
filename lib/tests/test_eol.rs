use std::sync::Arc;

use bstr::ByteSlice as _;
use jj_lib::git_backend::GitBackend;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::working_copy::CheckoutOptions;
use test_case::test_case;
use testutils::commit_with_tree;
use testutils::create_tree;
use testutils::repo_path;
use testutils::TestRepoBackend;
use testutils::TestTreeBuilder;
use testutils::TestWorkspace;

fn get_git_backend(repo: &Arc<ReadonlyRepo>) -> &GitBackend {
    repo.store()
        .backend_impl()
        .downcast_ref::<GitBackend>()
        .unwrap()
}

fn set_git_config_value(backend: &GitBackend, key: &'static &str, new_value: &str) {
    // We have to mutate the config associated with the original gix::Repository, so
    // that the the config change is visible to all the clients of the Store.
    backend
        .git_config_set_raw_value(key, new_value)
        .expect("failed to set the git config value");
    let config = backend.git_config();

    // Write back in case we reload the git config from the file system.
    let mut file = std::fs::File::create(config.meta().path.as_ref().unwrap()).unwrap();
    config
        .write_to_filter(&mut file, |section| section.meta() == config.meta())
        .unwrap();
}

static LF_FILE_CONTENT: &[u8] = b"aaa\nbbbb\nccccc\n";
static CRLF_FILE_CONTENT: &[u8] = b"aaa\r\nbbbb\r\nccccc\r\n";
static MIXED_EOL_FILE_CONTENT: &[u8] = b"aaa\nbbbb\r\nccccc\n";
static BINARY_FILE_CONTENT: &[u8] = include_bytes!("data/binary_file.png");

// Tests on binary files don't make sense if it doesn't include CRLF or LF.
#[test]
fn test_binary_file_should_contain_crlf_and_lf() {
    assert!(
        BINARY_FILE_CONTENT.find_byteset(b"\r\n").is_some(),
        "The binary file doesn't include CRLF."
    );
    let mut rest = BINARY_FILE_CONTENT;
    while let Some(pos) = rest.find_byte(b'\n') {
        if pos == 0 {
            return;
        }
        if rest[pos - 1] != b'\r' {
            return;
        }
        rest = &rest[(pos + 1)..];
    }
    panic!("Failed to find LF that doesn't follow CR in the binary file.");
}

struct Config {
    autocrlf: &'static str,
    file_content: &'static [u8],
}

#[test_case(Config {
    autocrlf: "false",
    file_content: LF_FILE_CONTENT,
 } => LF_FILE_CONTENT; "autocrlf false LF only file")]
#[test_case(Config {
    autocrlf: "false",
    file_content: CRLF_FILE_CONTENT,
} => CRLF_FILE_CONTENT; "autocrlf false CRLF only file")]
#[test_case(Config {
    autocrlf: "false",
    file_content: MIXED_EOL_FILE_CONTENT,
}  => MIXED_EOL_FILE_CONTENT; "autocrlf false Mixed EOL file")]
#[test_case(Config {
    autocrlf: "false",
    file_content: BINARY_FILE_CONTENT,
} => BINARY_FILE_CONTENT; "autocrlf false binary file")]
#[test_case(Config {
    autocrlf: "true",
    file_content: LF_FILE_CONTENT,
} => CRLF_FILE_CONTENT; "autocrlf true LF only file")]
#[test_case(Config {
    autocrlf: "true",
    file_content: CRLF_FILE_CONTENT,
} => CRLF_FILE_CONTENT; "autocrlf true CRLF only file")]
#[test_case(Config {
    autocrlf: "true",
    file_content: MIXED_EOL_FILE_CONTENT,
} => CRLF_FILE_CONTENT; "autocrlf true Mixed EOL file")]
#[test_case(Config {
    autocrlf: "true",
    file_content: BINARY_FILE_CONTENT,
} => BINARY_FILE_CONTENT; "autocrlf true binary file")]
#[test_case(Config {
    autocrlf: "input",
    file_content: LF_FILE_CONTENT,
} => LF_FILE_CONTENT; "autocrlf input LF only file")]
#[test_case(Config {
    autocrlf: "input",
    file_content: CRLF_FILE_CONTENT,
} => CRLF_FILE_CONTENT; "autocrlf input CRLF only file")]
#[test_case(Config {
    autocrlf: "input",
    file_content: MIXED_EOL_FILE_CONTENT,
} => MIXED_EOL_FILE_CONTENT; "autocrlf input Mixed EOL file")]
#[test_case(Config {
    autocrlf: "input",
    file_content: BINARY_FILE_CONTENT,
} => BINARY_FILE_CONTENT; "autocrlf input binary file")]
fn test_autocrlf_update_conversion(
    Config {
        autocrlf,
        file_content,
    }: Config,
) -> Vec<u8> {
    // This test checks in files with autocrlf=false, so that the store stores files
    // as is. Then we use jj to check out those files with different EOL
    // configurations to verify if the EOLs are converted as expected.

    let mut test_workspace = TestWorkspace::init_with_backend(TestRepoBackend::Git);
    let repo = &test_workspace.repo;
    let repo_path = repo_path("test-eol-file");
    let disk_path = repo_path
        .to_fs_path(test_workspace.workspace.workspace_root())
        .unwrap();

    // Set core.autocrlf to false, so that the input files are stored as is.
    let git_backend = get_git_backend(repo);
    set_git_config_value(git_backend, &"core.autocrlf", "false");

    // Create 2 commits. One with the test file, one without.
    let tree = {
        let mut builder = TestTreeBuilder::new(repo.store().clone());
        builder.file(repo_path, file_content);
        builder.write_merged_tree()
    };
    let file_added_commit = commit_with_tree(repo.store(), tree.id());
    let tree = create_tree(repo, &[]);
    let file_removed_commit = commit_with_tree(repo.store(), tree.id());

    // Check out the commit without the test file to clear the directory, so that
    // when we check out files later, those files are recreated.
    let workspace = &mut test_workspace.workspace;
    workspace
        .check_out(
            repo.op_id().clone(),
            None,
            &file_removed_commit,
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();
    assert!(!disk_path.exists());

    // Change the autocrlf config to the configuration under testing.
    set_git_config_value(git_backend, &"core.autocrlf", autocrlf);

    // Check out the commit with the test file. TreeState::update should update the
    // EOL accordingly.
    workspace
        .check_out(
            repo.op_id().clone(),
            None,
            &file_added_commit,
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();
    assert!(disk_path.exists());

    // When we take a snapshot now, the tree may not be clean, because the EOL our
    // snapshot creates may not align to what is currently used in store. e.g. with
    // core.autocrlf = true, the test-eol-file may have CRLF line endings, but the
    // snapshot will change the EOL to LF, hence the diff.

    // The checked out test file should have EOL converted correctly.
    std::fs::read(&disk_path).unwrap()
}

#[test_case(Config {
    autocrlf: "false",
    file_content: LF_FILE_CONTENT,
} => LF_FILE_CONTENT; "autocrlf false LF only file")]
#[test_case(Config {
    autocrlf: "false",
    file_content: CRLF_FILE_CONTENT,
} => CRLF_FILE_CONTENT; "autocrlf false CRLF only file")]
#[test_case(Config {
    autocrlf: "false",
    file_content: MIXED_EOL_FILE_CONTENT,
} => MIXED_EOL_FILE_CONTENT; "autocrlf false Mixed EOL file")]
#[test_case(Config {
    autocrlf: "false",
    file_content: BINARY_FILE_CONTENT,
} => BINARY_FILE_CONTENT; "autocrlf false binary file")]
#[test_case(Config {
    autocrlf: "true",
    file_content: LF_FILE_CONTENT,
} => LF_FILE_CONTENT; "autocrlf true LF only file")]
#[test_case(Config {
    autocrlf: "true",
    file_content: CRLF_FILE_CONTENT,
} => LF_FILE_CONTENT; "autocrlf true CRLF only file")]
#[test_case(Config {
    autocrlf: "true",
    file_content: MIXED_EOL_FILE_CONTENT,
} => LF_FILE_CONTENT; "autocrlf true Mixed EOL file")]
#[test_case(Config {
    autocrlf: "true",
    file_content: BINARY_FILE_CONTENT,
} => BINARY_FILE_CONTENT; "autocrlf true binary file")]
#[test_case(Config {
    autocrlf: "input",
    file_content: LF_FILE_CONTENT,
} => LF_FILE_CONTENT; "autocrlf input LF only file")]
#[test_case(Config {
    autocrlf: "input",
    file_content: CRLF_FILE_CONTENT,
} => LF_FILE_CONTENT; "autocrlf input CRLF only file")]
#[test_case(Config {
    autocrlf: "input",
    file_content: MIXED_EOL_FILE_CONTENT,
} => LF_FILE_CONTENT; "autocrlf input Mixed EOL file")]
#[test_case(Config {
    autocrlf: "input",
    file_content: BINARY_FILE_CONTENT,
} => BINARY_FILE_CONTENT; "autocrlf input binary file")]
fn test_autocrlf_snapshot_conversion(
    Config {
        file_content,
        autocrlf,
    }: Config,
) -> Vec<u8> {
    // This test creates snapshots with different EOL configurations, where proper
    // EOL conversion should apply before writing files back to the store. Then
    // files are checked out with autocrlf=false, which won't touch the EOLs, so
    // that we can tell whether the exact EOLs written to the store are expected.

    let mut test_workspace = TestWorkspace::init_with_backend(TestRepoBackend::Git);
    let repo = Arc::clone(&test_workspace.repo);
    let repo_path = repo_path("test-eol-file");
    let disk_path = repo_path
        .to_fs_path(test_workspace.workspace.workspace_root())
        .unwrap();

    let git_backend = get_git_backend(&repo);
    set_git_config_value(git_backend, &"core.autocrlf", autocrlf);

    testutils::write_working_copy_file(
        test_workspace.workspace.workspace_root(),
        repo_path,
        file_content,
    );
    let tree = test_workspace.snapshot().unwrap();
    let new_tree = test_workspace.snapshot().unwrap();
    assert_eq!(
        new_tree.id(),
        tree.id(),
        "The working copy should be clean."
    );
    let file_added_commit = commit_with_tree(repo.store(), tree.id());

    std::fs::remove_file(&disk_path).unwrap();
    let tree = test_workspace.snapshot().unwrap();
    let file_removed_commit = commit_with_tree(repo.store(), tree.id());

    let workspace = &mut test_workspace.workspace;
    workspace
        .check_out(
            repo.op_id().clone(),
            None,
            &file_removed_commit,
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();
    assert!(!disk_path.exists());

    set_git_config_value(git_backend, &"core.autocrlf", "false");
    workspace
        .check_out(
            repo.op_id().clone(),
            None,
            &file_added_commit,
            &CheckoutOptions::empty_for_test(),
        )
        .unwrap();
    assert!(disk_path.exists());
    let new_tree = test_workspace.snapshot().unwrap();
    assert_eq!(
        new_tree.id(),
        *file_added_commit.tree_id(),
        "The working copy should be clean."
    );

    std::fs::read(&disk_path).unwrap()
}
