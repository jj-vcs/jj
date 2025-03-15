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

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::env;
use std::env::split_paths;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use etcetera::BaseStrategy as _;
use itertools::Itertools as _;
use jj_lib::config::ConfigFile;
use jj_lib::config::ConfigGetError;
use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigLoadError;
use jj_lib::config::ConfigMigrationRule;
use jj_lib::config::ConfigNamePathBuf;
use jj_lib::config::ConfigResolutionContext;
use jj_lib::config::ConfigSource;
use jj_lib::config::ConfigValue;
use jj_lib::config::StackedConfig;
use jj_lib::dsl_util;
use regex::Captures;
use regex::Regex;
use tracing::instrument;

use crate::command_error::config_error;
use crate::command_error::config_error_with_message;
use crate::command_error::CommandError;
use crate::text_util;
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

/// Configuration variable with its source information.
#[derive(Clone, Debug)]
pub struct AnnotatedValue {
    /// Dotted name path to the configuration variable.
    pub name: ConfigNamePathBuf,
    /// Configuration value.
    pub value: ConfigValue,
    /// Source of the configuration value.
    pub source: ConfigSource,
    /// Path to the source file, if available.
    pub path: Option<PathBuf>,
    /// True if this value is overridden in higher precedence layers.
    pub is_overridden: bool,
}

/// Collects values under the given `filter_prefix` name recursively, from all
/// layers.
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

/// Newtype for unprocessed (or unresolved) [`StackedConfig`].
///
/// This doesn't provide any strict guarantee about the underlying config
/// object. It just requires an explicit cast to access to the config object.
#[derive(Clone, Debug)]
pub struct RawConfig(StackedConfig);

impl AsRef<StackedConfig> for RawConfig {
    fn as_ref(&self) -> &StackedConfig {
        &self.0
    }
}

impl AsMut<StackedConfig> for RawConfig {
    fn as_mut(&mut self) -> &mut StackedConfig {
        &mut self.0
    }
}

#[derive(Clone, Debug)]
enum ConfigPathState {
    New,
    Exists,
}

/// A ConfigPath can be in one of two states:
///
/// - exists(): a config file exists at the path
/// - !exists(): a config file doesn't exist here, but a new file _can_ be
///   created at this path
#[derive(Clone, Debug)]
struct ConfigPath {
    path: PathBuf,
    state: ConfigPathState,
}

impl ConfigPath {
    fn new(path: PathBuf) -> Self {
        use ConfigPathState::*;
        ConfigPath {
            state: if path.exists() { Exists } else { New },
            path,
        }
    }

    fn as_path(&self) -> &Path {
        &self.path
    }
    fn exists(&self) -> bool {
        match self.state {
            ConfigPathState::Exists => true,
            ConfigPathState::New => false,
        }
    }
}

/// Like std::fs::create_dir_all but creates new directories to be accessible to
/// the user only on Unix (chmod 700).
fn create_dir_all(path: &Path) -> std::io::Result<()> {
    let mut dir = std::fs::DirBuilder::new();
    dir.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        dir.mode(0o700);
    }
    dir.create(path)
}

// The struct exists so that we can mock certain global values in unit tests.
#[derive(Clone, Default, Debug)]
struct UnresolvedConfigEnv {
    config_dir: Option<PathBuf>,
    // TODO: remove after jj 0.35
    macos_legacy_config_dir: Option<PathBuf>,
    home_dir: Option<PathBuf>,
    jj_config: Option<String>,
}

impl UnresolvedConfigEnv {
    fn resolve(self, ui: &Ui) -> Vec<ConfigPath> {
        if let Some(paths) = self.jj_config {
            return split_paths(&paths)
                .filter(|path| !path.as_os_str().is_empty())
                .map(ConfigPath::new)
                .collect();
        }

        let mut paths = vec![];
        let home_config_path = self.home_dir.map(|mut home_dir| {
            home_dir.push(".jjconfig.toml");
            ConfigPath::new(home_dir)
        });
        let platform_config_path = self.config_dir.clone().map(|mut config_dir| {
            config_dir.push("jj");
            config_dir.push("config.toml");
            ConfigPath::new(config_dir)
        });
        let platform_config_dir = self.config_dir.map(|mut config_dir| {
            config_dir.push("jj");
            config_dir.push("conf.d");
            ConfigPath::new(config_dir)
        });
        let legacy_platform_config_path =
            self.macos_legacy_config_dir.clone().map(|mut config_dir| {
                config_dir.push("jj");
                config_dir.push("config.toml");
                ConfigPath::new(config_dir)
            });
        let legacy_platform_config_dir = self.macos_legacy_config_dir.map(|mut config_dir| {
            config_dir.push("jj");
            config_dir.push("conf.d");
            ConfigPath::new(config_dir)
        });

        if let Some(path) = home_config_path {
            if path.exists()
                || (platform_config_path.is_none() && legacy_platform_config_path.is_none())
            {
                paths.push(path);
            }
        }

        // This should be the default config created if there's
        // no user config and `jj config edit` is executed.
        if let Some(path) = platform_config_path {
            paths.push(path);
        }

        // theoretically these should be an `if let Some(...) = ... && ..., but that
        // isn't stable
        if let Some(path) = platform_config_dir {
            if path.exists() {
                paths.push(path);
            }
        }

        if let Some(path) = legacy_platform_config_path {
            if path.exists() {
                Self::warn_for_deprecated_path(
                    ui,
                    path.as_path(),
                    "~/Library/Application Support",
                    "~/.config",
                );
                paths.push(path);
            }
        }
        if let Some(path) = legacy_platform_config_dir {
            if path.exists() {
                Self::warn_for_deprecated_path(
                    ui,
                    path.as_path(),
                    "~/Library/Application Support",
                    "~/.config",
                );
                paths.push(path);
            }
        }

        paths
    }

