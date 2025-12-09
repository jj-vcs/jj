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

use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;

use itertools::Itertools as _;
use jj_lib::backend;
use jj_lib::backend::Backend;
use jj_lib::backend::BackendInitError;
use jj_lib::backend::ChangeId;
use jj_lib::backend::CommitId;
use jj_lib::backend::CopyId;
use jj_lib::backend::FileId;
use jj_lib::backend::MillisSinceEpoch;
use jj_lib::backend::Signature;
use jj_lib::backend::Timestamp;
use jj_lib::backend::TreeValue;
use jj_lib::commit::Commit;
use jj_lib::commit_builder::CommitBuilder;
use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigSource;
use jj_lib::config::StackedConfig;
use jj_lib::git_backend::GitBackend;
use jj_lib::gitignore::GitIgnoreFile;
use jj_lib::matchers::EverythingMatcher;
use jj_lib::matchers::NothingMatcher;
use jj_lib::merged_tree::MergedTree;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo::MutableRepo;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo;
use jj_lib::repo::RepoLoader;
use jj_lib::repo::StoreFactories;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::repo_path::RepoPathComponent;
use jj_lib::rewrite::RebaseOptions;
use jj_lib::rewrite::RebasedCommit;
use jj_lib::secret_backend::SecretBackend;
use jj_lib::settings::UserSettings;
use jj_lib::signing::Signer;
use jj_lib::simple_backend::SimpleBackend;
use jj_lib::store::Store;
use jj_lib::transaction::Transaction;
use jj_lib::tree::Tree;
use jj_lib::tree_builder::TreeBuilder;
use jj_lib::working_copy::SnapshotError;
use jj_lib::working_copy::SnapshotOptions;
use jj_lib::working_copy::SnapshotStats;
use jj_lib::workspace::Workspace;
use pollster::FutureExt as _;
use tempfile::TempDir;
use tokio::io::AsyncReadExt as _;

use crate::test_backend::TestBackendFactory;

pub mod git;
pub mod proptest;
pub mod test_backend;

// TODO: Consider figuring out a way to make `GitBackend` and `git(1)` calls in
// tests ignore external configuration and removing this function. This is
// somewhat tricky because `gix` looks at system and user configuration, and
// `GitBackend` also calls into `git(1)` for things like garbage collection.
pub fn hermetic_git() {
    unsafe {
        // Prevent GitBackend from loading user and system configurations. For
        // gitoxide API use in tests, Config::isolated() is probably better.
        env::set_var("GIT_CONFIG_SYSTEM", "/dev/null");
        env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
        // gitoxide uses "main" as the default branch name, whereas git
        // uses "master".
        env::set_var("GIT_CONFIG_KEY_0", "init.defaultBranch");
        env::set_var("GIT_CONFIG_VALUE_0", "master");
        env::set_var("GIT_CONFIG_COUNT", "1");
    }
}

pub fn new_temp_dir() -> TempDir {
    hermetic_git();
    tempfile::Builder::new()
        .prefix("jj-test-")
        .tempdir()
        .unwrap()
}

/// Returns new low-level config object that includes fake user configuration
/// needed to run basic operations.
pub fn base_user_config() -> StackedConfig {
    let config_text = r#"
        user.name = "Test User"
        user.email = "test.user@example.com"
        operation.username = "test-username"
        operation.hostname = "host.example.com"
        debug.randomness-seed = 42
    "#;
    let mut config = StackedConfig::with_defaults();
    config.add_layer(ConfigLayer::parse(ConfigSource::User, config_text).unwrap());
    config
}

/// Returns new immutable settings object that includes fake user configuration
/// needed to run basic operations.
pub fn user_settings() -> UserSettings {
    UserSettings::from_config(base_user_config()).unwrap()
}

/// Creates [`SnapshotOptions`] for use in tests.
pub fn empty_snapshot_options() -> SnapshotOptions<'static> {
    SnapshotOptions {
        base_ignores: GitIgnoreFile::empty(),
        progress: None,
        start_tracking_matcher: &EverythingMatcher,
        force_tracking_matcher: &NothingMatcher,
        max_new_file_size: u64::MAX,
    }
}

