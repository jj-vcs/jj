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

use std::assert_matches;
use std::cell::RefCell;
use std::fs;
use std::slice;
use std::sync::Arc;

use futures::TryStreamExt as _;
use jj_frontend::config_util;
use jj_frontend::operation_util::UndoError;
use jj_frontend::operation_util::ViewPortions;
use jj_frontend::operation_util::find_redo_target;
use jj_frontend::operation_util::find_undo_target;
use jj_frontend::operation_util::restore_operation;
use jj_frontend::revset_util::RevsetEvaluationSizeError;
use jj_frontend::revset_util::UserRevsetEvaluationError;
use jj_frontend::runner::RunnerError;
use jj_frontend::runner::RunnerEvent;
use jj_frontend::runner::RunnerEvents;
use jj_frontend::runner::RunnerOptions;
use jj_frontend::runner::SnapshotConfig;
use jj_frontend::runner::SnapshotConfigDiagnostics;
use jj_frontend::runner::WorkspaceOperationRunner;
use jj_frontend::workspace_env::WorkspaceEnvironment;
use jj_frontend::workspace_env::WorkspaceEnvironmentDiagnostics;
use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigSource;
use jj_lib::op_walk;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPath;
use jj_lib::revset::RevsetDiagnostics;
use jj_lib::revset::RevsetExtensions;
use pollster::FutureExt as _;
use testutils::TestEnvironment;
use testutils::TestWorkspace;

/// Records the names of reported events, like a GUI might collect them for
/// display.
#[derive(Default)]
struct RecordingEvents(RefCell<Vec<String>>);

impl RunnerEvents for RecordingEvents {
    fn event(&self, event: RunnerEvent) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let name = format!("{event:?}");
        let name = name.split([' ', '(', '{']).next().unwrap().to_owned();
        self.0.borrow_mut().push(name);
        Ok(())
    }
}

impl RecordingEvents {
    fn take(&self) -> Vec<String> {
        self.0.take()
    }
}

/// Loads the workspace like a GUI would, using only the default config from
/// `jj-lib` and `jj-frontend` plus `user_config`. Returns the test environment
/// (which must be kept alive), the runner, and the snapshot config.
fn load_runner(
    events: &RecordingEvents,
    user_config: &str,
    options: RunnerOptions,
) -> (TestEnvironment, WorkspaceOperationRunner, SnapshotConfig) {
    let mut config = testutils::base_user_config();
    config.extend_layers(config_util::default_config_layers());
    config.add_layer(ConfigLayer::parse(ConfigSource::User, user_config).unwrap());
    let settings = testutils::user_settings_from_config(config);
    let TestWorkspace {
        env: test_env,
        workspace,
        ..
    } = TestWorkspace::init_with_settings(&settings);
    let workspace_root = workspace.workspace_root().to_owned();

    let mut warnings = vec![];
    let fileset_aliases =
        config_util::load_fileset_aliases(&mut warnings, settings.config()).unwrap();
    let revset_aliases =
        config_util::load_revset_aliases(&mut warnings, settings.config()).unwrap();
    assert!(warnings.is_empty());
    let env = WorkspaceEnvironment::new(
        &mut WorkspaceEnvironmentDiagnostics::new(),
        &workspace,
        &workspace_root,
        Arc::new(RevsetExtensions::default()),
        fileset_aliases,
        revset_aliases,
    )
    .unwrap();
    let snapshot_config = SnapshotConfig::load(
        &mut SnapshotConfigDiagnostics::new(),
        &env,
        workspace.repo_loader().store(),
        None,
    )
    .unwrap();
    let runner = WorkspaceOperationRunner::load_at_head(events, workspace, Arc::new(env), options)
        .block_on()
        .unwrap();
    (test_env, runner, snapshot_config)
}

fn test_options() -> RunnerOptions {
    RunnerOptions {
        ignore_working_copy: false,
        integrate_operation: true,
        ignore_immutable: false,
        merge_concurrent_operations: false,
        transaction_attributes: vec![("client".to_owned(), "test".to_owned())],
    }
}

