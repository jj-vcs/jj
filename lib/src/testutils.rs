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

use std::fs;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use itertools::Itertools;
use tempfile::TempDir;

use crate::backend::{FileId, TreeId, TreeValue};
use crate::commit::Commit;
use crate::commit_builder::CommitBuilder;
use crate::git_backend::GitBackend;
use crate::local_backend::LocalBackend;
use crate::repo::{MutableRepo, ReadonlyRepo};
use crate::repo_path::RepoPath;
use crate::rewrite::RebasedDescendant;
use crate::settings::UserSettings;
use crate::store::Store;
use crate::tree::Tree;
use crate::tree_builder::TreeBuilder;
use crate::workspace::Workspace;

pub fn new_temp_dir() -> TempDir {
    tempfile::Builder::new()
        .prefix("jj-test-")
        .tempdir()
        .unwrap()
}

pub fn new_user_home() -> TempDir {
    // Set $HOME to some arbitrary place so libgit2 doesn't use ~/.gitignore
    // of the person running the tests.
    let home_dir = new_temp_dir();
    std::env::set_var("HOME", home_dir.path());
    home_dir
}

pub fn user_settings() -> UserSettings {
    let config = config::Config::builder()
        .set_override("user.name", "Test User")
        .unwrap()
        .set_override("user.email", "test.user@example.com")
        .unwrap()
        .build()
        .unwrap();
    UserSettings::from_config(config)
}

pub struct TestRepo {
    _temp_dir: TempDir,
    pub repo: Arc<ReadonlyRepo>,
}

impl TestRepo {
    pub fn init(use_git: bool) -> Self {
        let settings = user_settings();
        let temp_dir = new_temp_dir();

        let repo_dir = temp_dir.path().join("repo");
        fs::create_dir(&repo_dir).unwrap();

        let repo = if use_git {
            let git_path = temp_dir.path().join("git-repo");
            git2::Repository::init(&git_path).unwrap();
            ReadonlyRepo::init(&settings, &repo_dir, |store_path| {
                Box::new(GitBackend::init_external(store_path, &git_path))
            })
        } else {
            ReadonlyRepo::init(&settings, &repo_dir, |store_path| {
                Box::new(LocalBackend::init(store_path))
            })
        };

        Self {
            _temp_dir: temp_dir,
            repo,
        }
    }
}

pub struct TestWorkspace {
    temp_dir: TempDir,
    pub workspace: Workspace,
    pub repo: Arc<ReadonlyRepo>,
}

impl TestWorkspace {
    pub fn init(settings: &UserSettings, use_git: bool) -> Self {
        let temp_dir = new_temp_dir();

        let workspace_root = temp_dir.path().join("repo");
        fs::create_dir(&workspace_root).unwrap();

        let (workspace, repo) = if use_git {
            let git_path = temp_dir.path().join("git-repo");
            git2::Repository::init(&git_path).unwrap();
            Workspace::init_external_git(settings, &workspace_root, &git_path).unwrap()
        } else {
            Workspace::init_local(settings, &workspace_root).unwrap()
        };

        Self {
            temp_dir,
            workspace,
            repo,
        }
    }

    pub fn root_dir(&self) -> PathBuf {
        self.temp_dir.path().join("repo").join("..")
    }
}

pub fn read_file(store: &Store, path: &RepoPath, id: &FileId) -> Vec<u8> {
    let mut reader = store.read_file(path, id).unwrap();
    let mut content = vec![];
    reader.read_to_end(&mut content).unwrap();
    content
}

pub fn write_file(store: &Store, path: &RepoPath, contents: &str) -> FileId {
    store.write_file(path, &mut contents.as_bytes()).unwrap()
}

pub fn write_normal_file(tree_builder: &mut TreeBuilder, path: &RepoPath, contents: &str) {
    let id = write_file(tree_builder.store(), path, contents);
    tree_builder.set(
        path.clone(),
        TreeValue::Normal {
            id,
            executable: false,
        },
    );
}