/// Panic if `CI` environment variable is set to a non-empty value
///
/// Most CI environments set this variable automatically. See e.g.
/// <https://docs.github.com/en/actions/writing-workflows/choosing-what-your-workflow-does/store-information-in-variables#default-environment-variables>
#[track_caller]
pub fn ensure_running_outside_ci(reason: &str) {
    let running_in_ci = std::env::var("CI").is_ok_and(|value| !value.is_empty());
    assert!(!running_in_ci, "Running in CI, {reason}.");
}

/// Tests if an external tool is installed and in the PATH
pub fn is_external_tool_installed(program_name: impl AsRef<OsStr>) -> bool {
    Command::new(program_name)
        .arg("--version")
        .stdout(Stdio::null())
        .status()
        .is_ok()
}

#[derive(Debug)]
pub struct TestEnvironment {
    temp_dir: TempDir,
    test_backend_factory: TestBackendFactory,
}

impl TestEnvironment {
    pub fn init() -> Self {
        Self {
            temp_dir: new_temp_dir(),
            test_backend_factory: TestBackendFactory::default(),
        }
    }

    pub fn root(&self) -> &Path {
        self.temp_dir.path()
    }

    pub fn default_store_factories(&self) -> StoreFactories {
        let mut factories = StoreFactories::default();
        factories.add_backend("test", {
            let factory = self.test_backend_factory.clone();
            Box::new(move |_settings, store_path| Ok(Box::new(factory.load(store_path))))
        });
        factories.add_backend(
            SecretBackend::name(),
            Box::new(|settings, store_path| {
                Ok(Box::new(SecretBackend::load(settings, store_path)?))
            }),
        );
        factories
    }

    pub fn load_repo_at_head(
        &self,
        settings: &UserSettings,
        repo_path: &Path,
    ) -> Arc<ReadonlyRepo> {
        RepoLoader::init_from_file_system(settings, repo_path, &self.default_store_factories())
            .unwrap()
            .load_at_head()
            .unwrap()
    }
}

pub struct TestRepo {
    pub env: TestEnvironment,
    pub repo: Arc<ReadonlyRepo>,
    repo_path: PathBuf,
}

#[derive(PartialEq, Eq, Copy, Clone)]
pub enum TestRepoBackend {
    Git,
    Simple,
    Test,
}

impl TestRepoBackend {
    fn init_backend(
        &self,
        env: &TestEnvironment,
        settings: &UserSettings,
        store_path: &Path,
    ) -> Result<Box<dyn Backend>, BackendInitError> {
        match self {
            Self::Git => Ok(Box::new(GitBackend::init_internal(settings, store_path)?)),
            Self::Simple => Ok(Box::new(SimpleBackend::init(store_path))),
            Self::Test => Ok(Box::new(env.test_backend_factory.init(store_path))),
        }
    }
}

impl TestRepo {
    pub fn init() -> Self {
        Self::init_with_backend(TestRepoBackend::Test)
    }

    pub fn init_with_backend(backend: TestRepoBackend) -> Self {
        Self::init_with_backend_and_settings(backend, &user_settings())
    }

    pub fn init_with_settings(settings: &UserSettings) -> Self {
        Self::init_with_backend_and_settings(TestRepoBackend::Test, settings)
    }

    pub fn init_with_backend_and_settings(
        backend: TestRepoBackend,
        settings: &UserSettings,
    ) -> Self {
        let env = TestEnvironment::init();

        let repo_dir = env.root().join("repo");
        fs::create_dir(&repo_dir).unwrap();

        let repo = ReadonlyRepo::init(
            settings,
            &repo_dir,
            &|settings, store_path| backend.init_backend(&env, settings, store_path),
            Signer::from_settings(settings).unwrap(),
            ReadonlyRepo::default_op_store_initializer(),
            ReadonlyRepo::default_op_heads_store_initializer(),
            ReadonlyRepo::default_index_store_initializer(),
            ReadonlyRepo::default_submodule_store_initializer(),
        )
        .unwrap();

        Self {
            env,
            repo,
            repo_path: repo_dir,
        }
    }

    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }
}

pub struct TestWorkspace {
    pub env: TestEnvironment,
    pub workspace: Workspace,
    pub repo: Arc<ReadonlyRepo>,
}

