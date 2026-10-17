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

//! Locating and loading configuration files.

use std::collections::HashMap;
use std::env;
use std::env::split_paths;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use etcetera::BaseStrategy as _;
use itertools::Itertools as _;
use jj_lib::config::ConfigFile;
use jj_lib::config::ConfigGetError;
use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigLoadError;
use jj_lib::config::ConfigResolutionContext;
use jj_lib::config::ConfigSource;
use jj_lib::config::StackedConfig;
use jj_lib::secure_config::LoadedSecureConfig;
use jj_lib::secure_config::SecureConfig;
use jj_lib::secure_config::SecureConfigError;
use rand::SeedableRng as _;
use rand_chacha::ChaCha20Rng;
use thiserror::Error;
use tracing::instrument;

use crate::config_util::ConfigWarning;

/// Error loading repo- or workspace-specific configuration.
#[derive(Debug, Error)]
pub enum ConfigEnvError {
    /// The location of the config file couldn't be determined.
    #[error(transparent)]
    SecureConfig(#[from] SecureConfigError),
    /// The config file couldn't be loaded.
    #[error(transparent)]
    Load(#[from] ConfigLoadError),
}

const REPO_CONFIG_DIR: &str = "repos";
const WORKSPACE_CONFIG_DIR: &str = "workspaces";

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
        Self {
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
    user_config_dir: Option<PathBuf>,
    home_dir: Option<PathBuf>,
    jj_config: Option<String>,
    system_config_dir: Option<PathBuf>,
}

impl UnresolvedConfigEnv {
    fn root_config_dir(&self) -> Option<PathBuf> {
        self.user_config_dir.as_deref().map(|c| c.join("jj"))
    }

    fn resolve_user(self) -> Vec<ConfigPath> {
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
        let platform_config_path = self.user_config_dir.clone().map(|mut config_dir| {
            config_dir.push("jj");
            config_dir.push("config.toml");
            ConfigPath::new(config_dir)
        });
        let platform_config_dir = self.user_config_dir.map(|mut config_dir| {
            config_dir.push("jj");
            config_dir.push("conf.d");
            ConfigPath::new(config_dir)
        });

        if let Some(path) = home_config_path
            && (path.exists() || platform_config_path.is_none())
        {
            paths.push(path);
        }

        // This should be the default config created if there's
        // no user config and `jj config edit` is executed.
        if let Some(path) = platform_config_path {
            paths.push(path);
        }

        if let Some(path) = platform_config_dir
            && path.exists()
        {
            paths.push(path);
        }

        paths
    }

    fn resolve_system(&self) -> Vec<ConfigPath> {
        if let Some(path) = self.system_config_dir.as_ref()
            && self.jj_config.is_none()
        {
            [path.join("jj/config.toml"), path.join("jj/conf.d")]
                .into_iter()
                .map(ConfigPath::new)
                .collect()
        } else {
            Vec::new()
        }
    }
}

/// Locates configuration files and holds the context for resolving
/// conditional configuration, based on the process environment and the
/// current repo and workspace.
#[derive(Clone, Debug)]
pub struct ConfigEnv {
    home_dir: Option<PathBuf>,
    root_config_dir: Option<PathBuf>,
    repo_path: Option<PathBuf>,
    workspace_path: Option<PathBuf>,
    system_config_paths: Vec<ConfigPath>,
    user_config_paths: Vec<ConfigPath>,
    repo_config: Option<SecureConfig>,
    workspace_config: Option<SecureConfig>,
    command: Option<String>,
    hostname: Option<String>,
    environment: HashMap<String, String>,
    rng: Arc<Mutex<ChaCha20Rng>>,
}

impl ConfigEnv {
    /// Initializes configuration loader based on environment variables.
    pub fn from_environment() -> Self {
        let user_config_dir = etcetera::choose_base_strategy()
            .ok()
            .map(|s| s.config_dir());

        // Canonicalize home so it can be compared with canonicalized paths such
        // as the cwd in the CLI. $HOME might point to symlink.
        let home_dir = etcetera::home_dir()
            .ok()
            .map(|d| dunce::canonicalize(&d).unwrap_or(d));

        let system_config_dir = if cfg!(unix) {
            Some("/etc".into())
        } else {
            None
        };

        let env = UnresolvedConfigEnv {
            user_config_dir,
            home_dir: home_dir.clone(),
            jj_config: env::var("JJ_CONFIG").ok(),
            system_config_dir,
        };
        let environment = env::vars_os()
            .filter_map(|(k, v)| {
                // Silently ignore non-Unicode environment variables. Don't panic like vars()
                let k = k.into_string().ok()?;
                let v = v.into_string().ok()?;
                Some((k, v))
            })
            .collect();
        Self {
            home_dir,
            root_config_dir: env.root_config_dir(),
            repo_path: None,
            workspace_path: None,
            system_config_paths: env.resolve_system(),
            user_config_paths: env.resolve_user(),
            repo_config: None,
            workspace_config: None,
            command: None,
            hostname: whoami::hostname().ok(),
            environment,
            // We would ideally use JjRng, but that requires the seed from the
            // config, which requires the config to be loaded.
            rng: Arc::new(Mutex::new(
                if let Ok(Ok(value)) = env::var("JJ_RANDOMNESS_SEED").map(|s| s.parse::<u64>()) {
                    ChaCha20Rng::seed_from_u64(value)
                } else {
                    rand::make_rng()
                },
            )),
        }
    }

    /// Returns the user's home directory, if known.
    pub fn home_dir(&self) -> Option<&Path> {
        self.home_dir.as_deref()
    }

    /// Sets the name of the command being run, which conditional config can
    /// match on.
    pub fn set_command_name(&mut self, command: String) {
        self.command = Some(command);
    }

    /// Loads system-wide config files into the given `config`. The old
    /// system-config layers will be replaced if any.
    #[instrument]
    pub fn reload_system_config(&self, config: &mut RawConfig) -> Result<(), ConfigLoadError> {
        config.as_mut().remove_layers(ConfigSource::System);
        for path in self.existing_system_config_paths() {
            if path.is_dir() {
                config.as_mut().load_dir(ConfigSource::System, path)?;
            } else {
                config.as_mut().load_file(ConfigSource::System, path)?;
            }
        }
        Ok(())
    }

    /// Returns the paths to the existing system-wide config files or
    /// directories.
    pub fn existing_system_config_paths(&self) -> impl Iterator<Item = &Path> {
        self.system_config_paths
            .iter()
            .filter(|p| p.exists())
            .map(ConfigPath::as_path)
    }

    fn load_secure_config(
        &self,
        warnings: &mut Vec<ConfigWarning>,
        config: Option<&SecureConfig>,
        kind: &str,
        force: bool,
    ) -> Result<Option<LoadedSecureConfig>, SecureConfigError> {
        Ok(match (config, self.root_config_dir.as_ref()) {
            (Some(config), Some(root_config_dir)) => {
                let mut guard = self.rng.lock().unwrap();
                let loaded_config = if force {
                    config.load_config(&mut guard, &root_config_dir.join(kind))
                } else {
                    config.maybe_load_config(&mut guard, &root_config_dir.join(kind))
                }?;
                warnings.extend(loaded_config.warnings.iter().map(|message| {
                    ConfigWarning::SecureConfig {
                        message: message.clone(),
                    }
                }));
                Some(loaded_config)
            }
            _ => None,
        })
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

    /// Sets the directory where the repo-specific config file is stored. The
    /// path is usually `$REPO/.jj/repo`.
    pub fn reset_repo_path(&mut self, path: &Path) {
        self.repo_config = Some(SecureConfig::new_repo(path.to_path_buf()));
        self.repo_path = Some(path.to_owned());
    }

    /// Returns a path to the existing repo-specific config file.
    fn maybe_repo_config_path(
        &self,
        warnings: &mut Vec<ConfigWarning>,
    ) -> Result<Option<PathBuf>, SecureConfigError> {
        Ok(self
            .load_secure_config(warnings, self.repo_config.as_ref(), REPO_CONFIG_DIR, false)?
            .and_then(|c| c.config_file))
    }

    /// Returns a path to the existing repo-specific config file.
    /// If the config file does not exist, will create a new config ID and
    /// create a new directory for this.
    pub fn repo_config_path(
        &self,
        warnings: &mut Vec<ConfigWarning>,
    ) -> Result<Option<PathBuf>, SecureConfigError> {
        Ok(self
            .load_secure_config(warnings, self.repo_config.as_ref(), REPO_CONFIG_DIR, true)?
            .and_then(|c| c.config_file))
    }

    /// Returns the directory under which all repo-specific config
    /// subdirectories (one per config ID) are stored.
    pub fn repo_configs_root_dir(&self) -> Option<PathBuf> {
        self.root_config_dir
            .as_ref()
            .map(|dir| dir.join(REPO_CONFIG_DIR))
    }

    /// Returns repo configuration files for modification. Instantiates one if
    /// `config` has no repo configuration layers.
    ///
    /// If the repo path is unknown, this function returns an empty `Vec`. Since
    /// the repo config path cannot be a directory, the returned `Vec` should
    /// have at most one config file.
    pub fn repo_config_files(
        &self,
        warnings: &mut Vec<ConfigWarning>,
        config: &RawConfig,
    ) -> Result<Vec<ConfigFile>, ConfigEnvError> {
        config_files_for(config, ConfigSource::Repo, || {
            self.new_repo_config_file(warnings)
        })
    }

    fn new_repo_config_file(
        &self,
        warnings: &mut Vec<ConfigWarning>,
    ) -> Result<Option<ConfigFile>, ConfigEnvError> {
        Ok(self
            .repo_config_path(warnings)?
            // The path doesn't usually exist, but we shouldn't overwrite it
            // with an empty config if it did exist.
            .map(|path| ConfigFile::load_or_empty(ConfigSource::Repo, path))
            .transpose()?)
    }

    /// Loads repo-specific config file into the given `config`. The old
    /// repo-config layer will be replaced if any.
    #[instrument(skip(warnings))]
    pub fn reload_repo_config(
        &self,
        warnings: &mut Vec<ConfigWarning>,
        config: &mut RawConfig,
    ) -> Result<(), ConfigEnvError> {
        config.as_mut().remove_layers(ConfigSource::Repo);
        if let Some(path) = self.maybe_repo_config_path(warnings)?
            && path.exists()
        {
            config.as_mut().load_file(ConfigSource::Repo, path)?;
        }
        Ok(())
    }

    /// Sets the directory where the workspace-specific config file is stored.
    pub fn reset_workspace_path(&mut self, path: &Path) {
        self.workspace_config = Some(SecureConfig::new_workspace(path.join(".jj")));
        self.workspace_path = Some(path.to_owned());
    }

    /// Returns a path to the workspace-specific config file, if it exists.
    fn maybe_workspace_config_path(
        &self,
        warnings: &mut Vec<ConfigWarning>,
    ) -> Result<Option<PathBuf>, SecureConfigError> {
        Ok(self
            .load_secure_config(
                warnings,
                self.workspace_config.as_ref(),
                WORKSPACE_CONFIG_DIR,
                false,
            )?
            .and_then(|c| c.config_file))
    }

    /// Returns a path to the existing workspace-specific config file.
    /// If the config file does not exist, will create a new config ID and
    /// create a new directory for this.
    pub fn workspace_config_path(
        &self,
        warnings: &mut Vec<ConfigWarning>,
    ) -> Result<Option<PathBuf>, SecureConfigError> {
        Ok(self
            .load_secure_config(
                warnings,
                self.workspace_config.as_ref(),
                WORKSPACE_CONFIG_DIR,
                true,
            )?
            .and_then(|c| c.config_file))
    }

    /// Returns the directory under which all workspace-specific config
    /// subdirectories (one per config ID) are stored.
    pub fn workspace_configs_root_dir(&self) -> Option<PathBuf> {
        self.root_config_dir
            .as_ref()
            .map(|dir| dir.join(WORKSPACE_CONFIG_DIR))
    }

    /// Returns workspace configuration files for modification. Instantiates one
    /// if `config` has no workspace configuration layers.
    ///
    /// If the workspace path is unknown, this function returns an empty `Vec`.
    /// Since the workspace config path cannot be a directory, the returned
    /// `Vec` should have at most one config file.
    pub fn workspace_config_files(
        &self,
        warnings: &mut Vec<ConfigWarning>,
        config: &RawConfig,
    ) -> Result<Vec<ConfigFile>, ConfigEnvError> {
        config_files_for(config, ConfigSource::Workspace, || {
            self.new_workspace_config_file(warnings)
        })
    }

    fn new_workspace_config_file(
        &self,
        warnings: &mut Vec<ConfigWarning>,
    ) -> Result<Option<ConfigFile>, ConfigEnvError> {
        Ok(self
            .workspace_config_path(warnings)?
            .map(|path| ConfigFile::load_or_empty(ConfigSource::Workspace, path))
            .transpose()?)
    }

    /// Loads workspace-specific config file into the given `config`. The old
    /// workspace-config layer will be replaced if any.
    #[instrument(skip(warnings))]
    pub fn reload_workspace_config(
        &self,
        warnings: &mut Vec<ConfigWarning>,
        config: &mut RawConfig,
    ) -> Result<(), ConfigEnvError> {
        config.as_mut().remove_layers(ConfigSource::Workspace);
        if let Some(path) = self.maybe_workspace_config_path(warnings)?
            && path.exists()
        {
            config.as_mut().load_file(ConfigSource::Workspace, path)?;
        }
        Ok(())
    }

    /// Returns the configuration file at the specified `path` for modification.
    ///
    /// Validates that the path is a recognized Jujutsu configuration location
    /// (such as a loaded layer, a user config path or file in `conf.d/`,
    /// or a repo/workspace config path). If the file is already loaded in
    /// `config`, its existing [`ConfigFile`] representation is reused;
    /// otherwise, a new [`ConfigFile`] is initialized. Returns `None` if the
    /// path isn't a recognized configuration location.
    pub fn resolve_file_to_edit(
        &self,
        warnings: &mut Vec<ConfigWarning>,
        config: &RawConfig,
        path: &Path,
    ) -> Result<Option<ConfigFile>, ConfigLoadError> {
        let canonical_path = dunce::canonicalize(path).ok();
        let matches_path = |p: &Path| p == path || Some(p) == canonical_path.as_deref();

        // 1. Check if it matches an already loaded layer (user, system, repo,
        //    workspace, --config-file, JJ_CONFIG, etc.)
        for layer in config.as_ref().layers() {
            if let Some(layer_path) = layer.path.as_ref()
                && matches_path(layer_path)
                && let Ok(file) = ConfigFile::from_layer(layer.clone())
            {
                return Ok(Some(file));
            }
        }

        // 2. Check repo config path (even if not yet created on disk)
        if let Ok(Some(repo_path)) = self.repo_config_path(warnings)
            && matches_path(&repo_path)
        {
            if let Some(parent) = path.parent() {
                create_dir_all(parent).ok();
            }
            return ConfigFile::load_or_empty(ConfigSource::Repo, path).map(Some);
        }

        // 3. Check workspace config path (even if not yet created on disk)
        if let Ok(Some(workspace_path)) = self.workspace_config_path(warnings)
            && matches_path(&workspace_path)
        {
            if let Some(parent) = path.parent() {
                create_dir_all(parent).ok();
            }
            return ConfigFile::load_or_empty(ConfigSource::Workspace, path).map(Some);
        }

        // 4. Check user config paths (e.g. ~/.config/jj/config.toml,
        //    ~/.jjconfig.toml, or files in conf.d)
        for user_path in self.user_config_paths() {
            if matches_path(user_path) || is_file_in_config_dir(path, user_path) {
                if let Some(parent) = path.parent() {
                    create_dir_all(parent).ok();
                }
                return ConfigFile::load_or_empty(ConfigSource::User, path).map(Some);
            }
        }

        Ok(None)
    }

    /// Resolves conditional scopes within the current environment. Returns new
    /// resolved config.
    pub fn resolve_config(&self, config: &RawConfig) -> Result<StackedConfig, ConfigGetError> {
        let context = ConfigResolutionContext {
            home_dir: self.home_dir.as_deref(),
            repo_path: self.repo_path.as_deref(),
            workspace_path: self.workspace_path.as_deref(),
            command: self.command.as_deref(),
            hostname: self.hostname.as_deref().unwrap_or(""),
            environment: &self.environment,
        };
        jj_lib::config::resolve(config.as_ref(), &context)
    }
}

/// Similar to [`ConfigEnv::repo_config_files()`], but doesn't attempt to
/// initialize new config ID and its storage directory.
pub fn existing_repo_config_file(config: &RawConfig) -> Option<ConfigFile> {
    // There should be at most one repo-level config file.
    config
        .as_ref()
        .layers_for(ConfigSource::Repo)
        .iter()
        .find_map(|layer| ConfigFile::from_layer(layer.clone()).ok())
}

fn config_files_for<E>(
    config: &RawConfig,
    source: ConfigSource,
    new_file: impl FnOnce() -> Result<Option<ConfigFile>, E>,
) -> Result<Vec<ConfigFile>, E> {
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

fn is_file_in_config_dir(file_path: &Path, dir_path: &Path) -> bool {
    if file_path.extension() != Some("toml".as_ref()) {
        return false;
    }
    if dir_path.is_file() {
        return false;
    }
    let Some(parent) = file_path.parent() else {
        return false;
    };
    if parent == dir_path {
        return true;
    }
    dunce::canonicalize(parent).ok().as_deref() == Some(dir_path)
}

/// Initializes stacked config with the given `default_layers` and infallible
/// sources.
///
/// Sources from the lowest precedence:
/// 1. Default
/// 2. System config
/// 3. Base environment variables
/// 4. [User configs](https://docs.jj-vcs.dev/latest/config/)
/// 5. Repo config
/// 6. Workspace config
/// 7. Override environment variables
/// 8. Overrides from the user interface, such as the CLI's `--config` and
///    `--config-file` arguments
///
/// This function sets up 1, 3, and 7.
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
    if let Ok(value) =
        whoami::hostname().inspect_err(|err| tracing::warn!(?err, "failed to get hostname"))
    {
        layer.set_value(OP_HOSTNAME, value).unwrap();
    }
    if let Ok(value) =
        whoami::username().inspect_err(|err| tracing::warn!(?err, "failed to get username"))
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
    if let Ok(value) = env::var("VISUAL") {
        layer.set_value("ui.editor", value).unwrap();
    } else if let Ok(value) = env::var("EDITOR") {
        layer.set_value("ui.editor", value).unwrap();
    }
    // Intentionally NOT respecting $PAGER here as it often creates a bad
    // out-of-the-box experience for users, see http://github.com/jj-vcs/jj/issues/3502.
    layer
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
    if let Ok(value) = env::var("JJ_PAGER") {
        layer.set_value("ui.pager", value).unwrap();
    }
    layer
}

#[cfg(test)]
mod tests {
    use std::env::join_paths;

    use test_case::test_case;

    use super::*;

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
        const fn new(path: &'static str) -> Self {
            Self {
                path,
                state: WantState::New,
            }
        }

        const fn existing(path: &'static str) -> Self {
            Self {
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
                user_config_dir: Some("config".into()),
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
                user_config_dir: Some("config".into()),
                ..Default::default()
            },
            wants: vec![Want::existing("config/jj/config.toml")],
        }
    }

    fn config_path_platform_new() -> TestCase {
        TestCase {
            files: &[],
            env: UnresolvedConfigEnv {
                user_config_dir: Some("config".into()),
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
                user_config_dir: Some("config".into()),
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
                user_config_dir: Some("config".into()),
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
                user_config_dir: Some("config".into()),
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
                user_config_dir: Some("config".into()),
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
                user_config_dir: Some("config".into()),
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
                user_config_dir: Some("config".into()),
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

    fn system_config_path_none() -> TestCase {
        TestCase {
            files: &["etc/jj/config.toml", "system/jj/conf.d/_"],
            env: Default::default(),
            wants: vec![],
        }
    }

    fn system_config_path_existing() -> TestCase {
        TestCase {
            files: &["system/jj/config.toml", "system/jj/conf.d/_"],
            env: UnresolvedConfigEnv {
                system_config_dir: Some("system".into()),
                ..Default::default()
            },
            wants: vec![
                Want::existing("system/jj/config.toml"),
                Want::existing("system/jj/conf.d"),
            ],
        }
    }

    fn system_config_path_jj_config() -> TestCase {
        TestCase {
            files: &["system/jj/config.toml"],
            env: UnresolvedConfigEnv {
                jj_config: Some("custom.toml".into()),
                system_config_dir: Some("system".into()),
                ..Default::default()
            },
            wants: vec![],
        }
    }

    #[test_case(system_config_path_none())]
    #[test_case(system_config_path_existing())]
    #[test_case(system_config_path_jj_config())]
    fn test_system_config_path(case: TestCase) {
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

        let all_paths = env
            .system_config_paths
            .iter()
            .map(ConfigPath::as_path)
            .collect_vec();
        let exists_paths = env.existing_system_config_paths().collect_vec();

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
            user_config_dir: env.user_config_dir.as_ref().map(|p| root.join(p)),
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
            system_config_dir: env.system_config_dir.as_ref().map(|p| root.join(p)),
        };
        ConfigEnv {
            home_dir,
            root_config_dir: None,
            repo_path: None,
            workspace_path: None,
            system_config_paths: env.resolve_system(),
            user_config_paths: env.resolve_user(),
            repo_config: None,
            workspace_config: None,
            command: None,
            hostname: None,
            environment: HashMap::new(),
            rng: Arc::new(Mutex::new(ChaCha20Rng::seed_from_u64(0))),
        }
    }
}
