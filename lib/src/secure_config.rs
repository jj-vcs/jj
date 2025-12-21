// Copyright 2025 The Jujutsu Authors
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

//! A mechanism to access config files for a repo securely.

use std::cell::RefCell;
use std::fs;
use std::io::ErrorKind::NotFound;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;

use prost::Message as _;
use rand::RngExt as _;
use rand_chacha::ChaCha20Rng;
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::file_util::BadPathEncoding;
use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;
use crate::file_util::path_from_bytes;
use crate::file_util::path_to_bytes;
use crate::hex_util::encode_hex;
use crate::protos::secure_config::ConfigMetadata;

const CONFIG_FILE: &str = "config.toml";
const METADATA_FILE: &str = "metadata.binpb";
const CONFIG_ID_BYTES: usize = 10;
#[cfg(not(unix))]
const CONTENT_PREFIX: &str = r###"# DO NOT EDIT.
# This file is for old versions of jj.
# It will be used for jj >= v0.37.
# Use `jj config path` or `jj config edit` to find and edit the new file

"###;
const CONFIG_NOT_FOUND: &str = r###"Per-repo config not found. Generating an empty one.
Per-repo config is stored in the same directory as your user config for security reasons.
If you work across multiple computers, you may want to keep your user config directory in sync."###;

/// A mechanism to access config files for a repo securely.
#[derive(Clone, Debug)]
pub struct SecureConfig {
    /// Technically this is either a repo or a workspace.
    repo_dir: PathBuf,
    /// The name of the config id file.
    config_id_name: &'static str,
    /// The name of the legacy config file.
    legacy_config_name: &'static str,
    /// A cache of the output \[maybe_\]load_config
    cache: RefCell<Option<(Option<PathBuf>, ConfigMetadata)>>,
}