    fn warn_for_deprecated_path(ui: &Ui, path: &Path, old: &str, new: &str) {
        let _ = indoc::writedoc!(
            ui.warning_default(),
            r"
            Deprecated configuration file `{}`.
            Configuration files in `{old}` are deprecated, and support will be removed in a future release.
            Instead, move your configuration files to `{new}`.
            ",
            path.display(),
        );
    }
}

#[derive(Clone, Debug)]
pub struct ConfigEnv {
    home_dir: Option<PathBuf>,
    repo_path: Option<PathBuf>,
    user_config_paths: Vec<ConfigPath>,
    repo_config_path: Option<ConfigPath>,
    command: Option<String>,
}

impl ConfigEnv {
    /// Initializes configuration loader based on environment variables.
    pub fn from_environment(ui: &Ui) -> Self {
        let config_dir = etcetera::choose_base_strategy()
            .ok()
            .map(|s| s.config_dir());

        // older versions of jj used a more "GUI" config option,
        // which is not designed for user-editable configuration of CLI utilities.
        let macos_legacy_config_dir = if cfg!(target_os = "macos") {
            etcetera::base_strategy::choose_native_strategy()
                .ok()
                .map(|s| {
                    // note that etcetera calls Library/Application Support the "data dir",
                    // Library/Preferences is supposed to be exclusively plists
                    s.data_dir()
                })
        } else {
            None
        };

        // Canonicalize home as we do canonicalize cwd in CliRunner. $HOME might
        // point to symlink.
        let home_dir = etcetera::home_dir()
            .ok()
            .map(|d| dunce::canonicalize(&d).unwrap_or(d));

        let env = UnresolvedConfigEnv {
            config_dir,
            macos_legacy_config_dir,
            home_dir: home_dir.clone(),
            jj_config: env::var("JJ_CONFIG").ok(),
        };
        ConfigEnv {
            home_dir,
            repo_path: None,
            user_config_paths: env.resolve(ui),
            repo_config_path: None,
            command: None,
        }
    }

    pub fn set_command_name(&mut self, command: String) {
        self.command = Some(command);
    }

    /// Returns the paths to the user-specific config files or directories.
    pub fn user_config_paths(&self) -> impl Iterator<Item = &Path> {
        self.user_config_paths.iter().map(ConfigPath::as_path)
    }

    /// Returns the paths to the existing user-specific config files or
    /// directories.
    pub fn existing_user_config_paths(&self) -> impl Iterator<Item = &Path> {
        self.user_config_paths
            .iter()
            .filter(|p| p.exists())
            .map(ConfigPath::as_path)
    }

    /// Returns user configuration files for modification. Instantiates one if
    /// `config` has no user configuration layers.
    ///
    /// The parent directory for the new file may be created by this function.
    /// If the user configuration path is unknown, this function returns an
    /// empty `Vec`.
    pub fn user_config_files(
        &self,
        config: &RawConfig,
    ) -> Result<Vec<ConfigFile>, ConfigLoadError> {
        config_files_for(config, ConfigSource::User, || self.new_user_config_file())
    }

    fn new_user_config_file(&self) -> Result<Option<ConfigFile>, ConfigLoadError> {
        self.user_config_paths()
            .next()
            .map(|path| {
                // No need to propagate io::Error here. If the directory
                // couldn't be created, file.save() would fail later.
                if let Some(dir) = path.parent() {
                    create_dir_all(dir).ok();
                }
                // The path doesn't usually exist, but we shouldn't overwrite it
                // with an empty config if it did exist.
                ConfigFile::load_or_empty(ConfigSource::User, path)
            })
            .transpose()
    }

    /// Loads user-specific config files into the given `config`. The old
    /// user-config layers will be replaced if any.
    #[instrument]
    pub fn reload_user_config(&self, config: &mut RawConfig) -> Result<(), ConfigLoadError> {
        config.as_mut().remove_layers(ConfigSource::User);
        for path in self.existing_user_config_paths() {
            if path.is_dir() {
                config.as_mut().load_dir(ConfigSource::User, path)?;
            } else {
                config.as_mut().load_file(ConfigSource::User, path)?;
            }
        }
        Ok(())
    }

    /// Sets the directory where repo-specific config file is stored. The path
    /// is usually `.jj/repo`.
    pub fn reset_repo_path(&mut self, path: &Path) {
        self.repo_path = Some(path.to_owned());
        self.repo_config_path = Some(ConfigPath::new(path.join("config.toml")));
    }

