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

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Once};

use itertools::Itertools;
use jj_lib::backend::{
    Backend, BackendInitError, FileId, MergedTreeId, ObjectId, TreeId, TreeValue,
};
use jj_lib::commit::Commit;
use jj_lib::commit_builder::CommitBuilder;
use jj_lib::git_backend::GitBackend;
use jj_lib::local_backend::LocalBackend;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::{MutableRepo, ReadonlyRepo, Repo, RepoLoader, StoreFactories};
use jj_lib::repo_path::RepoPath;
use jj_lib::rewrite::RebasedDescendant;
use jj_lib::settings::UserSettings;
use jj_lib::store::Store;
use jj_lib::transaction::Transaction;
use jj_lib::tree::Tree;
use jj_lib::tree_builder::TreeBuilder;
use jj_lib::working_copy::{SnapshotError, SnapshotOptions};
use jj_lib::workspace::Workspace;
use tempfile::TempDir;

pub fn hermetic_libgit2() {
    // libgit2 respects init.defaultBranch (and possibly other config
    // variables) in the user's config files. Disable access to them to make
    // our tests hermetic.
    //
    // set_search_path is unsafe because it cannot guarantee thread safety (as
    // its documentation states). For the same reason, we wrap these invocations
    // in `call_once`.
    static CONFIGURE_GIT2: Once = Once::new();
    CONFIGURE_GIT2.call_once(|| unsafe {
        git2::opts::set_search_path(git2::ConfigLevel::System, "").unwrap();
        git2::opts::set_search_path(git2::ConfigLevel::Global, "").unwrap();
        git2::opts::set_search_path(git2::ConfigLevel::XDG, "").unwrap();
        git2::opts::set_search_path(git2::ConfigLevel::ProgramData, "").unwrap();
    });
}

pub fn new_temp_dir() -> TempDir {
    hermetic_libgit2();
    tempfile::Builder::new()
        .prefix("jj-test-")
        .tempdir()
        .unwrap()
}

pub fn base_config() -> config::ConfigBuilder<config::builder::DefaultState> {
    config::Config::builder().add_source(config::File::from_str(
        r#"
            user.name = "Test User"
            user.email = "test.user@example.com"
            operation.username = "test-username"
            operation.hostname = "host.example.com"
            debug.randomness-seed = "42"
        "#,
        config::FileFormat::Toml,
    ))
}

pub fn user_settings() -> UserSettings {
    let config = base_config().build().unwrap();
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
            ReadonlyRepo::init(
                &settings,
                &repo_dir,
                |store_path| -> Result<Box<dyn Backend>, BackendInitError> {
                    Ok(Box::new(GitBackend::init_external(store_path, &git_path)?))
                },
                ReadonlyRepo::default_op_store_factory(),
                ReadonlyRepo::default_op_heads_store_factory(),
                ReadonlyRepo::default_index_store_factory(),
                ReadonlyRepo::default_submodule_store_factory(),
            )
            .unwrap()
        } else {
            ReadonlyRepo::init(
                &settings,
                &repo_dir,
                |store_path| -> Result<Box<dyn Backend>, BackendInitError> {
                    Ok(Box::new(LocalBackend::init(store_path)))
                },
                ReadonlyRepo::default_op_store_factory(),
                ReadonlyRepo::default_op_heads_store_factory(),
                ReadonlyRepo::default_index_store_factory(),
                ReadonlyRepo::default_submodule_store_factory(),
            )
            .unwrap()
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
    settings: UserSettings,
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
            settings: settings.clone(),
        }
    }

    pub fn root_dir(&self) -> PathBuf {
        self.temp_dir.path().join("repo").join("..")
    }

    /// Snapshots the working copy and returns the tree. Updates the working
    /// copy state on disk, but does not update the working-copy commit (no
    /// new operation).
    pub fn snapshot(&mut self) -> Result<MergedTree, SnapshotError> {
        let mut locked_wc = self.workspace.working_copy_mut().start_mutation().unwrap();
        let tree_id = locked_wc.snapshot(SnapshotOptions {
            max_new_file_size: self.settings.max_new_file_size().unwrap(),
            ..SnapshotOptions::empty_for_test()
        })?;
        // arbitrary operation id
        locked_wc.finish(self.repo.op_id().clone()).unwrap();
        Ok(self.repo.store().get_root_tree(&tree_id).unwrap())
    }
}

pub fn load_repo_at_head(settings: &UserSettings, repo_path: &Path) -> Arc<ReadonlyRepo> {
    RepoLoader::init(settings, repo_path, &StoreFactories::default())
        .unwrap()
        .load_at_head(settings)
        .unwrap()
}