/// An error when attempting to load config from disk.
#[derive(Error, Debug)]
pub enum SecureConfigError {
    /// Failed to read / write to the specified path
    #[error(transparent)]
    PathError(#[from] PathError),

    /// Failed to decode the user configuration file.
    #[error(transparent)]
    DecodeError(#[from] prost::DecodeError),

    /// The path failed to decode to bytes.
    #[error(transparent)]
    BadPathEncoding(#[from] BadPathEncoding),

    /// The config ID isn't CONFIG_ID_BYTES * 2 hex chars.
    #[error("Found an invalid config ID")]
    BadConfigIdError,
}

/// The path to the config file for a secure config.
/// Also contains metadata about the repo and info to be displayed to the
/// user.
#[derive(Clone, Debug, Default)]
pub struct LoadedSecureConfig {
    /// The path to the config file.
    /// Can be None if the config-ID is not generated.
    pub config_file: Option<PathBuf>,
    /// The metadata for the config.
    pub metadata: ConfigMetadata,
    /// Any warnings that we want to be reported to the user.
    pub warnings: Vec<String>,
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<(), PathError> {
    let d = path.parent().unwrap();
    let mut temp_file = NamedTempFile::new_in(d).context(d)?;
    temp_file.write_all(content).context(temp_file.path())?;
    temp_file.persist(path).map_err(|e| PathError {
        path: path.to_path_buf(),
        source: e.error,
    })?;
    Ok(())
}

fn generate_config_id(rng: &mut ChaCha20Rng) -> String {
    encode_hex(&rng.random::<[u8; CONFIG_ID_BYTES]>())
}

fn update_metadata(config_dir: &Path, metadata: &ConfigMetadata) -> Result<(), SecureConfigError> {
    let metadata_path = config_dir.join(METADATA_FILE);
    atomic_write(&metadata_path, &metadata.encode_to_vec())?;
    Ok(())
}

impl SecureConfig {
    /// Creates a secure config.
    fn new(
        repo_dir: PathBuf,
        config_id_name: &'static str,
        legacy_config_name: &'static str,
    ) -> Self {
        Self {
            repo_dir,
            config_id_name,
            legacy_config_name,
            cache: RefCell::new(None),
        }
    }

    /// Creates a secure config for a repo. Takes the .jj/repo directory.
    pub fn new_repo(repo_dir: PathBuf) -> Self {
        Self::new(repo_dir, "config-id", "config.toml")
    }

    /// Creates a secure config for a workspace. Takes the .jj directory.
    pub fn new_workspace(workspace_dir: PathBuf) -> Self {
        Self::new(
            workspace_dir,
            "workspace-config-id",
            "workspace-config.toml",
        )
    }

    fn generate_config(
        &self,
        root_config_dir: &Path,
        config_id: &str,
        content: Option<&[u8]>,
        metadata: &ConfigMetadata,
    ) -> Result<PathBuf, SecureConfigError> {
        let config_dir = root_config_dir.join(config_id);
        let config_path = config_dir.join(CONFIG_FILE);
        fs::create_dir_all(&config_dir).context(&config_dir)?;
        update_metadata(&config_dir, metadata)?;
        if let Some(content) = content {
            fs::write(&config_path, content).context(&config_path)?;
        }

        // Write the config ID atomically. A half-formed config ID would be very bad.
        atomic_write(
            &self.repo_dir.join(self.config_id_name),
            config_id.as_bytes(),
        )?;
        Ok(config_path)
    }

    fn generate_initial_config(
        &self,
        root_config_dir: &Path,
        config_id: &str,
    ) -> Result<(PathBuf, ConfigMetadata), SecureConfigError> {
        let metadata = ConfigMetadata {
            path: path_to_bytes(&self.repo_dir).ok().map(|b| b.to_vec()),
            ..Default::default()
        };
        let path = self.generate_config(root_config_dir, config_id, None, &metadata)?;
        Ok((path, metadata))
    }

    /// Validates that the metadata path matches the repo path.
    /// If there's a mismatch, takes appropriate action.
    /// Returns the updated config dir and metadata.
    fn handle_metadata_path(
        &self,
        rng: &mut ChaCha20Rng,
        root_config_dir: &Path,
        config_dir: PathBuf,
        mut metadata: ConfigMetadata,
    ) -> Result<LoadedSecureConfig, SecureConfigError> {
        let encoded = path_to_bytes(&self.repo_dir).ok();
        let got = metadata.path.as_deref().map(path_from_bytes).transpose()?;

        if got == encoded.is_some().then_some(self.repo_dir.as_path()) {
            return Ok(LoadedSecureConfig {
                config_file: Some(config_dir.join(CONFIG_FILE)),
                metadata,
                warnings: vec![],
            });
        }
        let got = match got {
            Some(d) if d.is_dir() => d.to_path_buf(),
            _ => {
                // The old repo does not exist. Assume the user moved it.
                metadata.path = encoded.map(|b| b.to_vec());
                update_metadata(&config_dir, &metadata)?;
                return Ok(LoadedSecureConfig {
                    config_file: Some(config_dir.join(CONFIG_FILE)),
                    metadata,
                    warnings: vec![],
                });
            }
        };
        // We attempt to create a temporary file in the new repo.
        // If it fails, we have readonly access to a repo, so we do nothing.
        // If we write to the new repo and it shows up in the old one,
        // we can skip this step, since it's not a copy.
        if let Ok(tmp) = NamedTempFile::new_in(&self.repo_dir)
            && !got.join(tmp.path().file_name().unwrap()).exists()
        {
            // We now assume the repo was copied. Since the repo was copied,
            // the config should be copied too, rather than sharing the
            // config with what it copied from.
            let old_config_path = config_dir.join(CONFIG_FILE);
            metadata.path = encoded.map(|b| b.to_vec());
            let old_config_content = fs::read(&old_config_path).context(&old_config_path)?;
            let config_path = self.generate_config(
                root_config_dir,
                &generate_config_id(rng),
                Some(&old_config_content),
                &metadata,
            )?;
            return Ok(LoadedSecureConfig {
                config_file: Some(config_path.clone()),
                metadata,
                warnings: vec![format!(
                    "Your repo appears to have been copied from {} to {}. The corresponding repo \
                     config file has also been copied.",
                    got.display(),
                    &self.repo_dir.display()
                )],
            });
        }
        Ok(LoadedSecureConfig {
            config_file: Some(config_dir.join(CONFIG_FILE)),
            metadata,
            warnings: vec![],
        })
    }

    #[cfg(unix)]
    fn update_legacy_config_file(
        &self,
        new_config: &Path,
        _content: &[u8],
    ) -> Result<(), SecureConfigError> {
        let legacy_config = self.repo_dir.join(self.legacy_config_name);
        // Make old versions and new versions of jj share the same config file.
        fs::remove_file(&legacy_config).context(&legacy_config)?;
        std::os::unix::fs::symlink(new_config, &legacy_config).context(&legacy_config)?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn update_legacy_config_file(
        &self,
        _new_config: &Path,
        content: &[u8],
    ) -> Result<(), SecureConfigError> {
        let legacy_config = self.repo_dir.join(self.legacy_config_name);
        // I considered making this readonly, but that would prevent you from
        // updating the config with old versions of jj.
        // In the future, we consider something a little more robust, where as
        // the non-legacy config changes, we propagate that to the legacy config.
        // However, it seems a little overkill, considering it only affects windows
        // users who use multiple versions of jj at once, and only for a year.
        let mut new_content = CONTENT_PREFIX.as_bytes().to_vec();
        new_content.extend_from_slice(content);
        fs::write(&legacy_config, new_content).context(&legacy_config)?;
        Ok(())
    }

    /// Migrates the legacy config, if it exists.
    fn maybe_migrate_legacy_config(
        &self,
        rng: &mut ChaCha20Rng,
        root_config_dir: &Path,
    ) -> Result<LoadedSecureConfig, SecureConfigError> {
        // TODO: This function should be updated in jj 0.49 to no longer
        // automatically migrate repos, but instead print out a warning.
        let legacy_config = self.repo_dir.join(self.legacy_config_name);
        let config = match fs::read(&legacy_config).context(&legacy_config) {
            Ok(config_content) => config_content,
            // No legacy config files found.
            Err(e) if e.source.kind() == NotFound => return Ok(Default::default()),
            Err(e) => return Err(e.into()),
        };
        let metadata = ConfigMetadata {
            path: path_to_bytes(&self.repo_dir).ok().map(|b| b.to_vec()),
            ..Default::default()
        };
        let config_file = self.generate_config(
            root_config_dir,
            &generate_config_id(rng),
            Some(&config),
            &metadata,
        )?;
        self.update_legacy_config_file(&config_file, &config)?;
        Ok(LoadedSecureConfig {
            warnings: vec![format!(
                "Your config file has been migrated from {} to {}. You can edit the new file with \
                 `jj config edit`",
                legacy_config.display(),
                config_file.display(),
            )],
            config_file: Some(config_file),
            metadata,
        })
    }

    /// Determines the path to the config, and any metadata associated with it.
    /// If no config exists, the path will be None.
    pub fn maybe_load_config(
        &self,
        rng: &mut ChaCha20Rng,
        root_config_dir: &Path,
    ) -> Result<LoadedSecureConfig, SecureConfigError> {
        if let Some(cache) = self.cache.borrow().as_ref() {
            return Ok(LoadedSecureConfig {
                config_file: cache.0.clone(),
                metadata: cache.1.clone(),
                warnings: vec![],
            });
        }
        let config_id_path = self.repo_dir.join(self.config_id_name);
        let loaded = match fs::read_to_string(&config_id_path).context(&config_id_path) {
            Ok(config_id) => {
                if config_id.len() != CONFIG_ID_BYTES * 2
                    || !config_id.chars().all(|c| c.is_ascii_hexdigit())
                {
                    return Err(SecureConfigError::BadConfigIdError);
                }
                let config_dir = root_config_dir.join(&config_id);
                let metadata_path = config_dir.join(METADATA_FILE);
                match fs::read(&metadata_path).context(&metadata_path) {
                    Ok(buf) => self.handle_metadata_path(
                        rng,
                        root_config_dir,
                        config_dir,
                        ConfigMetadata::decode(buf.as_slice())?,
                    )?,
                    Err(e) if e.source.kind() == NotFound => {
                        let (path, metadata) =
                            self.generate_initial_config(root_config_dir, &config_id)?;
                        LoadedSecureConfig {
                            config_file: Some(path),
                            metadata,
                            warnings: vec![CONFIG_NOT_FOUND.to_string()],
                        }
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            Err(e) if e.source.kind() == NotFound => {
                self.maybe_migrate_legacy_config(rng, root_config_dir)?
            }
            Err(e) => return Err(SecureConfigError::PathError(e)),
        };
        *self.cache.borrow_mut() = Some((loaded.config_file.clone(), loaded.metadata.clone()));
        Ok(loaded)
    }

    /// Determines the path to the config, and any metadata associated with it.
    /// If no config exists, an empty config file will be generated.
    pub fn load_config(
        &self,
        rng: &mut ChaCha20Rng,
        root_config_dir: &Path,
    ) -> Result<LoadedSecureConfig, SecureConfigError> {
        let mut loaded = self.maybe_load_config(rng, root_config_dir)?;
        if loaded.config_file.is_none() {
            let (path, metadata) =
                self.generate_initial_config(root_config_dir, &generate_config_id(rng))?;
            *self.cache.borrow_mut() = Some((Some(path.clone()), metadata.clone()));
            loaded.config_file = Some(path);
            loaded.metadata = metadata;
        }
        Ok(loaded)
    }

    /// Updates the metadata file for the config.
    pub fn update_metadata(
        &self,
        config_file: &Path,
        metadata: ConfigMetadata,
    ) -> Result<(), PathError> {
        let metadata_file = config_file.parent().unwrap().join(METADATA_FILE);
        atomic_write(&metadata_file, &metadata.encode_to_vec())?;
        *self.cache.borrow_mut() = Some((Some(config_file.to_path_buf()), metadata));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use rand::SeedableRng as _;
    use tempfile::TempDir;

    use super::*;

    struct TestEnv {
        _td: TempDir,
        rng: ChaCha20Rng,
        config: SecureConfig,
        repo_dir: PathBuf,
        config_dir: PathBuf,
    }

    impl TestEnv {
        fn new() -> Self {
            let td = crate::tests::new_temp_dir();
            let repo_dir = td.path().join("repo");
            fs::create_dir(&repo_dir).unwrap();
            let config_dir = td.path().join("config");
            fs::create_dir(&config_dir).unwrap();
            Self {
                _td: td,
                rng: ChaCha20Rng::seed_from_u64(0),
                config: SecureConfig::new(repo_dir.clone(), "config-id", "legacy-config.toml"),
                repo_dir,
                config_dir,
            }
        }

        fn secure_config_for_dir(&self, d: PathBuf) -> SecureConfig {
            SecureConfig::new(d, "config-id", "legacy-config.toml")
        }
    }

    #[test]
    fn test_no_initial_config() {
        let mut env = TestEnv::new();

        // We shouldn't generate the config.
        let loaded = env
            .config
            .maybe_load_config(&mut env.rng, &env.config_dir)
            .unwrap();
        assert_eq!(loaded.config_file, None);
        assert_eq!(loaded.metadata, Default::default());
        assert!(loaded.warnings.is_empty());
        // The cache entry should be filled.
        assert!(env.config.cache.borrow().is_some());

        // load_config should generate the config if it previously didn't exist.
        let loaded = env
            .config
            .load_config(&mut env.rng, &env.config_dir)
            .unwrap();
        let path = loaded.config_file.unwrap();
        let components: Vec<_> = path.components().rev().collect();
        assert_eq!(
            components[0],
            std::path::Component::Normal(OsStr::new("config.toml"))
        );
        assert_eq!(
            components[2],
            std::path::Component::Normal(OsStr::new("config"))
        );
        assert!(!loaded.metadata.path.as_deref().unwrap().is_empty());
        assert!(loaded.warnings.is_empty());

        // load_config should leave it untouched if it did exist.
        // Empty the cache to ensure the function is actually being tested
        assert!(env.config.cache.borrow().is_some());
        *env.config.cache.borrow_mut() = None;
        let loaded2 = env
            .config
            .load_config(&mut env.rng, &env.config_dir)
            .unwrap();
        assert_eq!(loaded2.config_file.unwrap(), path);
        assert_eq!(loaded2.metadata, loaded.metadata);
        assert!(loaded2.warnings.is_empty());
    }

    #[test]
    fn test_migrate_legacy_config() {
        let mut env = TestEnv::new();

        let legacy_config = env.repo_dir.join("legacy-config.toml");
        fs::write(&legacy_config, "config").unwrap();
        let loaded = env
            .config
            .maybe_load_config(&mut env.rng, &env.config_dir)
            .unwrap();
        assert!(loaded.config_file.is_some());
        assert!(!loaded.metadata.path.unwrap().is_empty());
        assert_eq!(
            fs::read_to_string(loaded.config_file.as_deref().unwrap()).unwrap(),
            "config"
        );
        assert!(!loaded.warnings.is_empty());

        // On unix, it should be a symlink.
        if cfg!(unix) {
            fs::write(loaded.config_file.as_deref().unwrap(), "new").unwrap();
            assert_eq!(fs::read_to_string(&legacy_config).unwrap(), "new");
        }
    }

    #[test]
    fn test_repo_moved() {
        let mut env = TestEnv::new();
        let loaded = env
            .config
            .load_config(&mut env.rng, &env.config_dir)
            .unwrap();
        let path = loaded.config_file.unwrap();

        let dest = env.repo_dir.parent().unwrap().join("moved");
        fs::rename(&env.repo_dir, &dest).unwrap();
        let config = env.secure_config_for_dir(dest);
        let loaded2 = config.load_config(&mut env.rng, &env.config_dir).unwrap();
        assert_eq!(loaded2.config_file.unwrap(), path);
        assert_ne!(loaded.metadata.path, loaded2.metadata.path);
        assert!(loaded2.warnings.is_empty());
    }

    #[test]
    fn test_repo_copied() {
        let mut env = TestEnv::new();
        let loaded = env
            .config
            .load_config(&mut env.rng, &env.config_dir)
            .unwrap();
        let path = loaded.config_file.unwrap();
        fs::write(&path, "config").unwrap();

        let dest = env.repo_dir.parent().unwrap().join("copied");
        fs::create_dir(&dest).unwrap();
        fs::copy(env.repo_dir.join("config-id"), dest.join("config-id")).unwrap();
        let config = env.secure_config_for_dir(dest);
        let loaded2 = config.load_config(&mut env.rng, &env.config_dir).unwrap();
        let path2 = loaded2.config_file.unwrap();
        assert_ne!(path, path2);
        assert_eq!(fs::read_to_string(path2).unwrap(), "config");
        assert_ne!(loaded.metadata.path, loaded2.metadata.path);
        // We should get a warning about the repo having been copied.
        assert!(!loaded2.warnings.is_empty());
    }

    // This feature works on windows as well, it just isn't easy to replicate with a
    // test.
    #[cfg(unix)]
    #[test]
    fn test_repo_aliased() {
        let mut env = TestEnv::new();
        let loaded = env
            .config
            .load_config(&mut env.rng, &env.config_dir)
            .unwrap();
        let path = loaded.config_file.unwrap();

        let dest = env.repo_dir.parent().unwrap().join("copied");
        std::os::unix::fs::symlink(&env.repo_dir, &dest).unwrap();
        let config = env.secure_config_for_dir(dest);
        let loaded2 = config.load_config(&mut env.rng, &env.config_dir).unwrap();
        assert_eq!(loaded2.config_file.unwrap(), path);
        assert_eq!(loaded.metadata.path, loaded2.metadata.path);
        assert!(loaded2.warnings.is_empty());
    }

    #[test]
    fn test_missing_config() {
        let mut env = TestEnv::new();
        let loaded = env
            .config
            .load_config(&mut env.rng, &env.config_dir)
            .unwrap();
        let path = loaded.config_file.unwrap();

        fs::remove_dir_all(path.parent().unwrap()).unwrap();
        *env.config.cache.borrow_mut() = None;

        let loaded2 = env
            .config
            .load_config(&mut env.rng, &env.config_dir)
            .unwrap();
        assert_eq!(loaded2.config_file.unwrap(), path);
        assert_eq!(loaded.metadata.path, loaded2.metadata.path);
        // It should have recreated the directory.
        assert!(path.parent().unwrap().is_dir());
        assert!(!loaded2.warnings.is_empty());
    }
}
