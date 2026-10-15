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
use std::sync::Arc;

use jj_frontend::config_util;
use jj_frontend::workspace_env::WorkspaceEnvironment;
use jj_frontend::workspace_env::WorkspaceEnvironmentDiagnostics;
use jj_frontend::workspace_env::WorkspaceEnvironmentError;
use jj_lib::revset::RevsetExtensions;
use testutils::TestWorkspace;

#[test]
fn test_load_without_default_config() {
    // Only jj-lib's defaults, so `immutable_heads()` isn't defined
    let settings = testutils::user_settings();
    let test_workspace = TestWorkspace::init_with_settings(&settings);
    let workspace = &test_workspace.workspace;
    let mut warnings = vec![];
    let fileset_aliases =
        config_util::load_fileset_aliases(&mut warnings, settings.config()).unwrap();
    let revset_aliases =
        config_util::load_revset_aliases(&mut warnings, settings.config()).unwrap();
    let result = WorkspaceEnvironment::new(
        &mut WorkspaceEnvironmentDiagnostics::new(),
        workspace,
        workspace.workspace_root(),
        Arc::new(RevsetExtensions::default()),
        fileset_aliases,
        revset_aliases,
    );
    assert_matches!(
        result.err(),
        Some(WorkspaceEnvironmentError::ImmutableHeadsNotDefined)
    );
}