impl TestWorkspace {
    pub fn init() -> Self {
        Self::init_with_backend(TestRepoBackend::Test)
    }

    pub fn init_with_backend(backend: TestRepoBackend) -> Self {
        Self::init_with_backend_and_settings(backend, &user_settings())
    }

    pub fn init_with_settings(settings: &UserSettings) -> Self {
        Self::init_with_backend_and_settings(TestRepoBackend::Test, settings)
    }

    pub fn init_with_backend_and_settings(
        backend: TestRepoBackend,
        settings: &UserSettings,
    ) -> Self {
        let signer = Signer::from_settings(settings).unwrap();
        Self::init_with_backend_and_signer(backend, signer, settings)
    }

    pub fn init_with_backend_and_signer(
        backend: TestRepoBackend,
        signer: Signer,
        settings: &UserSettings,
    ) -> Self {
        let env = TestEnvironment::init();

        let workspace_root = env.root().join("repo");
        fs::create_dir(&workspace_root).unwrap();

        let (workspace, repo) = Workspace::init_with_backend(
            settings,
            &workspace_root,
            &|settings, store_path| backend.init_backend(&env, settings, store_path),
            signer,
        )
        .unwrap();

        Self {
            env,
            workspace,
            repo,
        }
    }

    pub fn root_dir(&self) -> PathBuf {
        self.env.root().join("repo").join("..")
    }

    pub fn repo_path(&self) -> &Path {
        self.workspace.repo_path()
    }

    /// Snapshots the working copy and returns the tree. Updates the working
    /// copy state on disk, but does not update the working-copy commit (no
    /// new operation).
    pub fn snapshot_with_options(
        &mut self,
        options: &SnapshotOptions,
    ) -> Result<(MergedTree, SnapshotStats), SnapshotError> {
        let mut locked_ws = self.workspace.start_working_copy_mutation().unwrap();
        let (tree, stats) = locked_ws.locked_wc().snapshot(options).block_on()?;
        // arbitrary operation id
        locked_ws.finish(self.repo.op_id().clone()).unwrap();
        Ok((tree, stats))
    }

    /// Like `snapshot_with_option()` but with default options
    pub fn snapshot(&mut self) -> Result<MergedTree, SnapshotError> {
        let (tree, _stats) = self.snapshot_with_options(&empty_snapshot_options())?;
        Ok(tree)
    }
}

pub fn commit_transactions(txs: Vec<Transaction>) -> Arc<ReadonlyRepo> {
    let repo_loader = txs[0].base_repo().loader().clone();
    let mut op_ids = vec![];
    for tx in txs {
        op_ids.push(tx.commit("test").unwrap().op_id().clone());
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let repo = repo_loader.load_at_head().unwrap();
    // Test the setup. The assumption here is that the parent order matches the
    // order in which they were merged (which currently matches the transaction
    // commit order), so we want to know make sure they appear in a certain
    // order, so the caller can decide the order by passing them to this
    // function in a certain order.
    assert_eq!(*repo.operation().parent_ids(), op_ids);
    repo
}

pub fn repo_path_component(value: &str) -> &RepoPathComponent {
    RepoPathComponent::new(value).unwrap()
}

pub fn repo_path(value: &str) -> &RepoPath {
    RepoPath::from_internal_string(value).unwrap()
}

pub fn repo_path_buf(value: impl Into<String>) -> RepoPathBuf {
    RepoPathBuf::from_internal_string(value).unwrap()
}

pub fn read_file(store: &Store, path: &RepoPath, id: &FileId) -> Vec<u8> {
    let mut reader = store.read_file(path, id).block_on().unwrap();
    let mut content = vec![];
    reader.read_to_end(&mut content).block_on().unwrap();
    content
}

pub fn write_file(store: &Store, path: &RepoPath, contents: &str) -> FileId {
    store
        .write_file(path, &mut contents.as_bytes())
        .block_on()
        .unwrap()
}

pub struct TestTreeBuilder {
    store: Arc<Store>,
    tree_builder: TreeBuilder,
}

impl TestTreeBuilder {
    pub fn new(store: Arc<Store>) -> Self {
        let tree_builder = TreeBuilder::new(store.clone(), store.empty_tree_id().clone());
        Self {
            store,
            tree_builder,
        }
    }

    pub fn file(
        &mut self,
        path: &RepoPath,
        contents: impl AsRef<[u8]>,
    ) -> TestTreeFileEntryBuilder<'_> {
        TestTreeFileEntryBuilder {
            tree_builder: &mut self.tree_builder,
            path: path.to_owned(),
            contents: contents.as_ref().to_vec(),
            executable: false,
        }
    }

    pub fn symlink(&mut self, path: &RepoPath, target: &str) {
        let id = self.store.write_symlink(path, target).block_on().unwrap();
        self.tree_builder
            .set(path.to_owned(), TreeValue::Symlink(id));
    }

    pub fn submodule(&mut self, path: &RepoPath, commit: CommitId) {
        self.tree_builder
            .set(path.to_owned(), TreeValue::GitSubmodule(commit));
    }

    pub fn write_single_tree(self) -> Tree {
        let id = self.tree_builder.write_tree().unwrap();
        self.store.get_tree(RepoPathBuf::root(), &id).unwrap()
    }

    pub fn write_merged_tree(self) -> MergedTree {
        let id = self.tree_builder.write_tree().unwrap();
        MergedTree::resolved(self.store, id)
    }
}