    /// Returns a path to the repo-specific config file.
    pub fn repo_config_path(&self) -> Option<&Path> {
        self.repo_config_path.as_ref().map(|p| p.as_path())
    }

    /// Returns a path to the existing repo-specific config file.
    fn existing_repo_config_path(&self) -> Option<&Path> {
        match self.repo_config_path {
            Some(ref path) if path.exists() => Some(path.as_path()),
            _ => None,
        }
    }

    /// Returns repo configuration files for modification. Instantiates one if
    /// `config` has no repo configuration layers.
    ///
    /// If the repo path is unknown, this function returns an empty `Vec`. Since
    /// the repo config path cannot be a directory, the returned `Vec` should
    /// have at most one config file.
    pub fn repo_config_files(
        &self,
        config: &RawConfig,
    ) -> Result<Vec<ConfigFile>, ConfigLoadError> {
        config_files_for(config, ConfigSource::Repo, || self.new_repo_config_file())
    }

    fn new_repo_config_file(&self) -> Result<Option<ConfigFile>, ConfigLoadError> {
        self.repo_config_path()
            // The path doesn't usually exist, but we shouldn't overwrite it
            // with an empty config if it did exist.
            .map(|path| ConfigFile::load_or_empty(ConfigSource::Repo, path))
            .transpose()
    }

    /// Loads repo-specific config file into the given `config`. The old
    /// repo-config layer will be replaced if any.
    #[instrument]
    pub fn reload_repo_config(&self, config: &mut RawConfig) -> Result<(), ConfigLoadError> {
        config.as_mut().remove_layers(ConfigSource::Repo);
        if let Some(path) = self.existing_repo_config_path() {
            config.as_mut().load_file(ConfigSource::Repo, path)?;
        }
        Ok(())
    }

    /// Resolves conditional scopes within the current environment. Returns new
    /// resolved config.
    pub fn resolve_config(&self, config: &RawConfig) -> Result<StackedConfig, ConfigGetError> {
        let context = ConfigResolutionContext {
            home_dir: self.home_dir.as_deref(),
            repo_path: self.repo_path.as_deref(),
            command: self.command.as_deref(),
        };
        jj_lib::config::resolve(config.as_ref(), &context)
    }
}

fn config_files_for(
    config: &RawConfig,
    source: ConfigSource,
    new_file: impl FnOnce() -> Result<Option<ConfigFile>, ConfigLoadError>,
) -> Result<Vec<ConfigFile>, ConfigLoadError> {
    let mut files = config
        .as_ref()
        .layers_for(source)
        .iter()
        .filter_map(|layer| ConfigFile::from_layer(layer.clone()).ok())
        .collect_vec();
    if files.is_empty() {
        files.extend(new_file()?);
    }
    Ok(files)
}

/// Initializes stacked config with the given `default_layers` and infallible
/// sources.
///
/// Sources from the lowest precedence:
/// 1. Default
/// 2. Base environment variables
/// 3. [User configs](https://jj-vcs.github.io/jj/latest/config/)
/// 4. Repo config `.jj/repo/config.toml`
/// 5. TODO: Workspace config `.jj/config.toml`
/// 6. Override environment variables
/// 7. Command-line arguments `--config`, `--config-toml`, `--config-file`
///
/// This function sets up 1, 2, and 6.
pub fn config_from_environment(default_layers: impl IntoIterator<Item = ConfigLayer>) -> RawConfig {
    let mut config = StackedConfig::with_defaults();
    config.extend_layers(default_layers);
    config.add_layer(env_base_layer());
    config.add_layer(env_overrides_layer());
    RawConfig(config)
}

const OP_HOSTNAME: &str = "operation.hostname";
const OP_USERNAME: &str = "operation.username";

/// Environment variables that should be overridden by config values
fn env_base_layer() -> ConfigLayer {
    let mut layer = ConfigLayer::empty(ConfigSource::EnvBase);
    if let Ok(value) = whoami::fallible::hostname()
        .inspect_err(|err| tracing::warn!(?err, "failed to get hostname"))
    {
        layer.set_value(OP_HOSTNAME, value).unwrap();
    }
    if let Ok(value) = whoami::fallible::username()
        .inspect_err(|err| tracing::warn!(?err, "failed to get username"))
    {
        layer.set_value(OP_USERNAME, value).unwrap();
    } else if let Ok(value) = env::var("USER") {
        // On Unix, $USER is set by login(1). Use it as a fallback because
        // getpwuid() of musl libc appears not (fully?) supporting nsswitch.
        layer.set_value(OP_USERNAME, value).unwrap();
    }
    if !env::var("NO_COLOR").unwrap_or_default().is_empty() {
        // "User-level configuration files and per-instance command-line arguments
        // should override $NO_COLOR." https://no-color.org/
        layer.set_value("ui.color", "never").unwrap();
    }
    if let Ok(value) = env::var("PAGER") {
        layer.set_value("ui.pager", value).unwrap();
    }
    if let Ok(value) = env::var("VISUAL") {
        layer.set_value("ui.editor", value).unwrap();
    } else if let Ok(value) = env::var("EDITOR") {
        layer.set_value("ui.editor", value).unwrap();
    }
    layer
}

