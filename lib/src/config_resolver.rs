// Copyright 2024 The Jujutsu Authors
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

//! Post-processing functions for [`StackedConfig`].

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use itertools::Itertools as _;
use serde::de::IntoDeserializer as _;
use serde::Deserialize as _;
use toml_edit::DocumentMut;

use crate::config::ConfigGetError;
use crate::config::ConfigLayer;
use crate::config::ConfigNamePathBuf;
use crate::config::ConfigUpdateError;
use crate::config::ConfigValue;
use crate::config::StackedConfig;
use crate::config::ToConfigNamePath;

// Prefixed by "--" so these keys look unusual. It's also nice that "-" is
// placed earlier than the other keys in lexicographical order.
const SCOPE_CONDITION_KEY: &str = "--when";
const SCOPE_TABLE_KEY: &str = "--scope";

/// Parameters to enable scoped config tables conditionally.
#[derive(Clone, Debug)]
pub struct ConfigResolutionContext<'a> {
    /// Home directory. `~` will be substituted with this path.
    pub home_dir: Option<&'a Path>,
    /// Repository path, which is usually `<workspace_root>/.jj/repo`.
    pub repo_path: Option<&'a Path>,
}

/// Conditions to enable the parent table.
///
/// - Each predicate is tested separately, and the results are intersected.
/// - `None` means there are no constraints. (i.e. always `true`)
// TODO: introduce fileset-like DSL?
// TODO: add support for fileset-like pattern prefixes? it might be a bit tricky
// if path canonicalization is involved.
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct ScopeCondition {
    /// Paths to match the repository path prefix.
    pub repositories: Option<Vec<PathBuf>>,
    // TODO: maybe add "workspaces"?
}

impl ScopeCondition {
    fn from_value(
        value: ConfigValue,
        context: &ConfigResolutionContext,
    ) -> Result<Self, toml_edit::de::Error> {
        Self::deserialize(value.into_deserializer())?
            .expand_paths(context)
            .map_err(serde::de::Error::custom)
    }

    fn expand_paths(mut self, context: &ConfigResolutionContext) -> Result<Self, &'static str> {
        // It might make some sense to compare paths in canonicalized form, but
        // be careful to not resolve relative path patterns against cwd, which
        // wouldn't be what the user would expect.
        for path in self.repositories.as_mut().into_iter().flatten() {
            if let Some(new_path) = expand_home(path, context.home_dir)? {
                *path = new_path;
            }
        }
        Ok(self)
    }

    fn matches(&self, context: &ConfigResolutionContext) -> bool {
        matches_path_prefix(self.repositories.as_deref(), context.repo_path)
    }
}

fn expand_home(path: &Path, home_dir: Option<&Path>) -> Result<Option<PathBuf>, &'static str> {
    match path.strip_prefix("~") {
        Ok(tail) => {
            let home_dir = home_dir.ok_or("Cannot expand ~ (home directory is unknown)")?;
            Ok(Some(home_dir.join(tail)))
        }
        Err(_) => Ok(None),
    }
}

fn matches_path_prefix(candidates: Option<&[PathBuf]>, actual: Option<&Path>) -> bool {
    match (candidates, actual) {
        (Some(candidates), Some(actual)) => candidates.iter().any(|base| actual.starts_with(base)),
        (Some(_), None) => false, // actual path not known (e.g. not in workspace)
        (None, _) => true,        // no constraints
    }
}