pub struct TestTreeFileEntryBuilder<'a> {
    tree_builder: &'a mut TreeBuilder,
    path: RepoPathBuf,
    contents: Vec<u8>,
    executable: bool,
}

impl TestTreeFileEntryBuilder<'_> {
    pub fn executable(mut self, executable: bool) -> Self {
        self.executable = executable;
        self
    }
}

impl Drop for TestTreeFileEntryBuilder<'_> {
    fn drop(&mut self) {
        let id = self
            .tree_builder
            .store()
            .write_file(&self.path, &mut self.contents.as_slice())
            .block_on()
            .unwrap();
        let path = std::mem::replace(&mut self.path, RepoPathBuf::root());
        self.tree_builder.set(
            path,
            TreeValue::File {
                id,
                executable: self.executable,
                copy_id: CopyId::placeholder(),
            },
        );
    }
}

pub fn create_single_tree_with(
    repo: &Arc<ReadonlyRepo>,
    build: impl FnOnce(&mut TestTreeBuilder),
) -> Tree {
    let mut builder = TestTreeBuilder::new(repo.store().clone());
    build(&mut builder);
    builder.write_single_tree()
}

pub fn create_single_tree(repo: &Arc<ReadonlyRepo>, path_contents: &[(&RepoPath, &str)]) -> Tree {
    create_single_tree_with(repo, |builder| {
        for (path, contents) in path_contents {
            builder.file(path, contents);
        }
    })
}

pub fn create_tree_with(
    repo: &Arc<ReadonlyRepo>,
    build: impl FnOnce(&mut TestTreeBuilder),
) -> MergedTree {
    let mut builder = TestTreeBuilder::new(repo.store().clone());
    build(&mut builder);
    builder.write_merged_tree()
}

pub fn create_tree(repo: &Arc<ReadonlyRepo>, path_contents: &[(&RepoPath, &str)]) -> MergedTree {
    create_tree_with(repo, |builder| {
        for (path, contents) in path_contents {
            builder.file(path, contents);
        }
    })
}

#[must_use]
pub fn create_random_tree(repo: &Arc<ReadonlyRepo>) -> MergedTree {
    let number = rand::random::<u32>();
    let path = repo_path_buf(format!("file{number}"));
    create_tree_with(repo, |builder| {
        builder.file(&path, "contents");
    })
}

pub fn create_random_commit(mut_repo: &mut MutableRepo) -> CommitBuilder<'_> {
    let tree = create_random_tree(mut_repo.base_repo());
    let number = rand::random::<u32>();
    mut_repo
        .new_commit(vec![mut_repo.store().root_commit_id().clone()], tree)
        .set_description(format!("random commit {number}"))
}

pub fn commit_with_tree(store: &Arc<Store>, tree: MergedTree) -> Commit {
    let signature = Signature {
        name: "Some One".to_string(),
        email: "someone@example.com".to_string(),
        timestamp: Timestamp {
            timestamp: MillisSinceEpoch(0),
            tz_offset: 0,
        },
    };
    let commit = backend::Commit {
        parents: vec![store.root_commit_id().clone()],
        predecessors: vec![],
        root_tree: tree.into_tree_ids(),
        change_id: ChangeId::from_hex("abcd"),
        description: "description".to_string(),
        author: signature.clone(),
        committer: signature,
        secure_sig: None,
    };
    store.write_commit(commit, None).block_on().unwrap()
}