pub fn default_config_layers() -> Vec<ConfigLayer> {
    // Syntax error in default config isn't a user error. That's why defaults are
    // loaded by separate builder.
    let parse = |text: &'static str| ConfigLayer::parse(ConfigSource::Default, text).unwrap();
    let mut layers = vec![
        parse(include_str!("config/colors.toml")),
        parse(include_str!("config/hints.toml")),
        parse(include_str!("config/merge_tools.toml")),
        parse(include_str!("config/misc.toml")),
        parse(include_str!("config/revsets.toml")),
        parse(include_str!("config/templates.toml")),
    ];
    if cfg!(unix) {
        layers.push(parse(include_str!("config/unix.toml")));
    }
    if cfg!(windows) {
        layers.push(parse(include_str!("config/windows.toml")));
    }
    layers
}

/// Environment variables that override config values
fn env_overrides_layer() -> ConfigLayer {
    let mut layer = ConfigLayer::empty(ConfigSource::EnvOverrides);
    if let Ok(value) = env::var("JJ_USER") {
        layer.set_value("user.name", value).unwrap();
    }
    if let Ok(value) = env::var("JJ_EMAIL") {
        layer.set_value("user.email", value).unwrap();
    }
    if let Ok(value) = env::var("JJ_TIMESTAMP") {
        layer.set_value("debug.commit-timestamp", value).unwrap();
    }
    if let Ok(Ok(value)) = env::var("JJ_RANDOMNESS_SEED").map(|s| s.parse::<i64>()) {
        layer.set_value("debug.randomness-seed", value).unwrap();
    }
    if let Ok(value) = env::var("JJ_OP_TIMESTAMP") {
        layer.set_value("debug.operation-timestamp", value).unwrap();
    }
    if let Ok(value) = env::var("JJ_OP_HOSTNAME") {
        layer.set_value(OP_HOSTNAME, value).unwrap();
    }
    if let Ok(value) = env::var("JJ_OP_USERNAME") {
        layer.set_value(OP_USERNAME, value).unwrap();
    }
    if let Ok(value) = env::var("JJ_EDITOR") {
        layer.set_value("ui.editor", value).unwrap();
    }
    layer
}

/// Configuration source/data type provided as command-line argument.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigArgKind {
    /// `--config=NAME=VALUE`
    Item,
    /// `--config-toml=TOML`
    Toml,
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
            ConfigArgKind::Toml => {
                for (_, text) in chunk {
                    layers.push(ConfigLayer::parse(source, text)?);
                }
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
        // TODO: Delete in jj 0.32+
        ConfigMigrationRule::rename_value("git.auto-local-branch", "git.auto-local-bookmark"),
        // TODO: Delete in jj 0.33+
        ConfigMigrationRule::rename_update_value(
            "signing.sign-all",
            "signing.behavior",
            |old_value| {
                if old_value
                    .as_bool()
                    .ok_or("signing.sign-all expects a boolean")?
                {
                    Ok("own".into())
                } else {
                    Ok("keep".into())
                }
            },
        ),
        // TODO: Delete in jj 0.34+
        ConfigMigrationRule::rename_value(
            "core.watchman.register_snapshot_trigger",
            "core.watchman.register-snapshot-trigger",
        ),
        // TODO: Delete in jj 0.34+
        ConfigMigrationRule::rename_value("diff.format", "ui.diff.format"),
        // TODO: Delete in jj 0.35.0+
        ConfigMigrationRule::rename_update_value(
            "ui.default-description",
            "template-aliases.default_commit_description",
            |old_value| {
                let value = old_value.as_str().ok_or("expected a string")?;
                // Trailing newline would be padded by templater
                let value = text_util::complete_newline(value);
                let escaped = dsl_util::escape_string(&value);
                Ok(format!(r#""{escaped}""#).into())
            },
        ),
    ]
}

/// Command name and arguments specified by config.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(untagged)]
pub enum CommandNameAndArgs {
    String(String),
    Vec(NonEmptyCommandArgsVec),
    Structured {
        env: HashMap<String, String>,
        command: NonEmptyCommandArgsVec,
    },
}

impl CommandNameAndArgs {
    /// Returns command name without arguments.
    pub fn split_name(&self) -> Cow<str> {
        let (name, _) = self.split_name_and_args();
        name
    }

    /// Returns command name and arguments.
    ///
    /// The command name may be an empty string (as well as each argument.)
    pub fn split_name_and_args(&self) -> (Cow<str>, Cow<[String]>) {
        match self {
            CommandNameAndArgs::String(s) => {
                // Handle things like `EDITOR=emacs -nw` (TODO: parse shell escapes)
                let mut args = s.split(' ').map(|s| s.to_owned());
                (args.next().unwrap().into(), args.collect())
            }
            CommandNameAndArgs::Vec(NonEmptyCommandArgsVec(a)) => {
                (Cow::Borrowed(&a[0]), Cow::Borrowed(&a[1..]))
            }
            CommandNameAndArgs::Structured {
                env: _,
                command: cmd,
            } => (Cow::Borrowed(&cmd.0[0]), Cow::Borrowed(&cmd.0[1..])),
        }
    }

    /// Returns process builder configured with this.
    pub fn to_command(&self) -> Command {
        let empty: HashMap<&str, &str> = HashMap::new();
        self.to_command_with_variables(&empty)
    }

    /// Returns process builder configured with this after interpolating
    /// variables into the arguments.
    pub fn to_command_with_variables<V: AsRef<str>>(
        &self,
        variables: &HashMap<&str, V>,
    ) -> Command {
        let (name, args) = self.split_name_and_args();
        let mut cmd = Command::new(name.as_ref());
        if let CommandNameAndArgs::Structured { env, .. } = self {
            cmd.envs(env);
        }
        cmd.args(interpolate_variables(&args, variables));
        cmd
    }
}

impl<T: AsRef<str> + ?Sized> From<&T> for CommandNameAndArgs {
    fn from(s: &T) -> Self {
        CommandNameAndArgs::String(s.as_ref().to_owned())
    }
}

impl fmt::Display for CommandNameAndArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandNameAndArgs::String(s) => write!(f, "{s}"),
            // TODO: format with shell escapes
            CommandNameAndArgs::Vec(a) => write!(f, "{}", a.0.join(" ")),
            CommandNameAndArgs::Structured { env, command } => {
                for (k, v) in env {
                    write!(f, "{k}={v} ")?;
                }
                write!(f, "{}", command.0.join(" "))
            }
        }
    }
}