/// Evaluates condition for each layer and scope, flattens scoped tables.
/// Returns new config that only contains enabled layers and tables.
pub fn resolve(
    source_config: &StackedConfig,
    context: &ConfigResolutionContext,
) -> Result<StackedConfig, ConfigGetError> {
    let mut source_layers_stack: Vec<Arc<ConfigLayer>> =
        source_config.layers().iter().rev().cloned().collect();
    let mut resolved_layers: Vec<Arc<ConfigLayer>> = Vec::new();
    while let Some(mut source_layer) = source_layers_stack.pop() {
        if !source_layer.data.contains_key(SCOPE_CONDITION_KEY)
            && !source_layer.data.contains_key(SCOPE_TABLE_KEY)
        {
            resolved_layers.push(source_layer); // reuse original table
            continue;
        }

        let layer_mut = Arc::make_mut(&mut source_layer);
        let condition = pop_scope_condition(layer_mut, context)?;
        if !condition.matches(context) {
            continue;
        }
        let tables = pop_scope_tables(layer_mut)?;
        // tables.iter() does not implement DoubleEndedIterator as of toml_edit
        // 0.22.22.
        let frame = source_layers_stack.len();
        for table in tables {
            let layer = ConfigLayer {
                source: source_layer.source,
                path: source_layer.path.clone(),
                data: DocumentMut::from(table),
            };
            source_layers_stack.push(Arc::new(layer));
        }
        source_layers_stack[frame..].reverse();
        resolved_layers.push(source_layer);
    }
    let mut resolved_config = StackedConfig::empty();
    resolved_config.extend_layers(resolved_layers);
    Ok(resolved_config)
}

fn pop_scope_condition(
    layer: &mut ConfigLayer,
    context: &ConfigResolutionContext,
) -> Result<ScopeCondition, ConfigGetError> {
    let Some(item) = layer.data.remove(SCOPE_CONDITION_KEY) else {
        return Ok(ScopeCondition::default());
    };
    let value = item
        .clone()
        .into_value()
        .expect("Item::None should not exist in table");
    ScopeCondition::from_value(value, context).map_err(|err| ConfigGetError::Type {
        name: SCOPE_CONDITION_KEY.to_owned(),
        error: err.into(),
        source_path: layer.path.clone(),
    })
}

fn pop_scope_tables(layer: &mut ConfigLayer) -> Result<toml_edit::ArrayOfTables, ConfigGetError> {
    let Some(item) = layer.data.remove(SCOPE_TABLE_KEY) else {
        return Ok(toml_edit::ArrayOfTables::new());
    };
    item.into_array_of_tables()
        .map_err(|item| ConfigGetError::Type {
            name: SCOPE_TABLE_KEY.to_owned(),
            error: format!("Expected an array of tables, but is {}", item.type_name()).into(),
            source_path: layer.path.clone(),
        })
}

/// Rule to migrate deprecated config variables.
pub struct ConfigMigrationRule {
    inner: MigrationRule,
}

enum MigrationRule {
    RenameValue {
        old_name: ConfigNamePathBuf,
        new_name: ConfigNamePathBuf,
    },
}

impl ConfigMigrationRule {
    /// Creates rule that moves value from `old_name` to `new_name`.
    pub fn rename_value(old_name: impl ToConfigNamePath, new_name: impl ToConfigNamePath) -> Self {
        let inner = MigrationRule::RenameValue {
            old_name: old_name.into_name_path().into(),
            new_name: new_name.into_name_path().into(),
        };
        ConfigMigrationRule { inner }
    }

    // TODO: rename + update value, generic Box<dyn Fn>, etc.

    /// Returns true if `layer` contains an item to be migrated.
    fn matches(&self, layer: &ConfigLayer) -> bool {
        match &self.inner {
            MigrationRule::RenameValue { old_name, .. } => {
                matches!(layer.look_up_item(old_name), Ok(Some(_)))
            }
        }
    }

    /// Migrates `layer` item. Returns a description of the applied migration.
    fn apply(&self, layer: &mut ConfigLayer) -> Result<String, ConfigUpdateError> {
        match &self.inner {
            MigrationRule::RenameValue { old_name, new_name } => {
                rename_value(layer, old_name, new_name)
            }
        }
    }
}

fn rename_value(
    layer: &mut ConfigLayer,
    old_name: &ConfigNamePathBuf,
    new_name: &ConfigNamePathBuf,
) -> Result<String, ConfigUpdateError> {
    let value = layer.delete_value(old_name)?.expect("tested by matches()");
    if matches!(layer.look_up_item(new_name), Ok(Some(_))) {
        return Ok(format!("{old_name} is deleted (superseded by {new_name})"));
    }
    layer.set_value(new_name, value)?;
    Ok(format!("{old_name} is renamed to {new_name}"))
}

