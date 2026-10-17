// Copyright 2022 The Jujutsu Authors
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

use std::collections::BTreeSet;
use std::fmt;
use std::io;
use std::path::PathBuf;

use itertools::Itertools as _;
use jj_frontend::config_util;
use jj_frontend::config_util::ConfigWarning;
use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigMigrationRule;
use jj_lib::config::ConfigNamePathBuf;
use jj_lib::config::ConfigSource;
use jj_lib::config::ConfigValue;
use jj_lib::config::StackedConfig;
use jj_lib::dsl_util::AliasDeclarationParser;
use jj_lib::dsl_util::AliasesMap;
use serde::Serialize as _;

use crate::command_error::CommandError;
use crate::command_error::config_error;
use crate::command_error::config_error_with_message;
use crate::ui::Ui;

// TODO(#879): Consider generating entire schema dynamically vs. static file.
pub const CONFIG_SCHEMA: &str = include_str!("config-schema.json");

/// Parses a TOML value expression. Interprets the given value as string if it
/// can't be parsed and doesn't look like a TOML expression.
pub fn parse_value_or_bare_string(value_str: &str) -> Result<ConfigValue, toml_edit::TomlError> {
    match value_str.parse() {
        Ok(value) => Ok(value),
        Err(_) if is_bare_string(value_str) => Ok(value_str.into()),
        Err(err) => Err(err),
    }
}

fn is_bare_string(value_str: &str) -> bool {
    // leading whitespace isn't ignored when parsing TOML value expression, but
    // "\n[]" doesn't look like a bare string.
    let trimmed = value_str.trim_ascii().as_bytes();
    if let (Some(&first), Some(&last)) = (trimmed.first(), trimmed.last()) {
        // string, array, or table constructs?
        !matches!(first, b'"' | b'\'' | b'[' | b'{') && !matches!(last, b'"' | b'\'' | b']' | b'}')
    } else {
        true // empty or whitespace only
    }
}

/// Converts [`ConfigValue`] (or [`toml_edit::Value`]) to [`toml::Value`] which
/// implements [`serde::Serialize`].
pub fn to_serializable_value(value: ConfigValue) -> toml::Value {
    match value {
        ConfigValue::String(v) => toml::Value::String(v.into_value()),
        ConfigValue::Integer(v) => toml::Value::Integer(v.into_value()),
        ConfigValue::Float(v) => toml::Value::Float(v.into_value()),
        ConfigValue::Boolean(v) => toml::Value::Boolean(v.into_value()),
        ConfigValue::Datetime(v) => toml::Value::Datetime(v.into_value()),
        ConfigValue::Array(array) => {
            let array = array.into_iter().map(to_serializable_value).collect();
            toml::Value::Array(array)
        }
        ConfigValue::InlineTable(table) => {
            let table = table
                .into_iter()
                .map(|(k, v)| (k, to_serializable_value(v)))
                .collect();
            toml::Value::Table(table)
        }
    }
}

/// Configuration variable with its source information.
#[derive(Clone, Debug, serde::Serialize)]
pub struct AnnotatedValue {
    /// Dotted name path to the configuration variable.
    #[serde(serialize_with = "serialize_name")]
    pub name: ConfigNamePathBuf,
    /// Configuration value.
    #[serde(serialize_with = "serialize_value")]
    pub value: ConfigValue,
    /// Source of the configuration value.
    #[serde(serialize_with = "serialize_source")]
    pub source: ConfigSource,
    /// Path to the source file, if available.
    pub path: Option<PathBuf>,
    /// True if this value is overridden in higher precedence layers.
    pub is_overridden: bool,
}

fn serialize_name<S>(name: &ConfigNamePathBuf, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    name.to_string().serialize(serializer)
}

fn serialize_value<S>(value: &ConfigValue, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    to_serializable_value(value.clone()).serialize(serializer)
}

fn serialize_source<S>(source: &ConfigSource, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    source.to_string().serialize(serializer)
}