#[test]
fn test_snapshot_and_finish_transaction_without_cli() {
    let events = RecordingEvents::default();
    let (_test_env, mut runner, snapshot_config) = load_runner(&events, "", test_options());
    let workspace_root = runner.workspace_root().to_owned();
    assert!(events.take().is_empty());

    // Snapshot a new file
    fs::write(workspace_root.join("file"), "contents").unwrap();
    let lock = runner.lock_git_import_export().unwrap();
    let stats = runner
        .snapshot(&events, &snapshot_config, &lock)
        .block_on()
        .unwrap();
    drop(lock);
    assert!(stats.untracked_paths.is_empty());
    let snapshot_commit = runner.get_wc_commit().block_on().unwrap().unwrap();
    let file_path = RepoPath::from_internal_string("file").unwrap();
    assert!(
        snapshot_commit
            .tree()
            .path_value(file_path)
            .block_on()
            .unwrap()
            .is_present()
    );
    assert_eq!(
        runner
            .repo()
            .operation()
            .metadata()
            .attributes
            .get("client"),
        Some(&"test".to_owned())
    );
    assert!(events.take().is_empty());

    // Create a new working-copy commit on top
    let mut tx = runner.start_transaction();
    let new_wc_commit = tx
        .repo_mut()
        .check_out(runner.workspace_name().to_owned(), &snapshot_commit)
        .block_on()
        .unwrap();
    runner
        .finish_transaction(&events, tx, "new commit")
        .block_on()
        .unwrap();
    assert_eq!(events.take(), ["WorkingCopyUpdated", "TransactionFinished"]);
    assert_eq!(runner.get_wc_commit_id(), Some(new_wc_commit.id()));
    assert_eq!(new_wc_commit.parent_ids(), [snapshot_commit.id().clone()]);
    assert!(workspace_root.join("file").is_file());
}

#[test]
fn test_check_rewritable() {
    let events = RecordingEvents::default();
    let (_test_env, runner, _) = load_runner(&events, "", test_options());
    let root_commit_id = runner.repo().store().root_commit_id().clone();
    let wc_commit_id = runner.get_wc_commit_id().unwrap().clone();

    // With the default config, only the root commit is immutable since there's
    // no trunk() bookmark.
    assert_matches!(
        runner.check_rewritable([&root_commit_id]).block_on(),
        Err(RunnerError::RootCommitImmutable { commit_id }) if commit_id == root_commit_id
    );
    runner.check_rewritable([&wc_commit_id]).block_on().unwrap();

    let user_config = r#"revset-aliases.'immutable_heads()' = '@'"#;
    let (_test_env, runner, _) = load_runner(&events, user_config, test_options());
    let wc_commit_id = runner.get_wc_commit_id().unwrap().clone();
    assert_matches!(
        runner.check_rewritable([&wc_commit_id]).block_on(),
        Err(RunnerError::ImmutableCommit { commit_id, num_immutable: (1, _) })
            if commit_id == wc_commit_id
    );

    // Immutable commits can be rewritten with `ignore_immutable`, but the root
    // commit can't.
    let options = RunnerOptions {
        ignore_immutable: true,
        ..test_options()
    };
    let (_test_env, runner, _) = load_runner(&events, user_config, options);
    let wc_commit_id = runner.get_wc_commit_id().unwrap().clone();
    runner.check_rewritable([&wc_commit_id]).block_on().unwrap();
    assert_matches!(
        runner.check_rewritable([&root_commit_id]).block_on(),
        Err(RunnerError::RootCommitImmutable { .. })
    );
}

