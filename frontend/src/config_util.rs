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

//! Loading configuration used by user interfaces.

use std::collections::HashMap;
use std::fmt;

use jj_lib::config::ConfigGetError;
use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigNamePathBuf;
use jj_lib::config::ConfigSource;
use jj_lib::config::StackedConfig;
use jj_lib::dsl_util::AliasDeclarationParser;
use jj_lib::dsl_util::AliasesMap;
use jj_lib::fileset::FilesetAliasesMap;
use jj_lib::ref_name::RemoteName;
use jj_lib::ref_name::RemoteNameBuf;
use jj_lib::revset;
use jj_lib::revset::RevsetAliasesMap;
use jj_lib::revset::RevsetDiagnostics;
use jj_lib::revset::RevsetParseError;
use jj_lib::settings::RemoteSettingsMap;
use jj_lib::str_util::StringExpression;
use jj_lib::str_util::StringMatcher;
use thiserror::Error;

/// Returns the default configuration that the logic in this crate depends on,
/// such as the `trunk()` and `immutable_heads()` revset aliases and the
/// `snapshot.*` settings. User interfaces should include these layers in
/// their configuration, before any user configuration.
pub fn default_config_layers() -> Vec<ConfigLayer> {
    // Syntax error in default config isn't a user error. That's why defaults are
    // loaded by separate builder.
    let parse = |text: &'static str| ConfigLayer::parse(ConfigSource::Default, text).unwrap();
    vec![
        parse(include_str!("config/misc.toml")),
        parse(include_str!("config/revsets.toml")),
    ]
}

/// Problem found while loading configuration. Loading continues after these.
#[derive(Clone, Debug)]
pub enum ConfigWarning {
    /// An alias couldn't be loaded and was skipped.
    InvalidAlias {
        /// The table containing the alias, e.g. `revset-aliases`.
        table_name: ConfigNamePathBuf,
        /// The alias declaration, e.g. `foo(x)`.
        decl: String,
        /// Why the alias couldn't be loaded.
        message: String,
    },
    /// A built-in revset alias that defines the immutable set was redefined.
    /// `immutable_heads()` should be redefined instead.
    RedefinedMutabilityBuiltin {
        /// The table containing the alias.
        table_name: ConfigNamePathBuf,
        /// The alias declaration, e.g. `immutable()`.
        decl: &'static str,
    },
    /// Something unexpected was found while locating a repo- or
    /// workspace-specific config file, such as the repo having been copied.
    SecureConfig {
        /// Description of the problem.
        message: String,
    },
}

impl fmt::Display for ConfigWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAlias {
                table_name,
                decl,
                message,
            } => write!(f, "Failed to load `{table_name}.{decl}`: {message}"),
            Self::RedefinedMutabilityBuiltin { table_name, decl } => write!(
                f,
                "Redefining `{table_name}.{decl}` is not recommended; redefine \
                 `immutable_heads()` instead."
            ),
            Self::SecureConfig { message } => write!(f, "{message}"),
        }
    }
}

/// Loads map of `{fileset,revset,template}-aliases` table from all config
/// layers. Invalid aliases are skipped and reported in `warnings`.
///
/// This function does not handle loading the `aliases` table, which supports
/// disabled aliases.
pub fn load_aliases_map<P>(
    warnings: &mut Vec<ConfigWarning>,
    config: &StackedConfig,
    table_name: &ConfigNamePathBuf,
) -> Result<AliasesMap<P, String>, ConfigGetError>
where
    P: AliasDeclarationParser + Default,
    P::Error: fmt::Display,
{
    let mut aliases_map = AliasesMap::new();
    // Load from all config layers in order. 'f(x)' in default layer should be
    // overridden by 'f(a)' in user.
    for layer in config.layers() {
        let table = match layer.look_up_table(table_name) {
            Ok(Some(table)) => table,
            Ok(None) => continue,
            Err(item) => {
                return Err(ConfigGetError::Type {
                    name: table_name.to_string(),
                    error: format!("Expected a table, but is {}", item.type_name()).into(),
                    source_path: layer.path.clone(),
                });
            }
        };
        for (decl, item) in table.iter() {
            let (definition, doc) = if let Some(t) = item.as_table_like() {
                let definition = t.get("definition").and_then(|i| i.as_str());
                let doc = t.get("doc").and_then(|i| i.as_str()).map(|s| s.to_owned());
                (definition, doc)
            } else {
                (item.as_str(), None)
            };

            let r = definition
                .ok_or_else(|| {
                    format!(
                        "Expected a string or a table with a `definition` string key, but is {}",
                        item.type_name()
                    )
                })
                .and_then(|v| aliases_map.insert(decl, v, doc).map_err(|e| format!("{e}")));
            if let Err(message) = r {
                warnings.push(ConfigWarning::InvalidAlias {
                    table_name: table_name.clone(),
                    decl: decl.to_owned(),
                    message,
                });
            }
        }
    }
    Ok(aliases_map)
}

/// Loads the `fileset-aliases` table.
pub fn load_fileset_aliases(
    warnings: &mut Vec<ConfigWarning>,
    config: &StackedConfig,
) -> Result<FilesetAliasesMap, ConfigGetError> {
    let table_name = ConfigNamePathBuf::from_iter(["fileset-aliases"]);
    load_aliases_map(warnings, config, &table_name)
}