/// Collects values under the given `filter_prefix` name recursively, from all
/// layers.
///
/// This does not exclude disabled `aliases` values.
pub fn resolved_config_values(
    stacked_config: &StackedConfig,
    filter_prefix: &ConfigNamePathBuf,
) -> Vec<AnnotatedValue> {
    // Collect annotated values in reverse order and mark each value shadowed by
    // value or table in upper layers.
    let mut config_vals = vec![];
    let mut upper_value_names = BTreeSet::new();
    for layer in stacked_config.layers().iter().rev() {
        let top_item = match layer.look_up_item(filter_prefix) {
            Ok(Some(item)) => item,
            Ok(None) => continue, // parent is a table, but no value found
            Err(_) => {
                // parent is not a table, shadows lower layers
                upper_value_names.insert(filter_prefix.clone());
                continue;
            }
        };
        let mut config_stack = vec![(filter_prefix.clone(), top_item, false)];
        while let Some((name, item, is_parent_overridden)) = config_stack.pop() {
            // Cannot retain inline table formatting because inner values may be
            // overridden independently.
            if let Some(table) = item.as_table_like() {
                // current table and children may be shadowed by value in upper layer
                let is_overridden = is_parent_overridden || upper_value_names.contains(&name);
                for (k, v) in table.iter() {
                    let mut sub_name = name.clone();
                    sub_name.push(k);
                    config_stack.push((sub_name, v, is_overridden)); // in reverse order
                }
            } else {
                // current value may be shadowed by value or table in upper layer
                let maybe_child = upper_value_names
                    .range(&name..)
                    .next()
                    .filter(|next| next.starts_with(&name));
                let is_overridden = is_parent_overridden || maybe_child.is_some();
                if maybe_child != Some(&name) {
                    upper_value_names.insert(name.clone());
                }
                let value = item
                    .clone()
                    .into_value()
                    .expect("Item::None should not exist in table");
                config_vals.push(AnnotatedValue {
                    name,
                    value,
                    source: layer.source,
                    path: layer.path.clone(),
                    is_overridden,
                });
            }
        }
    }
    config_vals.reverse();
    config_vals
}

pub fn default_config_layers() -> Vec<ConfigLayer> {
    // Syntax error in default config isn't a user error. That's why defaults are
    // loaded by separate builder.
    let parse = |text: &'static str| ConfigLayer::parse(ConfigSource::Default, text).unwrap();
    let mut layers = config_util::default_config_layers();
    layers.extend([
        parse(include_str!("config/colors.toml")),
        parse(include_str!("config/hints.toml")),
        parse(include_str!("config/misc.toml")),
        parse(include_str!("config/revsets.toml")),
        parse(include_str!("config/templates.toml")),
    ]);
    if cfg!(windows) {
        layers.push(parse(include_str!("config/windows.toml")));
    }
    layers
}

/// Configuration source/data type provided as command-line argument.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigArgKind {
    /// `--config=NAME=VALUE`
    Item,
    /// `--config-file=PATH`
    File,
}

/// Parses `--config*` arguments.
pub fn parse_config_args(
    toml_strs: &[(ConfigArgKind, &str)],
) -> Result<Vec<ConfigLayer>, CommandError> {
    let source = ConfigSource::CommandArg;
    let mut layers = Vec::new();
    for (kind, chunk) in &toml_strs.iter().chunk_by(|&(kind, _)| kind) {
        match kind {
            ConfigArgKind::Item => {
                let mut layer = ConfigLayer::empty(source);
                for (_, item) in chunk {
                    let (name, value) = parse_config_arg_item(item)?;
                    // Can fail depending on the argument order, but that
                    // wouldn't matter in practice.
                    layer.set_value(name, value).map_err(|err| {
                        config_error_with_message("--config argument cannot be set", err)
                    })?;
                }
                layers.push(layer);
            }
            ConfigArgKind::File => {
                for (_, path) in chunk {
                    layers.push(ConfigLayer::load_from_file(source, path.into())?);
                }
            }
        }
    }
    Ok(layers)
}

/// Parses `NAME=VALUE` string.
fn parse_config_arg_item(item_str: &str) -> Result<(ConfigNamePathBuf, ConfigValue), CommandError> {
    // split NAME=VALUE at the first parsable position
    let split_candidates = item_str.as_bytes().iter().positions(|&b| b == b'=');
    let Some((name, value_str)) = split_candidates
        .map(|p| (&item_str[..p], &item_str[p + 1..]))
        .map(|(name, value)| name.parse().map(|name| (name, value)))
        .find_or_last(Result::is_ok)
        .transpose()
        .map_err(|err| config_error_with_message("--config name cannot be parsed", err))?
    else {
        return Err(config_error("--config must be specified as NAME=VALUE"));
    };
    let value = parse_value_or_bare_string(value_str)
        .map_err(|err| config_error_with_message("--config value cannot be parsed", err))?;
    Ok((name, value))
}

