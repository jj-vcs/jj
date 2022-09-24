// Copyright 2020 Google LLC
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

use std::path::{Path, PathBuf};

use jujutsu_lib::op_store::WorkspaceId;
use jujutsu_lib::settings::UserSettings;
use jujutsu_lib::testutils;
use jujutsu_lib::testutils::TestWorkspace;
use jujutsu_lib::workspace::Workspace;
use test_case::test_case;

fn canonicalize(input: &Path) -> (PathBuf, PathBuf) {
    let uncanonical = input.join("..").join(input.file_name().unwrap());
    let canonical = uncanonical.canonicalize().unwrap();
    (canonical, uncanonical)
}

#[test]
fn test_init_local() {
    let settings = testutils::user_settings();
    let temp_dir = testutils::new_temp_dir();
    let (canonical, uncanonical) = canonicalize(temp_dir.path());
    let (workspace, repo) = Workspace::init_local(&settings, &uncanonical).unwrap();
    assert!(repo.store().git_repo().is_none());
    assert_eq!(repo.repo_path(), &canonical.join(".jj").join("repo"));
    assert_eq!(workspace.workspace_root(), &canonical);

    // Just test that we can write a commit to the store
    let mut tx = repo.start_transaction("test");
    testutils::create_random_commit(&settings, &repo).write_to_repo(tx.mut_repo());
}

#[test]
fn test_init_internal_git() {
    let settings = testutils::user_settings();
    let temp_dir = testutils::new_temp_dir();
    let (canonical, uncanonical) = canonicalize(temp_dir.path());
    let (workspace, repo) = Workspace::init_internal_git(&settings, &uncanonical).unwrap();
    assert!(repo.store().git_repo().is_some());
    assert_eq!(repo.repo_path(), &canonical.join(".jj").join("repo"));
    assert_eq!(workspace.workspace_root(), &canonical);

    // Just test that we ca write a commit to the store
    let mut tx = repo.start_transaction("test");
    testutils::create_random_commit(&settings, &repo).write_to_repo(tx.mut_repo());
}

#[test]
fn test_init_external_git() {
    let settings = testutils::user_settings();
    let temp_dir = testutils::new_temp_dir();
    let (canonical, uncanonical) = canonicalize(temp_dir.path());
    let git_repo_path = uncanonical.join("git");
    git2::Repository::init(&git_repo_path).unwrap();
    std::fs::create_dir(&uncanonical.join("jj")).unwrap();
    let (workspace, repo) =
        Workspace::init_external_git(&settings, &uncanonical.join("jj"), &git_repo_path).unwrap();
    assert!(repo.store().git_repo().is_some());
    assert_eq!(
        repo.repo_path(),
        &canonical.join("jj").join(".jj").join("repo")
    );
    assert_eq!(workspace.workspace_root(), &canonical.join("jj"));

    // Just test that we can write a commit to the store
    let mut tx = repo.start_transaction("test");
    testutils::create_random_commit(&settings, &repo).write_to_repo(tx.mut_repo());
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_init_no_config_set(use_git: bool) {
    // Test that we can create a repo without setting any config
    let settings = UserSettings::from_config(config::Config::default());
    let test_workspace = TestWorkspace::init(&settings, use_git);
    let repo = &test_workspace.repo;
    let wc_commit_id = repo
        .view()
        .get_wc_commit_id(&WorkspaceId::default())
        .unwrap();
    let wc_commit = repo.store().get_commit(wc_commit_id).unwrap();
    assert_eq!(wc_commit.author().name, "(no name configured)".to_string());
    assert_eq!(
        wc_commit.author().email,
        "(no email configured)".to_string()
    );
    assert_eq!(
        wc_commit.committer().name,
        "(no name configured)".to_string()
    );
    assert_eq!(
        wc_commit.committer().email,
        "(no email configured)".to_string()
    );
}

#[test_case(false ; "local backend")]
#[test_case(true ; "git backend")]
fn test_init_checkout(use_git: bool) {
    // Test the contents of the checkout after init
    let settings = testutils::user_settings();
    let test_workspace = TestWorkspace::init(&settings, use_git);
    let repo = &test_workspace.repo;
    let wc_commit_id = repo
        .view()
        .get_wc_commit_id(&WorkspaceId::default())
        .unwrap();
    let wc_commit = repo.store().get_commit(wc_commit_id).unwrap();
    assert_eq!(wc_commit.tree_id(), repo.store().empty_tree_id());
    assert_eq!(
        wc_commit.store_commit().parents,
        vec![repo.store().root_commit_id().clone()]
    );
    assert_eq!(wc_commit.predecessors(), vec![]);
    assert_eq!(wc_commit.description(), "");
    assert!(wc_commit.is_open());
    assert_eq!(wc_commit.author().name, settings.user_name());
    assert_eq!(wc_commit.author().email, settings.user_email());
    assert_eq!(wc_commit.committer().name, settings.user_name());
    assert_eq!(wc_commit.committer().email, settings.user_email());
}