/// Applies migration `rules` to `config`. Returns descriptions of the applied
/// migrations.
pub fn migrate(
    config: &mut StackedConfig,
    rules: &[ConfigMigrationRule],
) -> Result<Vec<String>, ConfigUpdateError> {
    let mut descriptions = Vec::new();
    for layer in config.layers_mut() {
        migrate_layer(layer, rules, &mut descriptions)?;
    }
    Ok(descriptions)
}

fn migrate_layer(
    layer: &mut Arc<ConfigLayer>,
    rules: &[ConfigMigrationRule],
    descriptions: &mut Vec<String>,
) -> Result<(), ConfigUpdateError> {
    let rules_to_apply = rules
        .iter()
        .filter(|rule| rule.matches(layer))
        .collect_vec();
    if rules_to_apply.is_empty() {
        return Ok(());
    }
    let layer_mut = Arc::make_mut(layer);
    for rule in rules_to_apply {
        let desc = rule.apply(layer_mut)?;
        descriptions.push(desc);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use indoc::indoc;

    use super::*;
    use crate::config::ConfigSource;

    #[test]
    fn test_expand_home() {
        let home_dir = Some(Path::new("/home/dir"));
        assert_eq!(
            expand_home("~".as_ref(), home_dir).unwrap(),
            Some(PathBuf::from("/home/dir"))
        );
        assert_eq!(expand_home("~foo".as_ref(), home_dir).unwrap(), None);
        assert_eq!(expand_home("/foo/~".as_ref(), home_dir).unwrap(), None);
        assert_eq!(
            expand_home("~/foo".as_ref(), home_dir).unwrap(),
            Some(PathBuf::from("/home/dir/foo"))
        );
        assert!(expand_home("~/foo".as_ref(), None).is_err());
    }

    #[test]
    fn test_condition_default() {
        let condition = ScopeCondition::default();

        let context = ConfigResolutionContext {
            home_dir: None,
            repo_path: None,
        };
        assert!(condition.matches(&context));
        let context = ConfigResolutionContext {
            home_dir: None,
            repo_path: Some(Path::new("/foo")),
        };
        assert!(condition.matches(&context));
    }

    #[test]
    fn test_condition_repo_path() {
        let condition = ScopeCondition {
            repositories: Some(["/foo", "/bar"].map(PathBuf::from).into()),
        };

        let context = ConfigResolutionContext {
            home_dir: None,
            repo_path: None,
        };
        assert!(!condition.matches(&context));
        let context = ConfigResolutionContext {
            home_dir: None,
            repo_path: Some(Path::new("/foo")),
        };
        assert!(condition.matches(&context));
        let context = ConfigResolutionContext {
            home_dir: None,
            repo_path: Some(Path::new("/fooo")),
        };
        assert!(!condition.matches(&context));
        let context = ConfigResolutionContext {
            home_dir: None,
            repo_path: Some(Path::new("/foo/baz")),
        };
        assert!(condition.matches(&context));
        let context = ConfigResolutionContext {
            home_dir: None,
            repo_path: Some(Path::new("/bar")),
        };
        assert!(condition.matches(&context));
    }

    #[test]
    fn test_condition_repo_path_windows() {
        let condition = ScopeCondition {
            repositories: Some(["c:/foo", r"d:\bar/baz"].map(PathBuf::from).into()),
        };

        let context = ConfigResolutionContext {
            home_dir: None,
            repo_path: Some(Path::new(r"c:\foo")),
        };
        assert_eq!(condition.matches(&context), cfg!(windows));
        let context = ConfigResolutionContext {
            home_dir: None,
            repo_path: Some(Path::new(r"c:\foo\baz")),
        };
        assert_eq!(condition.matches(&context), cfg!(windows));
        let context = ConfigResolutionContext {
            home_dir: None,
            repo_path: Some(Path::new(r"d:\foo")),
        };
        assert!(!condition.matches(&context));
        let context = ConfigResolutionContext {
            home_dir: None,
            repo_path: Some(Path::new(r"d:/bar\baz")),
        };
        assert_eq!(condition.matches(&context), cfg!(windows));
    }

    fn new_user_layer(text: &str) -> ConfigLayer {
        ConfigLayer::parse(ConfigSource::User, text).unwrap()
    }

    #[test]
    fn test_resolve_transparent() {
        let mut source_config = StackedConfig::empty();
        source_config.add_layer(ConfigLayer::empty(ConfigSource::Default));
        source_config.add_layer(ConfigLayer::empty(ConfigSource::User));

        let context = ConfigResolutionContext {
            home_dir: None,
            repo_path: None,
        };
        let resolved_config = resolve(&source_config, &context).unwrap();
        assert_eq!(resolved_config.layers().len(), 2);
        assert!(Arc::ptr_eq(
            &source_config.layers()[0],
            &resolved_config.layers()[0]
        ));
        assert!(Arc::ptr_eq(
            &source_config.layers()[1],
            &resolved_config.layers()[1]
        ));
    }

    #[test]
    fn test_resolve_table_order() {
        let mut source_config = StackedConfig::empty();
        source_config.add_layer(new_user_layer(indoc! {"
            a = 'a #0'
            [[--scope]]
            a = 'a #0.0'
            [[--scope]]
            a = 'a #0.1'
            [[--scope.--scope]]
            a = 'a #0.1.0'
            [[--scope]]
            a = 'a #0.2'
        "}));
        source_config.add_layer(new_user_layer(indoc! {"
            a = 'a #1'
            [[--scope]]
            a = 'a #1.0'
        "}));

        let context = ConfigResolutionContext {
            home_dir: None,
            repo_path: None,
        };
        let resolved_config = resolve(&source_config, &context).unwrap();
        assert_eq!(resolved_config.layers().len(), 7);
        insta::assert_snapshot!(resolved_config.layers()[0].data, @"a = 'a #0'");
        insta::assert_snapshot!(resolved_config.layers()[1].data, @"a = 'a #0.0'");
        insta::assert_snapshot!(resolved_config.layers()[2].data, @"a = 'a #0.1'");
        insta::assert_snapshot!(resolved_config.layers()[3].data, @"a = 'a #0.1.0'");
        insta::assert_snapshot!(resolved_config.layers()[4].data, @"a = 'a #0.2'");
        insta::assert_snapshot!(resolved_config.layers()[5].data, @"a = 'a #1'");
        insta::assert_snapshot!(resolved_config.layers()[6].data, @"a = 'a #1.0'");
    }

    #[test]
    fn test_resolve_repo_path() {
        let mut source_config = StackedConfig::empty();
        source_config.add_layer(new_user_layer(indoc! {"
            a = 'a #0'
            [[--scope]]
            --when.repositories = ['/foo']
            a = 'a #0.1 foo'
            [[--scope]]
            --when.repositories = ['/foo', '/bar']
            a = 'a #0.2 foo|bar'
            [[--scope]]
            --when.repositories = []
            a = 'a #0.3 none'
        "}));
        source_config.add_layer(new_user_layer(indoc! {"
            --when.repositories = ['~/baz']
            a = 'a #1 baz'
            [[--scope]]
            --when.repositories = ['/foo']  # should never be enabled
            a = 'a #1.1 baz&foo'
        "}));

        let context = ConfigResolutionContext {
            home_dir: Some(Path::new("/home/dir")),
            repo_path: None,
        };
        let resolved_config = resolve(&source_config, &context).unwrap();
        assert_eq!(resolved_config.layers().len(), 1);
        insta::assert_snapshot!(resolved_config.layers()[0].data, @"a = 'a #0'");

        let context = ConfigResolutionContext {
            home_dir: Some(Path::new("/home/dir")),
            repo_path: Some(Path::new("/foo/.jj/repo")),
        };
        let resolved_config = resolve(&source_config, &context).unwrap();
        assert_eq!(resolved_config.layers().len(), 3);
        insta::assert_snapshot!(resolved_config.layers()[0].data, @"a = 'a #0'");
        insta::assert_snapshot!(resolved_config.layers()[1].data, @"a = 'a #0.1 foo'");
        insta::assert_snapshot!(resolved_config.layers()[2].data, @"a = 'a #0.2 foo|bar'");

        let context = ConfigResolutionContext {
            home_dir: Some(Path::new("/home/dir")),
            repo_path: Some(Path::new("/bar/.jj/repo")),
        };
        let resolved_config = resolve(&source_config, &context).unwrap();
        assert_eq!(resolved_config.layers().len(), 2);
        insta::assert_snapshot!(resolved_config.layers()[0].data, @"a = 'a #0'");
        insta::assert_snapshot!(resolved_config.layers()[1].data, @"a = 'a #0.2 foo|bar'");

        let context = ConfigResolutionContext {
            home_dir: Some(Path::new("/home/dir")),
            repo_path: Some(Path::new("/home/dir/baz/.jj/repo")),
        };
        let resolved_config = resolve(&source_config, &context).unwrap();
        assert_eq!(resolved_config.layers().len(), 2);
        insta::assert_snapshot!(resolved_config.layers()[0].data, @"a = 'a #0'");
        insta::assert_snapshot!(resolved_config.layers()[1].data, @"a = 'a #1 baz'");
    }

    #[test]
    fn test_resolve_invalid_condition() {
        let new_config = |text: &str| {
            let mut config = StackedConfig::empty();
            config.add_layer(new_user_layer(text));
            config
        };
        let context = ConfigResolutionContext {
            home_dir: Some(Path::new("/home/dir")),
            repo_path: Some(Path::new("/foo/.jj/repo")),
        };
        assert_matches!(
            resolve(&new_config("--when.repositories = 0"), &context),
            Err(ConfigGetError::Type { .. })
        );
    }

    #[test]
    fn test_resolve_invalid_scoped_tables() {
        let new_config = |text: &str| {
            let mut config = StackedConfig::empty();
            config.add_layer(new_user_layer(text));
            config
        };
        let context = ConfigResolutionContext {
            home_dir: Some(Path::new("/home/dir")),
            repo_path: Some(Path::new("/foo/.jj/repo")),
        };
        assert_matches!(
            resolve(&new_config("[--scope]"), &context),
            Err(ConfigGetError::Type { .. })
        );
    }

    #[test]
    fn test_migrate_noop() {
        let mut config = StackedConfig::empty();
        config.add_layer(new_user_layer(indoc! {"
            foo = 'foo'
        "}));
        config.add_layer(new_user_layer(indoc! {"
            bar = 'bar'
        "}));

        let old_layers = config.layers().to_vec();
        let rules = [ConfigMigrationRule::rename_value("baz", "foo")];
        let descriptions = migrate(&mut config, &rules).unwrap();
        assert!(descriptions.is_empty());
        assert!(Arc::ptr_eq(&config.layers()[0], &old_layers[0]));
        assert!(Arc::ptr_eq(&config.layers()[1], &old_layers[1]));
    }

    #[test]
    fn test_migrate_rename_value() {
        let mut config = StackedConfig::empty();
        config.add_layer(new_user_layer(indoc! {"
            [foo]
            old = 'foo.old #0'
            [bar]
            old = 'bar.old #0'
            [baz]
            new = 'baz.new #0'
        "}));
        config.add_layer(new_user_layer(indoc! {"
            [bar]
            old = 'bar.old #1'
        "}));

        let rules = [
            ConfigMigrationRule::rename_value("foo.old", "foo.new"),
            ConfigMigrationRule::rename_value("bar.old", "baz.new"),
        ];
        let descriptions = migrate(&mut config, &rules).unwrap();
        insta::assert_debug_snapshot!(descriptions, @r#"
        [
            "foo.old is renamed to foo.new",
            "bar.old is deleted (superseded by baz.new)",
            "bar.old is renamed to baz.new",
        ]
        "#);
        insta::assert_snapshot!(config.layers()[0].data, @r"
        [foo]
        new = 'foo.old #0'
        [bar]
        [baz]
        new = 'baz.new #0'
        ");
        insta::assert_snapshot!(config.layers()[1].data, @r"
        [bar]

        [baz]
        new = 'bar.old #1'
        ");
    }
}
