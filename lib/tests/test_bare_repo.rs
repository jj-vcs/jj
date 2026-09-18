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

//! Tests for bare repo support (no default workspace, no working copy).

use std::fs;
use std::sync::Arc;

use assert_matches::assert_matches;
use jj_lib::default_backend_factories::default_working_copy_factories;
use jj_lib::default_backend_factories::default_working_copy_factory;
use jj_lib::object_id::ObjectId as _;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::signing_factory::signer_from_settings;
use jj_lib::simple_backend::SimpleBackend;
use jj_lib::workspace::BARE_MARKER;
use jj_lib::workspace::Workspace;
use jj_lib::workspace::WorkspaceLoadError;
use pollster::FutureExt as _;
use testutils::TestResult;

fn make_backend_initializer() -> &'static jj_lib::repo::BackendInitializer<'static> {
    &|_settings, store_path| Ok(Box::new(SimpleBackend::init(store_path)))
}

/// `Workspace::init_bare_with_backend` creates a bare repo with the marker file.
#[test]
fn test_bare_repo_init() -> TestResult {
    let settings = testutils::user_settings();
    let temp_dir = testutils::new_temp_dir();
    // Write path to a known location for manual inspection
    std::fs::write(
        "/tmp/jj-bare-demo-path",
        temp_dir.path().display().to_string(),
    )
    .ok();
    let project_root = temp_dir.path().join("my-project");
    fs::create_dir(&project_root)?;

    let signer = signer_from_settings(&settings)?;
    let _repo: Arc<ReadonlyRepo> = Workspace::init_bare_with_backend(
        &settings,
        &project_root,
        make_backend_initializer(),
        signer,
    )
    .block_on()?;

    let jj_dir = project_root.join(".jj");
    let repo_dir = jj_dir.join("repo");

    assert!(
        repo_dir.join("store").is_dir(),
        ".jj/repo/store/ should exist"
    );
    assert!(
        repo_dir.join("op_store").is_dir(),
        ".jj/repo/op_store/ should exist"
    );
    assert!(
        repo_dir.join("op_heads").is_dir(),
        ".jj/repo/op_heads/ should exist"
    );

    assert!(
        !jj_dir.join("working_copy").exists(),
        ".jj/working_copy/ should NOT exist in a bare repo"
    );

    assert!(
        jj_dir.join(BARE_MARKER).exists(),
        ".jj/bare marker should exist"
    );

    let store_factories = testutils::TestEnvironment::init().default_backend_factories();
    let result = Workspace::load(
        &settings,
        &project_root,
        &store_factories,
        &default_working_copy_factories(),
    );
    assert_matches!(
        result.err(),
        Some(WorkspaceLoadError::BareRepoHere(p)) if p == project_root
    );

    // Keep temp dir alive for manual inspection
    std::mem::forget(temp_dir);

    Ok(())
}

/// A workspace can be added to a bare repo and points back to the repo.
#[test]
fn test_workspace_add_to_bare_repo() -> TestResult {
    let settings = testutils::user_settings();
    let temp_dir = testutils::new_temp_dir();
    let project_root = temp_dir.path().join("my-project");
    fs::create_dir(&project_root)?;

    let signer = signer_from_settings(&settings)?;
    let repo: Arc<ReadonlyRepo> = Workspace::init_bare_with_backend(
        &settings,
        &project_root,
        make_backend_initializer(),
        signer,
    )
    .block_on()?;

    let jj_dir = project_root.join(".jj");
    let repo_dir = jj_dir.join("repo");

    assert!(!jj_dir.join("working_copy").exists());

    let main_dir = project_root.join("main");
    fs::create_dir(&main_dir)?;

    let (workspace, _repo) = Workspace::init_workspace_with_existing_repo(
        &main_dir,
        &repo_dir,
        &repo,
        &*default_working_copy_factory(),
        WorkspaceNameBuf::from("main"),
    )
    .block_on()?;

    assert_eq!(*workspace.repo_path(), dunce::canonicalize(&repo_dir)?);
    assert_eq!(*workspace.workspace_root(), dunce::canonicalize(&main_dir)?);

    assert!(main_dir.join(".jj").join("working_copy").is_dir());

    assert!(!jj_dir.join("working_copy").exists());

    Ok(())
}

/// A workspace created from a bare repo loads correctly and is fully usable.
#[test]
fn test_workspace_load_and_use_after_bare_repo_init() -> TestResult {
    let settings = testutils::user_settings();
    let temp_dir = testutils::new_temp_dir();
    // Write path to a known location for manual inspection
    std::fs::write(
        "/tmp/jj-bare-demo-path",
        temp_dir.path().display().to_string(),
    )
    .ok();
    let project_root = temp_dir.path().join("my-project");
    fs::create_dir(&project_root)?;

    let signer = signer_from_settings(&settings)?;
    let repo: Arc<ReadonlyRepo> = Workspace::init_bare_with_backend(
        &settings,
        &project_root,
        make_backend_initializer(),
        signer,
    )
    .block_on()?;

    let jj_dir = project_root.join(".jj");
    let repo_dir = jj_dir.join("repo");
    let main_dir = project_root.join("main");
    fs::create_dir(&main_dir)?;
    let ws_name = WorkspaceNameBuf::from("main");

    let (_workspace, repo) = Workspace::init_workspace_with_existing_repo(
        &main_dir,
        &repo_dir,
        &repo,
        &*default_working_copy_factory(),
        ws_name.clone(),
    )
    .block_on()?;

    let wc_commit_id = repo.view().get_wc_commit_id(&ws_name);
    assert!(
        wc_commit_id.is_some(),
        "workspace should have a working copy commit"
    );

    let store_factories = testutils::TestEnvironment::init().default_backend_factories();
    let loaded = Workspace::load(
        &settings,
        &main_dir,
        &store_factories,
        &default_working_copy_factories(),
    )?;

    assert_eq!(*loaded.repo_path(), dunce::canonicalize(&repo_dir)?);
    assert_eq!(loaded.workspace_name(), WorkspaceName::new("main"));

    let loaded_repo = loaded.repo_loader().load_at_head().block_on()?;
    let wc_id = loaded_repo
        .view()
        .get_wc_commit_id(WorkspaceName::new("main"))
        .expect("working copy commit should exist");
    let wc_commit = loaded_repo.store().get_commit(wc_id)?;
    assert_eq!(
        wc_commit.parent_ids(),
        &[loaded_repo.store().root_commit_id().clone()]
    );

    println!(
        "workspace name:       {}",
        loaded.workspace_name().as_symbol()
    );
    println!("working copy commit:  {}", wc_id.hex());
    println!("parent:               {}", wc_commit.parent_ids()[0].hex());

    // Keep temp dir alive for manual inspection
    std::mem::forget(temp_dir);

    Ok(())
}
