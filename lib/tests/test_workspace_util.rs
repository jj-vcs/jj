use std::sync::Arc;

use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigSource;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::repo_path::RepoPathUiConverter;
use jj_lib::revset;
use jj_lib::revset::RevsetDiagnostics;
use jj_lib::revset::RevsetExtensions;
use jj_lib::settings::UserSettings;
use jj_lib::workspace_util::WorkspaceEnvironment;
use testutils::TestWorkspace;

// TODO: all of this is just a placeholder to exercise some of the
// WorkspaceEnvironment code independently of jj_cli

#[test]
fn test_create_environment() {
    let mut config = testutils::base_user_config();
    config.add_layer(
        ConfigLayer::parse(
            ConfigSource::User,
            r#"
                ui.revsets-use-glob-by-default = true
                ui.conflict-marker-style = "diff"
                revsets.log = "all()"
                [revset-aliases]
                'trunk()' = 'root()'
                'immutable_heads()' = 'trunk()'
            "#,
        )
        .unwrap(),
    );
    let settings = UserSettings::from_config(config).unwrap();

    let test_workspace = TestWorkspace::init_with_settings(&settings);
    let mut env = WorkspaceEnvironment::new(
        &test_workspace.workspace,
        test_workspace.workspace.workspace_root().to_owned(),
        Arc::new(RevsetExtensions::new()),
        |_| Ok(()),
    )
    .unwrap();

    // smoke test
    assert_eq!(env.workspace_name(), WorkspaceName::DEFAULT);
    assert!(matches!(
        env.path_converter(),
        RepoPathUiConverter::Fs { .. }
    ));

    // parse a revset
    let mut diagnostics = RevsetDiagnostics::new();
    let context = env.revset_parse_context();
    let _expression = revset::parse(&mut diagnostics, "@", &context).unwrap();
    assert!(diagnostics.is_empty());

    // reload expressions to exercise config loading
    let mut immutable_diag = RevsetDiagnostics::new();
    let mut prefixes_diag = RevsetDiagnostics::new();
    env.reload_revset_expressions(&mut immutable_diag, &mut prefixes_diag)
        .unwrap();

    // derive some contexts
    let _immutable = env.immutable_expression();
    let _id_prefix = env.new_id_prefix_context();
}
