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

use std::fs;
use std::sync::Arc;

use jj_frontend::config_env::ConfigEnv;
use jj_frontend::config_env::ConfigEnvInputs;
use jj_frontend::config_env::config_from_environment;
use jj_frontend::config_util;
use jj_frontend::workspace_env::WorkspaceEnvironment;
use jj_frontend::workspace_env::WorkspaceEnvironmentDiagnostics;
use jj_frontend::workspace_env::find_workspace_dir;
use jj_lib::default_backend_factories::default_backend_factories;
use jj_lib::default_backend_factories::default_working_copy_factories;
use jj_lib::revset::RevsetExtensions;
use jj_lib::settings::UserSettings;
use jj_lib::workspace::DefaultWorkspaceLoaderFactory;
use jj_lib::workspace::WorkspaceLoaderFactory as _;
use testutils::TestRepoBackend;
use testutils::TestWorkspace;

/// Loads the user, repo, and workspace config for a workspace like a GUI
/// would, and then loads the workspace with it.
#[test]
fn test_load_workspace_with_config() {
    // The test backend can't be loaded with the default store factories
    let test_workspace = TestWorkspace::init_with_backend(TestRepoBackend::Simple);
    let workspace_root = test_workspace.workspace.workspace_root();
    let subdir = workspace_root.join("dir");
    fs::create_dir(&subdir).unwrap();

    let config_dir = testutils::new_temp_dir();
    fs::create_dir(config_dir.path().join("jj")).unwrap();
    fs::write(
        config_dir.path().join("jj").join("config.toml"),
        format!(
            r#"
            test.user = "user"
            test.overridden = "user"

            [[--scope]]
            --when.workspaces = ['{}']
            test.scoped = "workspace"
            "#,
            workspace_root.display()
        ),
    )
    .unwrap();
    let mut config_env = ConfigEnv::new(ConfigEnvInputs {
        user_config_dir: Some(config_dir.path().to_owned()),
        randomness_seed: Some(0),
        ..Default::default()
    });
    let mut raw_config = config_from_environment(config_util::default_config_layers());
    config_env.reload_system_config(&mut raw_config).unwrap();
    config_env.reload_user_config(&mut raw_config).unwrap();

    // Find the workspace from a subdirectory
    let workspace_dir = find_workspace_dir(&subdir);
    assert_eq!(workspace_dir, workspace_root);
    let loader = DefaultWorkspaceLoaderFactory.create(workspace_dir).unwrap();

    // Write a repo config file where the config env expects it
    let mut warnings = vec![];
    config_env.reset_repo_path(loader.repo_path());
    let repo_config_path = config_env.repo_config_path(&mut warnings).unwrap().unwrap();
    assert!(repo_config_path.starts_with(config_dir.path()));
    fs::write(
        &repo_config_path,
        r#"
        test.overridden = "repo"
        revset-aliases.'test_alias()' = 'root()'
        "#,
    )
    .unwrap();

    config_env
        .reset_workspace_and_reload_config(
            &mut warnings,
            &mut raw_config,
            loader.repo_path(),
            loader.workspace_root(),
        )
        .unwrap();
    assert!(warnings.is_empty());
    let config = config_env.resolve_config(&raw_config).unwrap();
    assert_eq!(config.get::<String>("test.user").unwrap(), "user");
    assert_eq!(config.get::<String>("test.overridden").unwrap(), "repo");
    assert_eq!(config.get::<String>("test.scoped").unwrap(), "workspace");

    let settings = UserSettings::from_config_and_home_dir(
        config,
        config_env.home_dir().map(ToOwned::to_owned),
    )
    .unwrap();
    let workspace = loader
        .load(
            &settings,
            &default_backend_factories(),
            &default_working_copy_factories(),
        )
        .unwrap();
    let mut config_warnings = vec![];
    let fileset_aliases =
        config_util::load_fileset_aliases(&mut config_warnings, settings.config()).unwrap();
    let revset_aliases =
        config_util::load_revset_aliases(&mut config_warnings, settings.config()).unwrap();
    assert!(config_warnings.is_empty());
    assert!(
        revset_aliases
            .function_names()
            .any(|name| name == "test_alias")
    );
    WorkspaceEnvironment::new(
        &mut WorkspaceEnvironmentDiagnostics::new(),
        &workspace,
        &subdir,
        Arc::new(RevsetExtensions::default()),
        fileset_aliases,
        revset_aliases,
    )
    .unwrap();
}
