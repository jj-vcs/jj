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

use std::fmt;

use jj_lib::config::ConfigGetError;
use jj_lib::config::ConfigNamePathBuf;
use jj_lib::config::ConfigSource;
use jj_lib::config::StackedConfig;
use jj_lib::dsl_util::AliasDeclarationParser;
use jj_lib::dsl_util::AliasesMap;
use jj_lib::fileset::FilesetAliasesMap;
use jj_lib::revset::RevsetAliasesMap;

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
