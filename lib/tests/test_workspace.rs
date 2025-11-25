// Copyright 2021 The Jujutsu Authors
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

use std::thread;

use assert_matches::assert_matches;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::Repo as _;
use jj_lib::workspace::Workspace;
use jj_lib::workspace::WorkspaceLoadError;
use jj_lib::workspace::default_working_copy_factories;
use jj_lib::workspace::default_working_copy_factory;
use testutils::TestEnvironment;
use testutils::TestWorkspace;

#[test]
fn test_load_bad_path() {
    let settings = testutils::user_settings();
    let test_env = TestEnvironment::init();
    let workspace_root = test_env.root().to_owned();
    // We haven't created a repo in the workspace_root, so it should fail to load.
    let result = Workspace::load(
        &settings,
        &workspace_root,
        &test_env.default_store_factories(),
        &default_working_copy_factories(),
    );
    assert_matches!(
        result.err(),
        Some(WorkspaceLoadError::NoWorkspaceHere(root)) if root == workspace_root
    );
}

#[test]
fn test_init_additional_workspace() {
    let settings = testutils::user_settings();
    let test_workspace = TestWorkspace::init_with_settings(&settings);
    let workspace = &test_workspace.workspace;

    let ws2_name = WorkspaceNameBuf::from("ws2");
    let ws2_root = test_workspace.root_dir().join("ws2_root");
    std::fs::create_dir(&ws2_root).unwrap();
    let (ws2, repo) = Workspace::init_workspace_with_existing_repo(
        &ws2_root,
        test_workspace.repo_path(),
        &test_workspace.repo,
        &*default_working_copy_factory(),
        ws2_name.clone(),
    )
    .unwrap();
    let wc_commit_id = repo.view().get_wc_commit_id(&ws2_name);
    assert_ne!(wc_commit_id, None);
    let wc_commit_id = wc_commit_id.unwrap();
    let wc_commit = repo.store().get_commit(wc_commit_id).unwrap();
    assert_eq!(
        wc_commit.parent_ids(),
        vec![repo.store().root_commit_id().clone()]
    );
    assert_eq!(ws2.workspace_name(), &ws2_name);
    assert_eq!(
        *ws2.repo_path(),
        dunce::canonicalize(workspace.repo_path()).unwrap()
    );
    assert_eq!(
        *ws2.workspace_root(),
        dunce::canonicalize(&ws2_root).unwrap()
    );
    let same_workspace = Workspace::load(
        &settings,
        &ws2_root,
        &test_workspace.env.default_store_factories(),
        &default_working_copy_factories(),
    );
    assert!(same_workspace.is_ok());
    let same_workspace = same_workspace.unwrap();
    assert_eq!(same_workspace.workspace_name(), &ws2_name);
    assert_eq!(
        *same_workspace.repo_path(),
        dunce::canonicalize(workspace.repo_path()).unwrap()
    );
    assert_eq!(same_workspace.workspace_root(), ws2.workspace_root());
}

#[cfg(unix)]
#[test]
fn test_init_additional_workspace_non_utf8_path() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    let settings = testutils::user_settings();
    let test_env = TestEnvironment::init();

    if testutils::check_strict_utf8_fs(test_env.root()) {
        eprintln!(
            "Skipping test \"test_init_additional_workspace_non_utf8_path\" due to strict UTF-8 \
             filesystem for path {:?}",
            test_env.root()
        );
        return;
    }

    let ws1_root = test_env.root().join(OsStr::from_bytes(b"ws1_root\xe0"));
    std::fs::create_dir(&ws1_root).unwrap();
    let (ws1, repo) = Workspace::init_simple(&settings, &ws1_root).unwrap();

    let ws2_name = WorkspaceNameBuf::from("ws2");
    let ws2_root = test_env.root().join(OsStr::from_bytes(b"ws2_root\xe0"));
    std::fs::create_dir(&ws2_root).unwrap();
    let (ws2, _repo) = Workspace::init_workspace_with_existing_repo(
        &ws2_root,
        ws1.repo_path(),
        &repo,
        &*default_working_copy_factory(),
        ws2_name.clone(),
    )
    .unwrap();
    assert_eq!(ws2.workspace_name(), &ws2_name);
    assert_eq!(
        *ws2.repo_path(),
        dunce::canonicalize(ws1.repo_path()).unwrap()
    );
    assert_eq!(
        *ws2.workspace_root(),
        dunce::canonicalize(&ws2_root).unwrap()
    );
    let same_workspace = Workspace::load(
        &settings,
        &ws2_root,
        &test_env.default_store_factories(),
        &default_working_copy_factories(),
    );
    let same_workspace = same_workspace.unwrap();
    assert_eq!(same_workspace.workspace_name(), &ws2_name);
    assert_eq!(
        *same_workspace.repo_path(),
        dunce::canonicalize(ws1.repo_path()).unwrap()
    );
    assert_eq!(same_workspace.workspace_root(), ws2.workspace_root());
}

/// Test cross-thread access to a workspace, which requires it to be Send
#[test]
fn test_sendable() {
    let test_workspace = TestWorkspace::init();
    let root = test_workspace.workspace.workspace_root().to_owned();

    thread::spawn(move || {
        let shared_workspace = test_workspace.workspace;
        assert_eq!(shared_workspace.workspace_root(), &root);
    })
    .join()
    .unwrap();
}
