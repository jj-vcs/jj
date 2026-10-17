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

use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fmt;
use std::process::Command;
use std::sync::LazyLock;

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
use regex::Regex;
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

/// Command name and arguments specified by config.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(untagged)]
pub enum CommandNameAndArgs {
    /// A command line, which is split into words at spaces, unless quotes are
    /// used.
    String(String),
    /// A command name followed by arguments.
    Vec(NonEmptyCommandArgsVec),
    /// A command with environment variables to set.
    Structured {
        /// Environment variables to set for the command.
        env: HashMap<String, String>,
        /// The command name followed by arguments.
        command: NonEmptyCommandArgsVec,
    },
}

impl CommandNameAndArgs {
    /// Returns command name without arguments.
    pub fn split_name(&self) -> Cow<'_, str> {
        let (name, _) = self.split_name_and_args();
        name
    }

    /// Returns command name and arguments.
    ///
    /// The command name may be an empty string (as well as each argument.)
    pub fn split_name_and_args(&self) -> (Cow<'_, str>, Cow<'_, [String]>) {
        match self {
            Self::String(s) => {
                if s.contains('"') || s.contains('\'') {
                    let mut parts = shlex::Shlex::new(s);
                    let res = (
                        parts.next().unwrap_or_default().into(),
                        parts.by_ref().collect(),
                    );
                    if !parts.had_error {
                        return res;
                    }
                }
                let mut args = s.split(' ').map(|s| s.to_owned());
                (args.next().unwrap().into(), args.collect())
            }
            Self::Vec(NonEmptyCommandArgsVec(a)) => (Cow::Borrowed(&a[0]), Cow::Borrowed(&a[1..])),
            Self::Structured {
                env: _,
                command: cmd,
            } => (Cow::Borrowed(&cmd.0[0]), Cow::Borrowed(&cmd.0[1..])),
        }
    }

    /// Returns command string only if the underlying type is a string.
    ///
    /// Use this to parse enum strings such as `":builtin"`, which can be
    /// escaped as `[":builtin"]`.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            Self::Vec(_) | Self::Structured { .. } => None,
        }
    }

    /// Returns process builder configured with this.
    pub fn to_command(&self) -> Command {
        let empty: HashMap<&str, &str> = HashMap::new();
        self.to_command_with_variables(&empty)
    }

    /// Returns process builder configured with this after interpolating
    /// variables into the arguments.
    pub fn to_command_with_variables<V: AsRef<OsStr>>(
        &self,
        variables: &HashMap<&str, V>,
    ) -> Command {
        let (name, args) = self.split_name_and_args();
        let mut cmd = Command::new(interpolate_variables_single(name.as_ref(), variables));
        if let Self::Structured { env, .. } = self {
            cmd.envs(env);
        }
        cmd.args(interpolate_variables(&args, variables));
        cmd
    }
}

impl<T: AsRef<str> + ?Sized> From<&T> for CommandNameAndArgs {
    fn from(s: &T) -> Self {
        Self::String(s.as_ref().to_owned())
    }
}

impl fmt::Display for CommandNameAndArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(s) => write!(f, "{s}"),
            // TODO: format with shell escapes
            Self::Vec(a) => write!(f, "{}", a.0.join(" ")),
            Self::Structured { env, command } => {
                for (k, v) in env {
                    write!(f, "{k}={v} ")?;
                }
                write!(f, "{}", command.0.join(" "))
            }
        }
    }
}

// Not interested in $UPPER_CASE_VARIABLES
static VARIABLE_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\$[a-z0-9_]+\b").unwrap());

/// Replaces `$name` variables in `args` with the values in `variables`.
/// Unknown variables are left as is.
pub fn interpolate_variables<A: AsRef<str>, V: AsRef<OsStr>>(
    args: &[A],
    variables: &HashMap<&str, V>,
) -> Vec<OsString> {
    args.iter()
        .map(|arg| interpolate_variables_single(arg.as_ref(), variables))
        .collect()
}

fn interpolate_variables_single<V: AsRef<OsStr>>(
    arg: &str,
    variables: &HashMap<&str, V>,
) -> OsString {
    let mut buf = OsString::new();
    let mut end = 0;
    for m in VARIABLE_REGEX.find_iter(arg) {
        let name = &m.as_str()[1..];
        if let Some(subst) = variables.get(name) {
            buf.push(&arg[end..m.start()]);
            buf.push(subst);
            end = m.end();
        }
    }
    buf.push(&arg[end..]);
    buf
}

/// Return all variable names found in the args, without the dollar sign
pub fn find_all_variables<A: AsRef<str>>(args: &[A]) -> impl Iterator<Item = &str> {
    let regex = &*VARIABLE_REGEX;
    args.iter()
        .flat_map(|arg| regex.find_iter(arg.as_ref()))
        .map(|single_match| {
            let s = single_match.as_str();
            &s[1..]
        })
}

