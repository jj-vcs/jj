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

use std::cell::RefCell;
use std::fs;
use std::sync::Arc;

use jj_frontend::config_util;
use jj_frontend::runner::RunnerEvent;
use jj_frontend::runner::RunnerEvents;
use jj_frontend::runner::RunnerOptions;
use jj_frontend::runner::SnapshotConfig;
use jj_frontend::runner::SnapshotConfigDiagnostics;
use jj_frontend::runner::WorkspaceOperationRunner;
use jj_frontend::workspace_env::WorkspaceEnvironment;
use jj_frontend::workspace_env::WorkspaceEnvironmentDiagnostics;
use jj_lib::repo_path::RepoPath;
use jj_lib::revset::RevsetExtensions;
use pollster::FutureExt as _;
use testutils::TestWorkspace;

/// Records the names of reported events, like a GUI might collect them for
/// display.
#[derive(Default)]
struct RecordingEvents(RefCell<Vec<String>>);

impl RunnerEvents for RecordingEvents {
    fn event(
        &self,
        event: RunnerEvent<'_>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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

#[test]
fn test_snapshot_and_finish_transaction_without_cli() {
    let mut config = testutils::base_user_config();
    config.extend_layers(config_util::default_config_layers());
    let settings = testutils::user_settings_from_config(config);
    let TestWorkspace {
        env: _test_env,
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
    let options = RunnerOptions {
        ignore_working_copy: false,
        integrate_operation: true,
        ignore_immutable: false,
        merge_concurrent_operations: false,
        transaction_attributes: vec![("client".to_owned(), "test".to_owned())],
    };

    let events = RecordingEvents::default();
    let mut runner =
        WorkspaceOperationRunner::load_at_head(&events, workspace, Arc::new(env), options)
            .block_on()
            .unwrap();
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