pub fn dump_tree(merged_tree: &MergedTree) -> String {
    use std::fmt::Write as _;
    let store = merged_tree.store();
    let mut buf = String::new();
    let trees = merged_tree.trees().unwrap();
    writeln!(&mut buf, "merged tree (sides: {})", trees.num_sides()).unwrap();
    for tree in trees.iter() {
        writeln!(&mut buf, "  tree {}", tree.id()).unwrap();
        for (path, entry) in tree.entries_matching(&EverythingMatcher) {
            match entry {
                TreeValue::File {
                    id,
                    executable: _,
                    copy_id: _,
                } => {
                    let file_buf = read_file(store, &path, &id);
                    let file_contents = String::from_utf8_lossy(&file_buf);
                    writeln!(&mut buf, "    file {path:?} ({id}): {file_contents:?}").unwrap();
                }
                TreeValue::Symlink(id) => {
                    writeln!(&mut buf, "    symlink {path:?} ({id})").unwrap();
                }
                TreeValue::GitSubmodule(id) => {
                    writeln!(&mut buf, "    submodule {path:?} ({id})").unwrap();
                }
                _ => {
                    writeln!(&mut buf, "    entry {path:?}: {entry:?}").unwrap();
                }
            }
        }
    }
    buf
}

#[macro_export]
macro_rules! assert_tree_eq {
    ($left_tree:expr, $right_tree:expr $(,)?) => {
        assert_tree_eq!($left_tree, $right_tree, "trees are different")
    };
    ($left_tree:expr, $right_tree:expr, $($args:tt)+) => {{
        let left_tree: &::jj_lib::merged_tree::MergedTree = &$left_tree;
        let right_tree: &::jj_lib::merged_tree::MergedTree = &$right_tree;
        assert_eq!(
            left_tree.tree_ids(),
            right_tree.tree_ids(),
            "{}:\n left: {}\nright: {}",
            format_args!($($args)*),
            $crate::dump_tree(left_tree),
            $crate::dump_tree(right_tree),
        )
    }};
}

pub fn write_random_commit(mut_repo: &mut MutableRepo) -> Commit {
    write_random_commit_with_parents(mut_repo, &[])
}

pub fn write_random_commit_with_parents(mut_repo: &mut MutableRepo, parents: &[&Commit]) -> Commit {
    let parents = if parents.is_empty() {
        &[&mut_repo.store().root_commit()]
    } else {
        parents
    };
    create_random_commit(mut_repo)
        .set_parents(parents.iter().map(|commit| commit.id().clone()).collect())
        .write()
        .unwrap()
}

pub fn write_working_copy_file(workspace_root: &Path, path: &RepoPath, contents: impl AsRef<[u8]>) {
    let path = path.to_fs_path(workspace_root).unwrap();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .unwrap();
    file.write_all(contents.as_ref()).unwrap();
}

/// Rebase descendants of the rewritten commits. Returns map of original commit
/// ID to rebased (or abandoned parent) commit ID.
pub fn rebase_descendants_with_options_return_map(
    repo: &mut MutableRepo,
    options: &RebaseOptions,
) -> HashMap<CommitId, CommitId> {
    let mut rebased: HashMap<CommitId, CommitId> = HashMap::new();
    repo.rebase_descendants_with_options(options, |old_commit, rebased_commit| {
        let old_commit_id = old_commit.id().clone();
        let new_commit_id = match rebased_commit {
            RebasedCommit::Rewritten(new_commit) => new_commit.id().clone(),
            RebasedCommit::Abandoned { parent_id } => parent_id,
        };
        rebased.insert(old_commit_id, new_commit_id);
    })
    .unwrap();
    rebased
}