/// List of rules to migrate deprecated config variables.
pub fn default_config_migrations() -> Vec<ConfigMigrationRule> {
    vec![
        // TODO: Remove in jj 0.51+
        ConfigMigrationRule::custom(
            |layer| {
                let Ok(Some(tools_table)) = layer.look_up_table("fix.tools") else {
                    return false;
                };
                tools_table.iter().any(|(_, item)| {
                    item.as_table_like()
                        .is_some_and(|tool_config| tool_config.contains_key("line-range-arg"))
                })
            },
            |layer| {
                let bad_tool_names = if let Ok(Some(tools_table)) = layer.look_up_table("fix.tools")
                {
                    tools_table
                        .iter()
                        .filter_map(|(tool_name, item)| {
                            if item.as_table_like()?.contains_key("line-range-arg") {
                                Some(tool_name.to_string())
                            } else {
                                None
                            }
                        })
                        .collect_vec()
                } else {
                    vec![]
                };
                if bad_tool_names.is_empty() {
                    // Somehow no tools config needed to be migrated. Should
                    // have been caught by `matches()`, but just return a
                    // generic message.
                    return Ok("`fix.tools.<name>.line-range-arg` is deprecated; use \
                               `line-range-args` instead."
                        .into());
                }
                let mut messages = vec![];
                for tool_name in bad_tool_names {
                    let old_name = &ConfigNamePathBuf::from_iter([
                        "fix",
                        "tools",
                        &tool_name,
                        "line-range-arg",
                    ]);
                    let new_name = &ConfigNamePathBuf::from_iter([
                        "fix",
                        "tools",
                        &tool_name,
                        "line-range-args",
                    ]);

                    let Ok(Some(old_value)) = layer.delete_value(old_name) else {
                        // Failed to delete the value. Ignore it; it'll be tried
                        // again later.
                        continue;
                    };
                    if matches!(layer.look_up_item(new_name), Ok(Some(_))) {
                        messages.push(format!("{old_name} is deleted (superseded by {new_name})"));
                        continue;
                    }
                    let Some(old_value_str) = old_value.as_str() else {
                        return Err(jj_lib::config::ConfigMigrateLayerError::Type {
                            name: old_name.to_string(),
                            error: format!(
                                "Failed to migrate {old_name}, which was expected to be a string"
                            )
                            .into(),
                        });
                    };
                    layer.set_value(new_name, ConfigValue::from_iter([old_value_str]))?;
                    messages.push(format!(
                        "{old_name} is updated to {new_name} = [{}]",
                        old_value.decorated("", "")
                    ));
                }
                Ok(messages.join(", "))
            },
        ),
    ]
}

/// Loads map of `{fileset,revset,template}-aliases` table.
///
/// This function does not handle loading the `aliases` table, which supports
/// disabled aliases.
pub fn load_aliases_map<P>(
    ui: &Ui,
    config: &StackedConfig,
    table_name: &ConfigNamePathBuf,
) -> Result<AliasesMap<P, String>, CommandError>
where
    P: AliasDeclarationParser + Default,
    P::Error: fmt::Display,
{
    let mut warnings = vec![];
    let result = config_util::load_aliases_map(&mut warnings, config, table_name);
    print_config_warnings(ui, &warnings)?;
    Ok(result?)
}

/// Prints warnings found while loading configuration.
pub fn print_config_warnings(ui: &Ui, warnings: &[ConfigWarning]) -> io::Result<()> {
    for warning in warnings {
        writeln!(ui.warning_default(), "{warning}")?;
    }
    Ok(())
}

/// Calls `f` with a list for collecting configuration warnings, and prints the
/// collected warnings before returning the result of `f`.
pub fn with_config_warnings<T, E>(
    ui: &Ui,
    f: impl FnOnce(&mut Vec<ConfigWarning>) -> Result<T, E>,
) -> Result<T, CommandError>
where
    CommandError: From<E>,
{
    let mut warnings = vec![];
    let result = f(&mut warnings);
    print_config_warnings(ui, &warnings)?;
    Ok(result?)
}

#[cfg(test)]
mod tests {

    use std::fmt::Write as _;

    use testutils::TestResult;

    use super::*;

    fn insta_settings() -> insta::Settings {
        let mut settings = insta::Settings::clone_current();
        // Suppress Decor { .. } which is uninteresting
        settings.add_filter(r"\bDecor \{[^}]*\}", "Decor { .. }");
        settings
    }