pub fn write_executable_file(tree_builder: &mut TreeBuilder, path: &RepoPath, contents: &str) {
    let id = write_file(tree_builder.store(), path, contents);
    tree_builder.set(
        path.clone(),
        TreeValue::Normal {
            id,
            executable: true,
        },
    );
}

pub fn write_symlink(tree_builder: &mut TreeBuilder, path: &RepoPath, target: &str) {
    let id = tree_builder.store().write_symlink(path, target).unwrap();
    tree_builder.set(path.clone(), TreeValue::Symlink(id));
}

pub fn create_tree(repo: &ReadonlyRepo, path_contents: &[(&RepoPath, &str)]) -> Tree {
    let store = repo.store();
    let mut tree_builder = store.tree_builder(store.empty_tree_id().clone());
    for (path, contents) in path_contents {
        write_normal_file(&mut tree_builder, path, contents);
    }
    let id = tree_builder.write_tree();
    store.get_tree(&RepoPath::root(), &id).unwrap()
}

#[must_use]
pub fn create_random_tree(repo: &ReadonlyRepo) -> TreeId {
    let mut tree_builder = repo
        .store()
        .tree_builder(repo.store().empty_tree_id().clone());
    let number = rand::random::<u32>();
    let path = RepoPath::from_internal_string(format!("file{}", number).as_str());
    write_normal_file(&mut tree_builder, &path, "contents");
    tree_builder.write_tree()
}

#[must_use]
pub fn create_random_commit(settings: &UserSettings, repo: &ReadonlyRepo) -> CommitBuilder {
    let tree_id = create_random_tree(repo);
    let number = rand::random::<u32>();
    CommitBuilder::for_new_commit(
        settings,
        vec![repo.store().root_commit_id().clone()],
        tree_id,
    )
    .set_description(format!("random commit {}", number))
}

pub fn write_working_copy_file(workspace_root: &Path, path: &RepoPath, contents: &str) {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path.to_fs_path(workspace_root))
        .unwrap();
    file.write_all(contents.as_bytes()).unwrap();
}

pub struct CommitGraphBuilder<'settings, 'repo> {
    settings: &'settings UserSettings,
    mut_repo: &'repo mut MutableRepo,
}

impl<'settings, 'repo> CommitGraphBuilder<'settings, 'repo> {
    pub fn new(
        settings: &'settings UserSettings,
        mut_repo: &'repo mut MutableRepo,
    ) -> CommitGraphBuilder<'settings, 'repo> {
        CommitGraphBuilder { settings, mut_repo }
    }

    pub fn initial_commit(&mut self) -> Commit {
        create_random_commit(self.settings, self.mut_repo.base_repo().as_ref())
            .write_to_repo(self.mut_repo)
    }

    pub fn commit_with_parents(&mut self, parents: &[&Commit]) -> Commit {
        let parent_ids = parents
            .iter()
            .map(|commit| commit.id().clone())
            .collect_vec();
        create_random_commit(self.settings, self.mut_repo.base_repo().as_ref())
            .set_parents(parent_ids)
            .write_to_repo(self.mut_repo)
    }
}

pub fn assert_rebased(
    rebased: Option<RebasedDescendant>,
    expected_old_commit: &Commit,
    expected_new_parents: &[&Commit],
) -> Commit {
    if let Some(RebasedDescendant {
        old_commit,
        new_commit,
    }) = rebased
    {
        assert_eq!(old_commit, *expected_old_commit);
        assert_eq!(new_commit.change_id(), expected_old_commit.change_id());
        assert_eq!(
            new_commit.parent_ids(),
            expected_new_parents
                .iter()
                .map(|commit| commit.id().clone())
                .collect_vec()
        );
        new_commit
    } else {
        panic!("expected rebased commit: {:?}", rebased);
    }
}