#[test]
fn test_resolve_revisions() {
    let events = RecordingEvents::default();
    let (_test_env, runner, _) = load_runner(&events, "", test_options());
    let env = runner.env();
    let repo = runner.repo().as_ref();
    let id_prefix_context = env.new_id_prefix_context();
    let wc_commit = runner.get_wc_commit().block_on().unwrap().unwrap();
    let resolve = |revset_str: &str| {
        let mut diagnostics = RevsetDiagnostics::new();
        let expression = env.parse_revset(&mut diagnostics, revset_str).unwrap();
        assert!(diagnostics.is_empty());
        env.revset_evaluator(repo, &id_prefix_context, expression)
            .evaluate_to_single_commit()
            .block_on()
    };

    assert_eq!(resolve("@").unwrap().id(), wc_commit.id());
    let change_id_prefix = &wc_commit.change_id().reverse_hex()[..12];
    assert_eq!(resolve(change_id_prefix).unwrap().id(), wc_commit.id());
    assert_matches!(resolve("none()"), Err(RevsetEvaluationSizeError::Empty));
    assert_matches!(
        resolve("root() | @"),
        Err(RevsetEvaluationSizeError::Multiple { commits, has_more: false })
            if commits.len() == 2
    );
    assert_matches!(
        resolve("nonexistent"),
        Err(RevsetEvaluationSizeError::Other(
            UserRevsetEvaluationError::Resolution(_)
        ))
    );
    assert!(
        env.parse_revset(&mut RevsetDiagnostics::new(), "@(")
            .is_err()
    );
}

#[test]
fn test_undo_redo_and_op_log() {
    let events = RecordingEvents::default();
    let (_test_env, mut runner, _) = load_runner(&events, "", test_options());
    let old_wc_commit = runner.get_wc_commit().block_on().unwrap().unwrap();
    let workspace_name = runner.workspace_name().to_owned();

    // Nothing to redo after a regular operation
    assert_matches!(
        find_redo_target(runner.repo(), &workspace_name, false).block_on(),
        Err(UndoError::NothingToRedo)
    );

    // Check out a new commit
    let mut tx = runner.start_transaction();
    let new_wc_commit = tx
        .repo_mut()
        .check_out(workspace_name.clone(), &old_wc_commit)
        .block_on()
        .unwrap();
    runner
        .finish_transaction(&events, tx, "new commit")
        .block_on()
        .unwrap();
    let new_commit_op = runner.repo().operation().clone();
    assert_eq!(runner.get_wc_commit_id(), Some(new_wc_commit.id()));

    // Undo it
    let target = find_undo_target(runner.repo(), &workspace_name, false)
        .block_on()
        .unwrap();
    assert_eq!(target.operation.id(), new_commit_op.id());
    let mut tx = runner.start_transaction();
    restore_operation(tx.repo_mut(), &target.restore_to, ViewPortions::ALL)
        .block_on()
        .unwrap();
    runner
        .finish_transaction(&events, tx, target.description())
        .block_on()
        .unwrap();
    assert_eq!(runner.get_wc_commit_id(), Some(old_wc_commit.id()));

    // Redo it
    let target = find_redo_target(runner.repo(), &workspace_name, false)
        .block_on()
        .unwrap();
    assert_eq!(target.restore_to.id(), new_commit_op.id());
    let mut tx = runner.start_transaction();
    restore_operation(tx.repo_mut(), &target.restore_to, ViewPortions::ALL)
        .block_on()
        .unwrap();
    runner
        .finish_transaction(&events, tx, target.description())
        .block_on()
        .unwrap();
    assert_eq!(runner.get_wc_commit_id(), Some(new_wc_commit.id()));

    // The operation log can be walked from the current operation
    let descriptions: Vec<_> = op_walk::walk_ancestors(slice::from_ref(runner.repo().operation()))
        .map_ok(|op| op.metadata().description.clone())
        .try_collect()
        .block_on()
        .unwrap();
    assert_eq!(descriptions.len(), 5);
    assert!(descriptions[0].starts_with("redo: restore to operation "));
    assert!(descriptions[1].starts_with("undo: restore to operation "));
    assert_eq!(descriptions[2], "new commit");
}