pub fn commit_transactions(settings: &UserSettings, txs: Vec<Transaction>) -> Arc<ReadonlyRepo> {
    let repo_loader = txs[0].base_repo().loader();
    let mut op_ids = vec![];
    for tx in txs {
        op_ids.push(tx.commit().op_id().clone());
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let repo = repo_loader.load_at_head(settings).unwrap();
    // Test the setup. The assumption here is that the parent order matches the
    // order in which they were merged (which currently matches the transaction
    // commit order), so we want to know make sure they appear in a certain
    // order, so the caller can decide the order by passing them to this
    // function in a certain order.
    assert_eq!(*repo.operation().parent_ids(), op_ids);
    repo
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

pub fn write_normal_file(
    tree_builder: &mut TreeBuilder,
    path: &RepoPath,
    contents: &str,
) -> FileId {
    let id = write_file(tree_builder.store(), path, contents);
    tree_builder.set(
        path.clone(),
        TreeValue::File {
            id: id.clone(),
            executable: false,
        },
    );
    id
}

pub fn write_executable_file(tree_builder: &mut TreeBuilder, path: &RepoPath, contents: &str) {
    let id = write_file(tree_builder.store(), path, contents);
    tree_builder.set(
        path.clone(),
        TreeValue::File {
            id,
            executable: true,
        },
    );
}

pub fn write_symlink(tree_builder: &mut TreeBuilder, path: &RepoPath, target: &str) {
    let id = tree_builder.store().write_symlink(path, target).unwrap();
    tree_builder.set(path.clone(), TreeValue::Symlink(id));
}

pub fn create_single_tree(repo: &Arc<ReadonlyRepo>, path_contents: &[(&RepoPath, &str)]) -> Tree {
    let store = repo.store();
    let mut tree_builder = store.tree_builder(store.empty_tree_id().clone());
    for (path, contents) in path_contents {
        write_normal_file(&mut tree_builder, path, contents);
    }
    let id = tree_builder.write_tree();
    store.get_tree(&RepoPath::root(), &id).unwrap()
}

pub fn create_tree(repo: &Arc<ReadonlyRepo>, path_contents: &[(&RepoPath, &str)]) -> MergedTree {
    MergedTree::legacy(create_single_tree(repo, path_contents))
}

#[must_use]
pub fn create_random_tree(repo: &Arc<ReadonlyRepo>) -> MergedTreeId {
    let mut tree_builder = repo
        .store()
        .tree_builder(repo.store().empty_tree_id().clone());
    let number = rand::random::<u32>();
    let path = RepoPath::from_internal_string(format!("file{number}").as_str());
    write_normal_file(&mut tree_builder, &path, "contents");
    MergedTreeId::Legacy(tree_builder.write_tree())
}

pub fn create_random_commit<'repo>(
    mut_repo: &'repo mut MutableRepo,
    settings: &UserSettings,
) -> CommitBuilder<'repo> {
    let tree_id = create_random_tree(mut_repo.base_repo());
    let number = rand::random::<u32>();
    mut_repo
        .new_commit(
            settings,
            vec![mut_repo.store().root_commit_id().clone()],
            tree_id,
        )
        .set_description(format!("random commit {number}"))
}

pub fn dump_tree(store: &Arc<Store>, tree_id: &TreeId) -> String {
    use std::fmt::Write;
    let mut buf = String::new();
    writeln!(&mut buf, "tree {}", tree_id.hex()).unwrap();
    let tree = store.get_tree(&RepoPath::root(), tree_id).unwrap();
    for (path, value) in tree.entries() {
        match value {
            TreeValue::File { id, executable: _ } => {
                let file_buf = read_file(store, &path, &id);
                let file_contents = String::from_utf8_lossy(&file_buf);
                writeln!(
                    &mut buf,
                    "  file {path:?} ({}): {file_contents:?}",
                    id.hex()
                )
                .unwrap();
            }
            TreeValue::Symlink(id) => {
                writeln!(&mut buf, "  symlink {path:?} ({})", id.hex()).unwrap();
            }
            TreeValue::Conflict(id) => {
                writeln!(&mut buf, "  conflict {path:?} ({})", id.hex()).unwrap();
            }
            TreeValue::GitSubmodule(id) => {
                writeln!(&mut buf, "  submodule {path:?} ({})", id.hex()).unwrap();
            }
            entry => {
                unimplemented!("dumping tree entry {entry:?}");
            }
        }
    }
    buf
}

pub fn write_random_commit(mut_repo: &mut MutableRepo, settings: &UserSettings) -> Commit {
    create_random_commit(mut_repo, settings).write().unwrap()
}

pub fn write_working_copy_file(workspace_root: &Path, path: &RepoPath, contents: &str) {
    let path = path.to_fs_path(workspace_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
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
        write_random_commit(self.mut_repo, self.settings)
    }

    pub fn commit_with_parents(&mut self, parents: &[&Commit]) -> Commit {
        let parent_ids = parents
            .iter()
            .map(|commit| commit.id().clone())
            .collect_vec();
        create_random_commit(self.mut_repo, self.settings)
            .set_parents(parent_ids)
            .write()
            .unwrap()
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
        panic!("expected rebased commit: {rebased:?}");
    }
}
