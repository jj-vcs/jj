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

use std::path::Path;
use std::path::PathBuf;

use assert_matches::assert_matches;
use jj_lib::config::StackedConfig;
use jj_lib::git_backend::GitBackend;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::repo::Repo as _;
use jj_lib::settings::UserSettings;
use jj_lib::workspace::Workspace;
use test_case::test_case;
use testutils::TestRepoBackend;
use testutils::TestWorkspace;
use testutils::assert_tree_eq;
use testutils::git;
use testutils::write_random_commit;

fn canonicalize(input: &Path) -> (PathBuf, PathBuf) {
    let uncanonical = input.join("..").join(input.file_name().unwrap());
    let canonical = dunce::canonicalize(&uncanonical).unwrap();
    (canonical, uncanonical)
}

#[test]
fn test_init_local() {
    let settings = testutils::user_settings();
    let temp_dir = testutils::new_temp_dir();
    let (canonical, uncanonical) = canonicalize(temp_dir.path());
    let (workspace, repo) = Workspace::init_simple(&settings, &uncanonical).unwrap();
    assert!(repo.store().backend_impl::<GitBackend>().is_none());
    assert_eq!(workspace.workspace_root(), &canonical);

    // Just test that we can write a commit to the store
    let mut tx = repo.start_transaction();
    write_random_commit(tx.repo_mut());
}

#[test]
fn test_init_internal_git() {
    let settings = testutils::user_settings();
    let temp_dir = testutils::new_temp_dir();
    let (canonical, uncanonical) = canonicalize(temp_dir.path());
    let (workspace, repo) = Workspace::init_internal_git(&settings, &uncanonical).unwrap();
    let git_backend: &GitBackend = repo.store().backend_impl().unwrap();
    let repo_path = canonical.join(".jj").join("repo");
    assert_eq!(workspace.workspace_root(), &canonical);
    assert_eq!(
        git_backend.git_repo_path(),
        canonical.join(PathBuf::from_iter([".jj", "repo", "store", "git"])),
    );
    assert!(git_backend.git_workdir().is_none());
    assert_eq!(
        std::fs::read_to_string(repo_path.join("store").join("git_target")).unwrap(),
        "git"
    );

    // Just test that we can write a commit to the store
    let mut tx = repo.start_transaction();
    write_random_commit(tx.repo_mut());
}

#[test]
fn test_init_colocated_git() {
    let settings = testutils::user_settings();
    let temp_dir = testutils::new_temp_dir();
    let (canonical, uncanonical) = canonicalize(temp_dir.path());
    let (workspace, repo) = Workspace::init_colocated_git(&settings, &uncanonical).unwrap();
    let git_backend: &GitBackend = repo.store().backend_impl().unwrap();
    let repo_path = canonical.join(".jj").join("repo");
    assert_eq!(workspace.workspace_root(), &canonical);
    assert_eq!(git_backend.git_repo_path(), canonical.join(".git"));
    assert_eq!(git_backend.git_workdir(), Some(canonical.as_ref()));
    assert_eq!(
        std::fs::read_to_string(repo_path.join("store").join("git_target")).unwrap(),
        "../../../.git"
    );

    // Just test that we can write a commit to the store
    let mut tx = repo.start_transaction();
    write_random_commit(tx.repo_mut());
}

#[test]
fn test_init_external_git() {
    let settings = testutils::user_settings();
    let temp_dir = testutils::new_temp_dir();
    let (canonical, uncanonical) = canonicalize(temp_dir.path());
    let git_repo_path = uncanonical.join("git");
    git::init(&git_repo_path);
    std::fs::create_dir(uncanonical.join("jj")).unwrap();
    let (workspace, repo) = Workspace::init_external_git(
        &settings,
        &uncanonical.join("jj"),
        &git_repo_path.join(".git"),
    )
    .unwrap();
    let git_backend: &GitBackend = repo.store().backend_impl().unwrap();
    assert_eq!(workspace.workspace_root(), &canonical.join("jj"));
    assert_eq!(
        git_backend.git_repo_path(),
        canonical.join("git").join(".git")
    );
    assert_eq!(
        git_backend.git_workdir(),
        Some(canonical.join("git").as_ref())
    );

    // Just test that we can write a commit to the store
    let mut tx = repo.start_transaction();
    write_random_commit(tx.repo_mut());
}