/// Loads the `revset-aliases` table.
pub fn load_revset_aliases(
    warnings: &mut Vec<ConfigWarning>,
    config: &StackedConfig,
) -> Result<RevsetAliasesMap, ConfigGetError> {
    let table_name = ConfigNamePathBuf::from_iter(["revset-aliases"]);
    let aliases_map = load_aliases_map(warnings, config, &table_name)?;
    check_user_redefined_builtin(warnings, config, &table_name);
    Ok(aliases_map)
}

fn check_user_redefined_builtin(
    warnings: &mut Vec<ConfigWarning>,
    config: &StackedConfig,
    table_name: &ConfigNamePathBuf,
) {
    let checked_mutability_builtins = ["mutable()", "immutable()", "builtin_immutable_heads()"];
    for layer in config
        .layers()
        .iter()
        .skip_while(|layer| layer.source == ConfigSource::Default)
    {
        let Ok(Some(table)) = layer.look_up_table(table_name) else {
            continue;
        };
        for decl in checked_mutability_builtins
            .iter()
            .filter(|decl| table.contains_key(decl))
        {
            warnings.push(ConfigWarning::RedefinedMutabilityBuiltin {
                table_name: table_name.clone(),
                decl,
            });
        }
    }
}

/// A `remotes.<name>.<field>` setting couldn't be parsed.
#[derive(Debug, Error)]
#[error("Invalid `remotes.{}.{field_name}`: {}", remote.as_symbol(), source.kind())]
pub struct RemoteSettingParseError {
    /// The remote whose setting couldn't be parsed.
    pub remote: RemoteNameBuf,
    /// The name of the setting, e.g. `auto-track-bookmarks`.
    pub field_name: &'static str,
    /// The underlying parse error.
    #[source]
    pub source: RevsetParseError,
}

/// Parse diagnostics for a `remotes.<name>.<field>` setting.
#[derive(Debug)]
pub struct RemoteSettingDiagnostics {
    /// The remote whose setting had diagnostics.
    pub remote: RemoteNameBuf,
    /// The name of the setting, e.g. `auto-track-bookmarks`.
    pub field_name: &'static str,
    /// The diagnostics.
    pub diagnostics: RevsetDiagnostics,
}

/// Parses the string pattern `text` from the `remotes.<name>.<field_name>`
/// setting. Any non-fatal problems are added to `diagnostics`.
pub fn parse_remote_string_expression(
    diagnostics: &mut Vec<RemoteSettingDiagnostics>,
    name: &RemoteName,
    text: &str,
    field_name: &'static str,
) -> Result<StringExpression, RemoteSettingParseError> {
    let mut revset_diagnostics = RevsetDiagnostics::new();
    let result = revset::parse_string_expression(&mut revset_diagnostics, text);
    if !revset_diagnostics.is_empty() {
        diagnostics.push(RemoteSettingDiagnostics {
            remote: name.to_owned(),
            field_name,
            diagnostics: revset_diagnostics,
        });
    }
    result.map_err(|source| RemoteSettingParseError {
        remote: name.to_owned(),
        field_name,
        source,
    })
}

/// Parses the `remotes.<name>.fetch-bookmarks` setting, if set. Any non-fatal
/// problems are added to `diagnostics`.
pub fn parse_remote_fetch_bookmarks(
    diagnostics: &mut Vec<RemoteSettingDiagnostics>,
    remote_settings: &RemoteSettingsMap,
    name: &RemoteName,
) -> Result<Option<StringExpression>, RemoteSettingParseError> {
    remote_settings
        .get(name)
        .and_then(|settings| settings.fetch_bookmarks.as_ref())
        .map(|text| parse_remote_string_expression(diagnostics, name, text, "fetch-bookmarks"))
        .transpose()
}

/// Parses the `remotes.<name>.fetch-tags` setting, if set. Any non-fatal
/// problems are added to `diagnostics`.
pub fn parse_remote_fetch_tags(
    diagnostics: &mut Vec<RemoteSettingDiagnostics>,
    remote_settings: &RemoteSettingsMap,
    name: &RemoteName,
) -> Result<Option<StringExpression>, RemoteSettingParseError> {
    remote_settings
        .get(name)
        .and_then(|settings| settings.fetch_tags.as_ref())
        .map(|text| parse_remote_string_expression(diagnostics, name, text, "fetch-tags"))
        .transpose()
}

/// Parses the given `remotes.<name>.auto-track-bookmarks` settings into a map
/// of string matchers.
pub fn parse_remote_auto_track_bookmarks_map(
    diagnostics: &mut Vec<RemoteSettingDiagnostics>,
    remote_settings: &RemoteSettingsMap,
) -> Result<HashMap<RemoteNameBuf, StringMatcher>, RemoteSettingParseError> {
    let mut matchers = HashMap::new();
    for (name, settings) in remote_settings {
        let Some(text) = &settings.auto_track_bookmarks else {
            continue;
        };
        let expr = parse_remote_string_expression(diagnostics, name, text, "auto-track-bookmarks")?;
        matchers.insert(name.clone(), expr.to_matcher());
    }
    Ok(matchers)
}