// Not interested in $UPPER_CASE_VARIABLES
static VARIABLE_REGEX: once_cell::sync::Lazy<Regex> =
    once_cell::sync::Lazy::new(|| Regex::new(r"\$([a-z0-9_]+)\b").unwrap());

pub fn interpolate_variables<V: AsRef<str>>(
    args: &[String],
    variables: &HashMap<&str, V>,
) -> Vec<String> {
    args.iter()
        .map(|arg| {
            VARIABLE_REGEX
                .replace_all(arg, |caps: &Captures| {
                    let name = &caps[1];
                    if let Some(subst) = variables.get(name) {
                        subst.as_ref().to_owned()
                    } else {
                        caps[0].to_owned()
                    }
                })
                .into_owned()
        })
        .collect()
}

/// Return all variable names found in the args, without the dollar sign
pub fn find_all_variables(args: &[String]) -> impl Iterator<Item = &str> {
    let regex = &*VARIABLE_REGEX;
    args.iter()
        .flat_map(|arg| regex.find_iter(arg))
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
            Ok(NonEmptyCommandArgsVec(args))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::env::join_paths;
    use std::fmt::Write as _;

    use indoc::indoc;
    use maplit::hashmap;
    use test_case::test_case;

    use super::*;

    fn insta_settings() -> insta::Settings {
        let mut settings = insta::Settings::clone_current();
        // Suppress Decor { .. } which is uninteresting
        settings.add_filter(r"\bDecor \{[^}]*\}", "Decor { .. }");
        settings
    }

    #[test]
    fn test_parse_value_or_bare_string() {
        let parse = |s: &str| parse_value_or_bare_string(s);

        // Value in TOML syntax
        assert_eq!(parse("true").unwrap().as_bool(), Some(true));
        assert_eq!(parse("42").unwrap().as_integer(), Some(42));
        assert_eq!(parse("-1").unwrap().as_integer(), Some(-1));
        assert_eq!(parse("'a'").unwrap().as_str(), Some("a"));
        assert!(parse("[]").unwrap().is_array());
        assert!(parse("{ a = 'b' }").unwrap().is_inline_table());

        // Bare string
        assert_eq!(parse("").unwrap().as_str(), Some(""));
        assert_eq!(parse("John Doe").unwrap().as_str(), Some("John Doe"));
        assert_eq!(parse("Doe, John").unwrap().as_str(), Some("Doe, John"));
        assert_eq!(parse("It's okay").unwrap().as_str(), Some("It's okay"));
        assert_eq!(
            parse("<foo+bar@example.org>").unwrap().as_str(),
            Some("<foo+bar@example.org>")
        );
        assert_eq!(parse("#ff00aa").unwrap().as_str(), Some("#ff00aa"));
        assert_eq!(parse("all()").unwrap().as_str(), Some("all()"));
        assert_eq!(parse("glob:*.*").unwrap().as_str(), Some("glob:*.*"));
        assert_eq!(parse("柔術").unwrap().as_str(), Some("柔術"));

        // Error in TOML value
        assert!(parse("'foo").is_err());
        assert!(parse(r#" bar" "#).is_err());
        assert!(parse("[0 1]").is_err());
        assert!(parse("{ x = }").is_err());
        assert!(parse("\n { x").is_err());
        assert!(parse(" x ] ").is_err());
        assert!(parse("[table]\nkey = 'value'").is_err());
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
    fn test_command_args() {
        let mut config = StackedConfig::empty();
        config.add_layer(
            ConfigLayer::parse(
                ConfigSource::User,
                indoc! {"
                    empty_array = []
                    empty_string = ''
                    array = ['emacs', '-nw']
                    string = 'emacs -nw'
                    structured.env = { KEY1 = 'value1', KEY2 = 'value2' }
                    structured.command = ['emacs', '-nw']
                "},
            )
            .unwrap(),
        );

        assert!(config.get::<CommandNameAndArgs>("empty_array").is_err());

        let command_args: CommandNameAndArgs = config.get("empty_string").unwrap();
        assert_eq!(command_args, CommandNameAndArgs::String("".to_owned()));
        let (name, args) = command_args.split_name_and_args();
        assert_eq!(name, "");
        assert!(args.is_empty());

        let command_args: CommandNameAndArgs = config.get("array").unwrap();
        assert_eq!(
            command_args,
            CommandNameAndArgs::Vec(NonEmptyCommandArgsVec(
                ["emacs", "-nw",].map(|s| s.to_owned()).to_vec()
            ))
        );
        let (name, args) = command_args.split_name_and_args();
        assert_eq!(name, "emacs");
        assert_eq!(args, ["-nw"].as_ref());

        let command_args: CommandNameAndArgs = config.get("string").unwrap();
        assert_eq!(
            command_args,
            CommandNameAndArgs::String("emacs -nw".to_owned())
        );
        let (name, args) = command_args.split_name_and_args();
        assert_eq!(name, "emacs");
        assert_eq!(args, ["-nw"].as_ref());

        let command_args: CommandNameAndArgs = config.get("structured").unwrap();
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
    }

    #[test]
    fn test_resolved_config_values_empty() {
        let config = StackedConfig::empty();
        assert!(resolved_config_values(&config, &ConfigNamePathBuf::root()).is_empty());
    }

    #[test]
    fn test_resolved_config_values_single_key() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();
        let mut env_base_layer = ConfigLayer::empty(ConfigSource::EnvBase);
        env_base_layer
            .set_value("user.name", "base-user-name")
            .unwrap();
        env_base_layer
            .set_value("user.email", "base@user.email")
            .unwrap();
        let mut repo_layer = ConfigLayer::empty(ConfigSource::Repo);
        repo_layer
            .set_value("user.email", "repo@user.email")
            .unwrap();
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
    }

    #[test]
    fn test_resolved_config_values_filter_path() {
        let settings = insta_settings();
        let _guard = settings.bind_to_scope();
        let mut user_layer = ConfigLayer::empty(ConfigSource::User);
        user_layer.set_value("test-table1.foo", "user-FOO").unwrap();
        user_layer.set_value("test-table2.bar", "user-BAR").unwrap();
        let mut repo_layer = ConfigLayer::empty(ConfigSource::Repo);
        repo_layer.set_value("test-table1.bar", "repo-BAR").unwrap();
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
    }

    #[test]
    fn test_resolved_config_values_overridden() {
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
        layer0.set_value("a.b.e", "0.0").unwrap();
        layer0.set_value("a.b.c.f", "0.1").unwrap();
        layer0.set_value("a.b.d", "0.2").unwrap();
        let mut layer1 = ConfigLayer::empty(ConfigSource::User);
        layer1.set_value("a.b", "1.0").unwrap();
        layer1.set_value("a.c", "1.1").unwrap();
        let mut layer2 = ConfigLayer::empty(ConfigSource::User);
        layer2.set_value("a.b.g", "2.0").unwrap();
        layer2.set_value("a.b.d", "2.1").unwrap();

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
    }

    struct TestCase {
        files: &'static [&'static str],
        env: UnresolvedConfigEnv,
        wants: Vec<Want>,
    }

    #[derive(Debug)]
    enum WantState {
        New,
        Existing,
    }
    #[derive(Debug)]
    struct Want {
        path: &'static str,
        state: WantState,
    }

    impl Want {
        const fn new(path: &'static str) -> Want {
            Want {
                path,
                state: WantState::New,
            }
        }

        const fn existing(path: &'static str) -> Want {
            Want {
                path,
                state: WantState::Existing,
            }
        }

        fn rooted_path(&self, root: &Path) -> PathBuf {
            root.join(self.path)
        }

        fn exists(&self) -> bool {
            matches!(self.state, WantState::Existing)
        }
    }

    fn config_path_home_existing() -> TestCase {
        TestCase {
            files: &["home/.jjconfig.toml"],
            env: UnresolvedConfigEnv {
                home_dir: Some("home".into()),
                ..Default::default()
            },
            wants: vec![Want::existing("home/.jjconfig.toml")],
        }
    }

    fn config_path_home_new() -> TestCase {
        TestCase {
            files: &[],
            env: UnresolvedConfigEnv {
                home_dir: Some("home".into()),
                ..Default::default()
            },
            wants: vec![Want::new("home/.jjconfig.toml")],
        }
    }

    fn config_path_home_existing_platform_new() -> TestCase {
        TestCase {
            files: &["home/.jjconfig.toml"],
            env: UnresolvedConfigEnv {
                home_dir: Some("home".into()),
                config_dir: Some("config".into()),
                ..Default::default()
            },
            wants: vec![
                Want::existing("home/.jjconfig.toml"),
                Want::new("config/jj/config.toml"),
            ],
        }
    }

    fn config_path_platform_existing() -> TestCase {
        TestCase {
            files: &["config/jj/config.toml"],
            env: UnresolvedConfigEnv {
                home_dir: Some("home".into()),
                config_dir: Some("config".into()),
                ..Default::default()
            },
            wants: vec![Want::existing("config/jj/config.toml")],
        }
    }

    fn config_path_platform_new() -> TestCase {
        TestCase {
            files: &[],
            env: UnresolvedConfigEnv {
                config_dir: Some("config".into()),
                ..Default::default()
            },
            wants: vec![Want::new("config/jj/config.toml")],
        }
    }

    fn config_path_new_prefer_platform() -> TestCase {
        TestCase {
            files: &[],
            env: UnresolvedConfigEnv {
                home_dir: Some("home".into()),
                config_dir: Some("config".into()),
                ..Default::default()
            },
            wants: vec![Want::new("config/jj/config.toml")],
        }
    }

    fn config_path_jj_config_existing() -> TestCase {
        TestCase {
            files: &["custom.toml"],
            env: UnresolvedConfigEnv {
                jj_config: Some("custom.toml".into()),
                ..Default::default()
            },
            wants: vec![Want::existing("custom.toml")],
        }
    }

    fn config_path_jj_config_new() -> TestCase {
        TestCase {
            files: &[],
            env: UnresolvedConfigEnv {
                jj_config: Some("custom.toml".into()),
                ..Default::default()
            },
            wants: vec![Want::new("custom.toml")],
        }
    }

    fn config_path_jj_config_existing_multiple() -> TestCase {
        TestCase {
            files: &["custom1.toml", "custom2.toml"],
            env: UnresolvedConfigEnv {
                jj_config: Some(
                    join_paths(["custom1.toml", "custom2.toml"])
                        .unwrap()
                        .into_string()
                        .unwrap(),
                ),
                ..Default::default()
            },
            wants: vec![
                Want::existing("custom1.toml"),
                Want::existing("custom2.toml"),
            ],
        }
    }

    fn config_path_jj_config_new_multiple() -> TestCase {
        TestCase {
            files: &["custom1.toml"],
            env: UnresolvedConfigEnv {
                jj_config: Some(
                    join_paths(["custom1.toml", "custom2.toml"])
                        .unwrap()
                        .into_string()
                        .unwrap(),
                ),
                ..Default::default()
            },
            wants: vec![Want::existing("custom1.toml"), Want::new("custom2.toml")],
        }
    }

    fn config_path_jj_config_empty_paths_filtered() -> TestCase {
        TestCase {
            files: &["custom1.toml"],
            env: UnresolvedConfigEnv {
                jj_config: Some(
                    join_paths(["custom1.toml", "", "custom2.toml"])
                        .unwrap()
                        .into_string()
                        .unwrap(),
                ),
                ..Default::default()
            },
            wants: vec![Want::existing("custom1.toml"), Want::new("custom2.toml")],
        }
    }

    fn config_path_jj_config_empty() -> TestCase {
        TestCase {
            files: &[],
            env: UnresolvedConfigEnv {
                jj_config: Some("".to_owned()),
                ..Default::default()
            },
            wants: vec![],
        }
    }

    fn config_path_config_pick_platform() -> TestCase {
        TestCase {
            files: &["config/jj/config.toml"],
            env: UnresolvedConfigEnv {
                home_dir: Some("home".into()),
                config_dir: Some("config".into()),
                ..Default::default()
            },
            wants: vec![Want::existing("config/jj/config.toml")],
        }
    }

    fn config_path_config_pick_home() -> TestCase {
        TestCase {
            files: &["home/.jjconfig.toml"],
            env: UnresolvedConfigEnv {
                home_dir: Some("home".into()),
                config_dir: Some("config".into()),
                ..Default::default()
            },
            wants: vec![
                Want::existing("home/.jjconfig.toml"),
                Want::new("config/jj/config.toml"),
            ],
        }
    }

    fn config_path_platform_new_conf_dir_existing() -> TestCase {
        TestCase {
            files: &["config/jj/conf.d/_"],
            env: UnresolvedConfigEnv {
                home_dir: Some("home".into()),
                config_dir: Some("config".into()),
                ..Default::default()
            },
            wants: vec![
                Want::new("config/jj/config.toml"),
                Want::existing("config/jj/conf.d"),
            ],
        }
    }

    fn config_path_platform_existing_conf_dir_existing() -> TestCase {
        TestCase {
            files: &["config/jj/config.toml", "config/jj/conf.d/_"],
            env: UnresolvedConfigEnv {
                home_dir: Some("home".into()),
                config_dir: Some("config".into()),
                ..Default::default()
            },
            wants: vec![
                Want::existing("config/jj/config.toml"),
                Want::existing("config/jj/conf.d"),
            ],
        }
    }

    fn config_path_all_existing() -> TestCase {
        TestCase {
            files: &[
                "config/jj/conf.d/_",
                "config/jj/config.toml",
                "home/.jjconfig.toml",
            ],
            env: UnresolvedConfigEnv {
                home_dir: Some("home".into()),
                config_dir: Some("config".into()),
                ..Default::default()
            },
            // Precedence order is important
            wants: vec![
                Want::existing("home/.jjconfig.toml"),
                Want::existing("config/jj/config.toml"),
                Want::existing("config/jj/conf.d"),
            ],
        }
    }

    fn config_path_none() -> TestCase {
        TestCase {
            files: &[],
            env: Default::default(),
            wants: vec![],
        }
    }

    fn config_path_macos_legacy_exists() -> TestCase {
        TestCase {
            files: &["macos-legacy/jj/config.toml"],
            env: UnresolvedConfigEnv {
                home_dir: Some("home".into()),
                config_dir: Some("config".into()),
                macos_legacy_config_dir: Some("macos-legacy".into()),
                ..Default::default()
            },
            wants: vec![
                Want::new("config/jj/config.toml"),
                Want::existing("macos-legacy/jj/config.toml"),
            ],
        }
    }

    fn config_path_macos_legacy_both_exist() -> TestCase {
        TestCase {
            files: &["macos-legacy/jj/config.toml", "config/jj/config.toml"],
            env: UnresolvedConfigEnv {
                home_dir: Some("home".into()),
                config_dir: Some("config".into()),
                macos_legacy_config_dir: Some("macos-legacy".into()),
                ..Default::default()
            },
            wants: vec![
                Want::existing("config/jj/config.toml"),
                Want::existing("macos-legacy/jj/config.toml"),
            ],
        }
    }

    fn config_path_macos_legacy_new() -> TestCase {
        TestCase {
            files: &[],
            env: UnresolvedConfigEnv {
                home_dir: Some("home".into()),
                config_dir: Some("config".into()),
                macos_legacy_config_dir: Some("macos-legacy".into()),
                ..Default::default()
            },
            wants: vec![Want::new("config/jj/config.toml")],
        }
    }

    #[test_case(config_path_home_existing())]
    #[test_case(config_path_home_new())]
    #[test_case(config_path_home_existing_platform_new())]
    #[test_case(config_path_platform_existing())]
    #[test_case(config_path_platform_new())]
    #[test_case(config_path_new_prefer_platform())]
    #[test_case(config_path_jj_config_existing())]
    #[test_case(config_path_jj_config_new())]
    #[test_case(config_path_jj_config_existing_multiple())]
    #[test_case(config_path_jj_config_new_multiple())]
    #[test_case(config_path_jj_config_empty_paths_filtered())]
    #[test_case(config_path_jj_config_empty())]
    #[test_case(config_path_config_pick_platform())]
    #[test_case(config_path_config_pick_home())]
    #[test_case(config_path_platform_new_conf_dir_existing())]
    #[test_case(config_path_platform_existing_conf_dir_existing())]
    #[test_case(config_path_all_existing())]
    #[test_case(config_path_none())]
    #[test_case(config_path_macos_legacy_exists())]
    #[test_case(config_path_macos_legacy_both_exist())]
    #[test_case(config_path_macos_legacy_new())]
    fn test_config_path(case: TestCase) {
        let tmp = setup_config_fs(case.files);
        let env = resolve_config_env(&case.env, tmp.path());

        let all_expected_paths = case
            .wants
            .iter()
            .map(|w| w.rooted_path(tmp.path()))
            .collect_vec();
        let exists_expected_paths = case
            .wants
            .iter()
            .filter(|w| w.exists())
            .map(|w| w.rooted_path(tmp.path()))
            .collect_vec();

        let all_paths = env.user_config_paths().collect_vec();
        let exists_paths = env.existing_user_config_paths().collect_vec();

        assert_eq!(all_paths, all_expected_paths);
        assert_eq!(exists_paths, exists_expected_paths);
    }

    fn setup_config_fs(files: &[&str]) -> tempfile::TempDir {
        let tmp = testutils::new_temp_dir();
        for file in files {
            let path = tmp.path().join(file);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::File::create(path).unwrap();
        }
        tmp
    }

    fn resolve_config_env(env: &UnresolvedConfigEnv, root: &Path) -> ConfigEnv {
        let home_dir = env.home_dir.as_ref().map(|p| root.join(p));
        let env = UnresolvedConfigEnv {
            config_dir: env.config_dir.as_ref().map(|p| root.join(p)),
            macos_legacy_config_dir: env.macos_legacy_config_dir.as_ref().map(|p| root.join(p)),
            home_dir: home_dir.clone(),
            jj_config: env.jj_config.as_ref().map(|p| {
                join_paths(split_paths(p).map(|p| {
                    if p.as_os_str().is_empty() {
                        return p;
                    }
                    root.join(p)
                }))
                .unwrap()
                .into_string()
                .unwrap()
            }),
        };
        ConfigEnv {
            home_dir,
            repo_path: None,
            user_config_paths: env.resolve(&Ui::null()),
            repo_config_path: None,
            command: None,
        }
    }
}