    #[test]
    fn test_parse_value_or_bare_string() -> TestResult {
        let parse = |s: &str| parse_value_or_bare_string(s);

        // Value in TOML syntax
        assert_eq!(parse("true")?.as_bool(), Some(true));
        assert_eq!(parse("42")?.as_integer(), Some(42));
        assert_eq!(parse("-1")?.as_integer(), Some(-1));
        assert_eq!(parse("'a'")?.as_str(), Some("a"));
        assert!(parse("[]")?.is_array());
        assert!(parse("{ a = 'b' }")?.is_inline_table());

        // Bare string
        assert_eq!(parse("")?.as_str(), Some(""));
        assert_eq!(parse("John Doe")?.as_str(), Some("John Doe"));
        assert_eq!(parse("Doe, John")?.as_str(), Some("Doe, John"));
        assert_eq!(parse("It's okay")?.as_str(), Some("It's okay"));
        assert_eq!(
            parse("<foo+bar@example.org>")?.as_str(),
            Some("<foo+bar@example.org>")
        );
        assert_eq!(parse("#ff00aa")?.as_str(), Some("#ff00aa"));
        assert_eq!(parse("all()")?.as_str(), Some("all()"));
        assert_eq!(parse("glob:*.*")?.as_str(), Some("glob:*.*"));
        assert_eq!(parse("柔術")?.as_str(), Some("柔術"));

        // Error in TOML value
        assert!(parse("'foo").is_err());
        assert!(parse(r#" bar" "#).is_err());
        assert!(parse("[0 1]").is_err());
        assert!(parse("{ x = y }").is_err());
        assert!(parse("\n { x").is_err());
        assert!(parse(" x ] ").is_err());
        assert!(parse("[table]\nkey = 'value'").is_err());
        Ok(())
    }

    #[test]
    fn test_parse_config_arg_item() {
        assert!(parse_config_arg_item("").is_err());
        assert!(parse_config_arg_item("a").is_err());
        assert!(parse_config_arg_item("=").is_err());
        // The value parser is sensitive to leading whitespaces, which seems
        // good because the parsing falls back to a bare string.
        assert!(parse_config_arg_item("a = 'b'").is_err());

        let (name, value) = parse_config_arg_item("a=b").unwrap();
        assert_eq!(name, ConfigNamePathBuf::from_iter(["a"]));
        assert_eq!(value.as_str(), Some("b"));

        let (name, value) = parse_config_arg_item("a=").unwrap();
        assert_eq!(name, ConfigNamePathBuf::from_iter(["a"]));
        assert_eq!(value.as_str(), Some(""));

        let (name, value) = parse_config_arg_item("a= ").unwrap();
        assert_eq!(name, ConfigNamePathBuf::from_iter(["a"]));
        assert_eq!(value.as_str(), Some(" "));

        // This one is a bit cryptic, but b=c can be a bare string.
        let (name, value) = parse_config_arg_item("a=b=c").unwrap();
        assert_eq!(name, ConfigNamePathBuf::from_iter(["a"]));
        assert_eq!(value.as_str(), Some("b=c"));

        let (name, value) = parse_config_arg_item("a.b=true").unwrap();
        assert_eq!(name, ConfigNamePathBuf::from_iter(["a", "b"]));
        assert_eq!(value.as_bool(), Some(true));

        let (name, value) = parse_config_arg_item("a='b=c'").unwrap();
        assert_eq!(name, ConfigNamePathBuf::from_iter(["a"]));
        assert_eq!(value.as_str(), Some("b=c"));

        let (name, value) = parse_config_arg_item("'a=b'=c").unwrap();
        assert_eq!(name, ConfigNamePathBuf::from_iter(["a=b"]));
        assert_eq!(value.as_str(), Some("c"));

        let (name, value) = parse_config_arg_item("'a = b=c '={d = 'e=f'}").unwrap();
        assert_eq!(name, ConfigNamePathBuf::from_iter(["a = b=c "]));
        assert!(value.is_inline_table());
        assert_eq!(value.to_string(), "{d = 'e=f'}");
    }

    #[test]
    fn test_resolved_config_values_empty() {
        let config = StackedConfig::empty();
        assert!(resolved_config_values(&config, &ConfigNamePathBuf::root()).is_empty());
    }

    #[test]
    fn test_resolved_config_values_single_key() -> TestResult {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();
        let mut env_base_layer = ConfigLayer::empty(ConfigSource::EnvBase);
        env_base_layer.set_value("user.name", "base-user-name")?;
        env_base_layer.set_value("user.email", "base@user.email")?;
        let mut repo_layer = ConfigLayer::empty(ConfigSource::Repo);
        repo_layer.set_value("user.email", "repo@user.email")?;
        let mut config = StackedConfig::empty();
        config.add_layer(env_base_layer);
        config.add_layer(repo_layer);
        // Note: "email" is alphabetized, before "name" from same layer.
        insta::assert_debug_snapshot!(
            resolved_config_values(&config, &ConfigNamePathBuf::root()),
            @r#"
        [
            AnnotatedValue {
                name: ConfigNamePathBuf(
                    [
                        Key {
                            key: "user",
                            repr: None,
                            leaf_decor: Decor { .. },
                            dotted_decor: Decor { .. },
                        },
                        Key {
                            key: "name",
                            repr: None,
                            leaf_decor: Decor { .. },
                            dotted_decor: Decor { .. },
                        },
                    ],
                ),
                value: String(
                    Formatted {
                        value: "base-user-name",
                        repr: "default",
                        decor: Decor { .. },
                    },
                ),
                source: EnvBase,
                path: None,
                is_overridden: false,
            },
            AnnotatedValue {
                name: ConfigNamePathBuf(
                    [
                        Key {
                            key: "user",
                            repr: None,
                            leaf_decor: Decor { .. },
                            dotted_decor: Decor { .. },
                        },
                        Key {
                            key: "email",
                            repr: None,
                            leaf_decor: Decor { .. },
                            dotted_decor: Decor { .. },
                        },
                    ],
                ),
                value: String(
                    Formatted {
                        value: "base@user.email",
                        repr: "default",
                        decor: Decor { .. },
                    },
                ),
                source: EnvBase,
                path: None,
                is_overridden: true,
            },
            AnnotatedValue {
                name: ConfigNamePathBuf(
                    [
                        Key {
                            key: "user",
                            repr: None,
                            leaf_decor: Decor { .. },
                            dotted_decor: Decor { .. },
                        },
                        Key {
                            key: "email",
                            repr: None,
                            leaf_decor: Decor { .. },
                            dotted_decor: Decor { .. },
                        },
                    ],
                ),
                value: String(
                    Formatted {
                        value: "repo@user.email",
                        repr: "default",
                        decor: Decor { .. },
                    },
                ),
                source: Repo,
                path: None,
                is_overridden: false,
            },
        ]
        "#
        );
        Ok(())
    }

    #[test]
    fn test_resolved_config_values_filter_path() -> TestResult {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();
        let mut user_layer = ConfigLayer::empty(ConfigSource::User);
        user_layer.set_value("test-table1.foo", "user-FOO")?;
        user_layer.set_value("test-table2.bar", "user-BAR")?;
        let mut repo_layer = ConfigLayer::empty(ConfigSource::Repo);
        repo_layer.set_value("test-table1.bar", "repo-BAR")?;
        let mut config = StackedConfig::empty();
        config.add_layer(user_layer);
        config.add_layer(repo_layer);
        insta::assert_debug_snapshot!(
            resolved_config_values(&config, &ConfigNamePathBuf::from_iter(["test-table1"])),
            @r#"
        [
            AnnotatedValue {
                name: ConfigNamePathBuf(
                    [
                        Key {
                            key: "test-table1",
                            repr: None,
                            leaf_decor: Decor { .. },
                            dotted_decor: Decor { .. },
                        },
                        Key {
                            key: "foo",
                            repr: None,
                            leaf_decor: Decor { .. },
                            dotted_decor: Decor { .. },
                        },
                    ],
                ),
                value: String(
                    Formatted {
                        value: "user-FOO",
                        repr: "default",
                        decor: Decor { .. },
                    },
                ),
                source: User,
                path: None,
                is_overridden: false,
            },
            AnnotatedValue {
                name: ConfigNamePathBuf(
                    [
                        Key {
                            key: "test-table1",
                            repr: None,
                            leaf_decor: Decor { .. },
                            dotted_decor: Decor { .. },
                        },
                        Key {
                            key: "bar",
                            repr: None,
                            leaf_decor: Decor { .. },
                            dotted_decor: Decor { .. },
                        },
                    ],
                ),
                value: String(
                    Formatted {
                        value: "repo-BAR",
                        repr: "default",
                        decor: Decor { .. },
                    },
                ),
                source: Repo,
                path: None,
                is_overridden: false,
            },
        ]
        "#
        );
        Ok(())
    }

    #[test]
    fn test_resolved_config_values_overridden() -> TestResult {
        let list = |layers: &[&ConfigLayer], prefix: &str| -> String {
            let mut config = StackedConfig::empty();
            config.extend_layers(layers.iter().copied().cloned());
            let prefix = if prefix.is_empty() {
                ConfigNamePathBuf::root()
            } else {
                prefix.parse().unwrap()
            };
            let mut output = String::new();
            for annotated in resolved_config_values(&config, &prefix) {
                let AnnotatedValue { name, value, .. } = &annotated;
                let sigil = if annotated.is_overridden { '!' } else { ' ' };
                writeln!(output, "{sigil}{name} = {value}").unwrap();
            }
            output
        };

        let mut layer0 = ConfigLayer::empty(ConfigSource::User);
        layer0.set_value("a.b.e", "0.0")?;
        layer0.set_value("a.b.c.f", "0.1")?;
        layer0.set_value("a.b.d", "0.2")?;
        let mut layer1 = ConfigLayer::empty(ConfigSource::User);
        layer1.set_value("a.b", "1.0")?;
        layer1.set_value("a.c", "1.1")?;
        let mut layer2 = ConfigLayer::empty(ConfigSource::User);
        layer2.set_value("a.b.g", "2.0")?;
        layer2.set_value("a.b.d", "2.1")?;

        // a.b.* is shadowed by a.b
        let layers = [&layer0, &layer1];
        insta::assert_snapshot!(list(&layers, ""), @r#"
        !a.b.e = "0.0"
        !a.b.c.f = "0.1"
        !a.b.d = "0.2"
         a.b = "1.0"
         a.c = "1.1"
        "#);
        insta::assert_snapshot!(list(&layers, "a.b"), @r#"
        !a.b.e = "0.0"
        !a.b.c.f = "0.1"
        !a.b.d = "0.2"
         a.b = "1.0"
        "#);
        insta::assert_snapshot!(list(&layers, "a.b.c"), @r#"!a.b.c.f = "0.1""#);
        insta::assert_snapshot!(list(&layers, "a.b.d"), @r#"!a.b.d = "0.2""#);

        // a.b is shadowed by a.b.*
        let layers = [&layer1, &layer2];
        insta::assert_snapshot!(list(&layers, ""), @r#"
        !a.b = "1.0"
         a.c = "1.1"
         a.b.g = "2.0"
         a.b.d = "2.1"
        "#);
        insta::assert_snapshot!(list(&layers, "a.b"), @r#"
        !a.b = "1.0"
         a.b.g = "2.0"
         a.b.d = "2.1"
        "#);

        // a.b.d is shadowed by a.b.d
        let layers = [&layer0, &layer2];
        insta::assert_snapshot!(list(&layers, ""), @r#"
         a.b.e = "0.0"
         a.b.c.f = "0.1"
        !a.b.d = "0.2"
         a.b.g = "2.0"
         a.b.d = "2.1"
        "#);
        insta::assert_snapshot!(list(&layers, "a.b"), @r#"
         a.b.e = "0.0"
         a.b.c.f = "0.1"
        !a.b.d = "0.2"
         a.b.g = "2.0"
         a.b.d = "2.1"
        "#);
        insta::assert_snapshot!(list(&layers, "a.b.c"), @r#" a.b.c.f = "0.1""#);
        insta::assert_snapshot!(list(&layers, "a.b.d"), @r#"
        !a.b.d = "0.2"
         a.b.d = "2.1"
        "#);

        // a.b.* is shadowed by a.b, which is shadowed by a.b.*
        let layers = [&layer0, &layer1, &layer2];
        insta::assert_snapshot!(list(&layers, ""), @r#"
        !a.b.e = "0.0"
        !a.b.c.f = "0.1"
        !a.b.d = "0.2"
        !a.b = "1.0"
         a.c = "1.1"
         a.b.g = "2.0"
         a.b.d = "2.1"
        "#);
        insta::assert_snapshot!(list(&layers, "a.b"), @r#"
        !a.b.e = "0.0"
        !a.b.c.f = "0.1"
        !a.b.d = "0.2"
        !a.b = "1.0"
         a.b.g = "2.0"
         a.b.d = "2.1"
        "#);
        insta::assert_snapshot!(list(&layers, "a.b.c"), @r#"!a.b.c.f = "0.1""#);
        Ok(())
    }
}