#[test_case(TestRepoBackend::Simple ; "simple backend")]
#[test_case(TestRepoBackend::Git ; "git backend")]
fn test_init_with_default_config(backend: TestRepoBackend) {
    // Test that we can create a repo without setting any non-default config
    let settings = UserSettings::from_config(StackedConfig::with_defaults()).unwrap();
    let test_workspace = TestWorkspace::init_with_backend_and_settings(backend, &settings);
    let repo = &test_workspace.repo;
    let wc_commit_id = repo
        .view()
        .get_wc_commit_id(WorkspaceName::DEFAULT)
        .unwrap();
    let wc_commit = repo.store().get_commit(wc_commit_id).unwrap();
    assert_eq!(wc_commit.author().name, "".to_string());
    assert_eq!(wc_commit.author().email, "".to_string());
    assert_eq!(wc_commit.committer().name, "".to_string());
    assert_eq!(wc_commit.committer().email, "".to_string());
}

#[test_case(TestRepoBackend::Simple ; "simple backend")]
#[test_case(TestRepoBackend::Git ; "git backend")]
fn test_init_checkout(backend: TestRepoBackend) {
    // Test the contents of the working-copy commit after init
    let settings = testutils::user_settings();
    let test_workspace = TestWorkspace::init_with_backend_and_settings(backend, &settings);
    let repo = &test_workspace.repo;
    let wc_commit_id = repo
        .view()
        .get_wc_commit_id(WorkspaceName::DEFAULT)
        .unwrap();
    let wc_commit = repo.store().get_commit(wc_commit_id).unwrap();
    assert_tree_eq!(wc_commit.tree(), repo.store().empty_merged_tree());
    assert_eq!(
        wc_commit.store_commit().parents,
        vec![repo.store().root_commit_id().clone()]
    );
    assert!(wc_commit.store_commit().predecessors.is_empty());
    assert_eq!(wc_commit.description(), "");
    assert_eq!(wc_commit.author().name, settings.user_name());
    assert_eq!(wc_commit.author().email, settings.user_email());
    assert_eq!(wc_commit.committer().name, settings.user_name());
    assert_eq!(wc_commit.committer().email, settings.user_email());
    assert_matches!(
        repo.operation().predecessors_for_commit(wc_commit.id()),
        Some([])
    );
}

#[cfg(unix)]
#[cfg_attr(target_os = "macos", ignore = "APFS/HFS+ don't like non-UTF-8 paths")]
#[test]
fn test_init_load_non_utf8_path() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    use jj_lib::workspace::default_working_copy_factories;
    use testutils::TestEnvironment;

    let settings = testutils::user_settings();
    let test_env = TestEnvironment::init();

    let git_repo_path = test_env.root().join(OsStr::from_bytes(b"git\xe0"));
    assert!(git_repo_path.to_str().is_none());
    git::init(&git_repo_path);

    // Workspace can be created
    let workspace_root = test_env.root().join(OsStr::from_bytes(b"jj\xe0"));
    std::fs::create_dir(&workspace_root).unwrap();
    Workspace::init_external_git(&settings, &workspace_root, &git_repo_path.join(".git")).unwrap();

    // Workspace can be loaded
    let workspace = Workspace::load(
        &settings,
        &workspace_root,
        &test_env.default_store_factories(),
        &default_working_copy_factories(),
    )
    .unwrap();

    // Just test that we can write a commit to the store
    let repo = workspace.repo_loader().load_at_head().unwrap();
    let mut tx = repo.start_transaction();
    write_random_commit(tx.repo_mut());
}