fn assert_in_rebased_map(
    repo: &impl Repo,
    rebased: &HashMap<CommitId, CommitId>,
    expected_old_commit: &Commit,
) -> Commit {
    let new_commit_id = rebased.get(expected_old_commit.id()).unwrap_or_else(|| {
        panic!(
            "Expected commit to have been rebased: {}",
            expected_old_commit.id().hex()
        )
    });
    repo.store().get_commit(new_commit_id).unwrap().clone()
}

pub fn assert_rebased_onto(
    repo: &impl Repo,
    rebased: &HashMap<CommitId, CommitId>,
    expected_old_commit: &Commit,
    expected_new_parent_ids: &[&CommitId],
) -> Commit {
    let new_commit = assert_in_rebased_map(repo, rebased, expected_old_commit);
    assert_eq!(
        new_commit.parent_ids().to_vec(),
        expected_new_parent_ids
            .iter()
            .map(|x| (*x).clone())
            .collect_vec()
    );
    assert_eq!(new_commit.change_id(), expected_old_commit.change_id());
    new_commit
}

/// Maps children of an abandoned commit to a new rebase target.
///
/// If `expected_old_commit` was abandoned, the `rebased` map indicates the
/// commit the children of `expected_old_commit` should be rebased to, which
/// would have a different change id. This happens when the EmptyBehavior in
/// RebaseOptions is not the default; because of the details of the
/// implementation this returned parent commit is always singular.
pub fn assert_abandoned_with_parent(
    repo: &impl Repo,
    rebased: &HashMap<CommitId, CommitId>,
    expected_old_commit: &Commit,
    expected_new_parent_id: &CommitId,
) -> Commit {
    let new_parent_commit = assert_in_rebased_map(repo, rebased, expected_old_commit);
    assert_eq!(new_parent_commit.id(), expected_new_parent_id);
    assert_ne!(
        new_parent_commit.change_id(),
        expected_old_commit.change_id()
    );
    new_parent_commit
}

pub fn assert_no_forgotten_test_files(test_dir: &Path) {
    // Parse the integration tests' main modules from the Cargo manifest.
    let manifest = {
        let file_path = test_dir.parent().unwrap().join("Cargo.toml");
        let text = fs::read_to_string(&file_path).unwrap();
        toml_edit::Document::parse(text).unwrap()
    };
    let test_bin_mods = if let Some(item) = manifest.get("test") {
        let tables = item.as_array_of_tables().unwrap();
        tables
            .iter()
            .map(|test| test.get("name").unwrap().as_str().unwrap().to_owned())
            .collect()
    } else {
        vec![]
    };

    // Add to that all submodules which are declared in the main test modules via
    // `mod`.
    let mut test_mods: HashSet<_> = test_bin_mods
        .iter()
        .flat_map(|test_mod| {
            let test_mod_path = test_dir.join(test_mod).with_extension("rs");
            let test_mod_contents = fs::read_to_string(&test_mod_path).unwrap();
            test_mod_contents
                .lines()
                .map(|line| line.trim_start_matches("pub "))
                .filter_map(|line| line.strip_prefix("mod"))
                .filter_map(|line| line.strip_suffix(";"))
                .map(|line| line.trim().to_string())
                .collect_vec()
        })
        .collect();
    test_mods.extend(test_bin_mods);

    // Gather list of Rust source files in test directory for comparison.
    let test_mod_files: HashSet<_> = fs::read_dir(test_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .filter_map(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_os_string().into_string().ok())
        })
        .collect();

    assert!(
        test_mod_files.is_subset(&test_mods),
        "the following test source files are not declared as integration tests nor included as \
         submodules of one: {}",
        test_mod_files
            .difference(&test_mods)
            .map(|mod_stem| format!("{mod_stem}.rs"))
            .join(", "),
    );
}

/// Returns true if the directory appears to be on a filesystem with strict
/// UTF-8 validation, as on ZFS with the `utf8only=on` property set.
#[cfg(unix)]
pub fn check_strict_utf8_fs(dir: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt as _;

    let test_file_normal = tempfile::Builder::new()
        .prefix(OsStr::from_bytes(b"strict-utf8-normal-"))
        .tempfile_in(dir);
    assert!(test_file_normal.is_ok());

    let test_file_invalid = tempfile::Builder::new()
        .prefix(OsStr::from_bytes(b"strict-utf8-\xe0-"))
        .tempfile_in(dir);
    test_file_invalid.is_err()
}