/// Wrapper to reject an array without command name.
// Based on https://github.com/serde-rs/serde/issues/939
#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Deserialize)]
#[serde(try_from = "Vec<String>")]
pub struct NonEmptyCommandArgsVec(Vec<String>);

impl TryFrom<Vec<String>> for NonEmptyCommandArgsVec {
    type Error = &'static str;

    fn try_from(args: Vec<String>) -> Result<Self, Self::Error> {
        if args.is_empty() {
            Err("command arguments should not be empty")
        } else {
            Ok(Self(args))
        }
    }
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use itertools::Itertools as _;
    use maplit::hashmap;
    use testutils::TestResult;

    use super::*;

    #[test]
    fn test_command_args() -> TestResult {
        let mut config = StackedConfig::empty();
        config.add_layer(ConfigLayer::parse(
            ConfigSource::User,
            indoc! {"
                    empty_array = []
                    empty_string = ''
                    array = ['emacs', '-nw']
                    string = 'emacs -nw'
                    string_quoted = '\"spaced path/to/emacs\" -nw'
                    structured.env = { KEY1 = 'value1', KEY2 = 'value2' }
                    structured.command = ['emacs', '-nw']
                "},
        )?);

        assert!(config.get::<CommandNameAndArgs>("empty_array").is_err());

        let command_args: CommandNameAndArgs = config.get("empty_string")?;
        assert_eq!(command_args, CommandNameAndArgs::String("".to_owned()));
        let (name, args) = command_args.split_name_and_args();
        assert_eq!(name, "");
        assert!(args.is_empty());

        let command_args: CommandNameAndArgs = config.get("array")?;
        assert_eq!(
            command_args,
            CommandNameAndArgs::Vec(NonEmptyCommandArgsVec(
                ["emacs", "-nw",].map(|s| s.to_owned()).to_vec()
            ))
        );
        let (name, args) = command_args.split_name_and_args();
        assert_eq!(name, "emacs");
        assert_eq!(args, ["-nw"].as_ref());

        let command_args: CommandNameAndArgs = config.get("string")?;
        assert_eq!(
            command_args,
            CommandNameAndArgs::String("emacs -nw".to_owned())
        );
        let (name, args) = command_args.split_name_and_args();
        assert_eq!(name, "emacs");
        assert_eq!(args, ["-nw"].as_ref());

        let command_args: CommandNameAndArgs = config.get("string_quoted")?;
        assert_eq!(
            command_args,
            CommandNameAndArgs::String("\"spaced path/to/emacs\" -nw".to_owned())
        );
        let (name, args) = command_args.split_name_and_args();
        assert_eq!(name, "spaced path/to/emacs");
        assert_eq!(args, ["-nw"].as_ref());

        let command_args: CommandNameAndArgs = config.get("structured")?;
        assert_eq!(
            command_args,
            CommandNameAndArgs::Structured {
                env: hashmap! {
                    "KEY1".to_string() => "value1".to_string(),
                    "KEY2".to_string() => "value2".to_string(),
                },
                command: NonEmptyCommandArgsVec(["emacs", "-nw",].map(|s| s.to_owned()).to_vec())
            }
        );
        let (name, args) = command_args.split_name_and_args();
        assert_eq!(name, "emacs");
        assert_eq!(args, ["-nw"].as_ref());
        Ok(())
    }

    #[test]
    fn test_interpolate_variables() {
        let patterns = hashmap! {
            "left" => "LEFT",
            "right" => "RIGHT",
            "left_right" => "$left $right",
        };

        assert_eq!(
            interpolate_variables(&["$left", "$1", "$right", "$2"], &patterns),
            ["LEFT", "$1", "RIGHT", "$2"],
        );

        // Option-like
        assert_eq!(
            interpolate_variables(&["-o$left$right"], &patterns),
            ["-oLEFTRIGHT"],
        );

        // Sexp-like
        assert_eq!(
            interpolate_variables(&["($unknown $left $right)"], &patterns),
            ["($unknown LEFT RIGHT)"],
        );

        // Not a word "$left"
        assert_eq!(interpolate_variables(&["$lefty"], &patterns), ["$lefty"]);

        // Patterns in pattern: not expanded recursively
        assert_eq!(
            interpolate_variables(&["$left_right"], &patterns),
            ["$left $right"],
        );
    }

    #[test]
    fn test_find_all_variables() {
        assert_eq!(
            find_all_variables(&[
                "$left",
                "$right",
                "--two=$1 and $2",
                "--can-be-part-of-string=$output",
                "$NOT_CAPITALS",
                "--can-repeat=$right"
            ],)
            .collect_vec(),
            ["left", "right", "1", "2", "output", "right"],
        );
    }
}
