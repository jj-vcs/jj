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
use std::io::ErrorKind::NotFound;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;

use prost::Message as _;
use rand::Rng as _;
use rand::SeedableRng as _;
use rand_chacha::ChaCha20Rng;
use tempfile::NamedTempFile;
use tempfile::PersistError;
use thiserror::Error;

use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;
use crate::protos::secure_config::ConfigMetadata;

const CONFIG_FILE: &str = "config.toml";
const METADATA_FILE: &str = "metadata.binpb";
#[cfg(not(unix))]
const CONTENT_PREFIX: &str = r###"# DO NOT EDIT.
# This file is for old versions of jj.
# Use `jj config path` or `jj config edit` to find and edit the new file

"###;

/// A mechanism to access config files for a repo securely.
#[derive(Clone, Debug)]
pub struct SecureConfig {
    // We don't use JJRng because that depends on the seed, which comes
    // from config files, so doing so would be circular.
    rng: RefCell<ChaCha20Rng>,

    // Technically this is either a repo or a workspace.
    repo_dir: PathBuf,
    // A directory containing subdirectories, each corresponding to a config id.
    config_dir: PathBuf,
    // The name of the config id file.
    config_id_name: &'static str,
    // The name of the legacy config file.
    legacy_config_name: &'static str,
    // A cache of the output of the config.
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

    /// I/O for a file failed
    #[error(transparent)]
    IoError(#[from] std::io::Error),

    /// Failed to persist the temporary file to disk
    #[error(transparent)]
    PersistError(#[from] PersistError),
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<(), SecureConfigError> {
    let mut temp_file = NamedTempFile::new_in(path.parent().unwrap()).context(path)?;
    temp_file.write_all(content).context(path)?;
    temp_file.persist(path)?;
    Ok(())
}

impl SecureConfig {
    /// Creates a secure config.
    pub fn new(
        repo_dir: PathBuf,
        config_dir: PathBuf,
        config_id_name: &'static str,
        legacy_config_name: &'static str,
    ) -> Self {
        Self {
            rng: RefCell::new(ChaCha20Rng::from_os_rng()),
            repo_dir,
            config_dir,
            config_id_name,
            legacy_config_name,
            cache: RefCell::new(None),
        }
    }

    fn generate_config_id(&self) -> String {
        hex::encode(
            (0..10)
                .map(|_| self.rng.borrow_mut().random::<u8>())
                .collect::<Vec<_>>(),
        )
    }

    fn generate_config(
        &self,
        config_id: &str,
        content: &[u8],
        metadata: &ConfigMetadata,
    ) -> Result<PathBuf, SecureConfigError> {
        let d = self.config_dir.join(config_id);
        let config_path = d.join(CONFIG_FILE);
        std::fs::create_dir_all(&d).context(&d)?;
        self.update_metadata(&d, metadata)?;
        if !content.is_empty() {
            std::fs::write(&config_path, content).context(&config_path)?;
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
        config_id: &str,
    ) -> Result<(PathBuf, ConfigMetadata), SecureConfigError> {
        let metadata = ConfigMetadata {
            path: dunce::canonicalize(&self.repo_dir)?
                .to_str()
                .unwrap()
                .to_string(),
        };
        let path = self.generate_config(config_id, &[], &metadata)?;
        Ok((path, metadata))
    }

    fn update_metadata(
        &self,
        config_dir: &Path,
        metadata: &ConfigMetadata,
    ) -> Result<(), SecureConfigError> {
        let metadata_path = config_dir.join(METADATA_FILE);
        atomic_write(&metadata_path, &metadata.encode_to_vec())?;
        Ok(())
    }

    /// Validates that the metadata path matches the repo path.
    /// If there's a mismatch, takes appropriate action.
    /// Returns the updated config dir and metadata.
    fn handle_metadata_path(
        &self,
        config_dir: PathBuf,
        mut metadata: ConfigMetadata,
    ) -> Result<(PathBuf, ConfigMetadata), SecureConfigError> {
        let want = dunce::canonicalize(&self.repo_dir)?;
        let got = PathBuf::from(metadata.path.clone());
        if want == got {
            return Ok((config_dir, metadata));
        }
        if !got.is_dir() {
            // The old repo does not exist. Assume the user moved it.
            metadata.path = want.to_str().unwrap().to_string();
            self.update_metadata(&config_dir, &metadata)?;
            return Ok((config_dir, metadata));
        }
        // We attempt to create a temporary file in the new repo.
        // If it fails, we have readonly access to a repo, so we do nothing.
        if let Ok(tmp) = NamedTempFile::new_in(&self.repo_dir) {
            // If we write to the new repo and it shows up in the old one,
            // we can skip this step, since it's not a copy.
            if !got.join(tmp.path().file_name().unwrap()).exists() {
                // We now assume the repo was copied. Since the repo was copied,
                // the config should be copied too, rather than sharing the
                // config with what it copied from.
                let old_config_path = config_dir.join(CONFIG_FILE);
                metadata.path = want.to_str().unwrap().to_string();
                let old_config_content =
                    std::fs::read(&old_config_path).context(&old_config_path)?;
                return Ok((
                    self.generate_config(
                        &self.generate_config_id(),
                        &old_config_content,
                        &metadata,
                    )?
                    .parent()
                    .unwrap()
                    .to_path_buf(),
                    metadata,
                ));
            }
        }
        Ok((config_dir, metadata))
    }

    /// Migrates the legacy config, if it exists.
    fn maybe_migrate_legacy_config(
        &self,
    ) -> Result<(Option<PathBuf>, ConfigMetadata), SecureConfigError> {
        let legacy_config = self.repo_dir.join(self.legacy_config_name);
        let config = match std::fs::read(&legacy_config).context(&legacy_config) {
            Ok(config) => config,
            // No legacy config files found.
            Err(e) if e.source.kind() == NotFound => return Ok(Default::default()),
            Err(e) => return Err(e.into()),
        };
        let canonical_repo_dir = dunce::canonicalize(&self.repo_dir)?;
        let metadata = ConfigMetadata {
            path: canonical_repo_dir.to_str().unwrap().to_string(),
        };
        let config_file = self.generate_config(&self.generate_config_id(), &config, &metadata)?;

        #[cfg(unix)]
        {
            // Make old versions and new versions of jj share the same config file.
            std::fs::remove_file(&legacy_config).context(&legacy_config)?;
            std::os::unix::fs::symlink(dunce::canonicalize(&config_file)?, &legacy_config)
                .context(&legacy_config)?;
        }
        #[cfg(not(unix))]
        {
            // I considered making this readonly, but that would prevent you from
            // updating the config with old versions of jj.
            // In the future, we consider something a little more robust, where as
            // the non-legacy config changes, we propagate that to the legacy config.
            // However, it seems a little overkill, considering it only affects windows
            // users who use multiple versions of jj at once, and only for a year.
            let mut content = CONTENT_PREFIX.as_bytes().to_vec();
            content.extend_from_slice(&config);
            std::fs::write(&legacy_config, content).context(&legacy_config)?;
        }
        Ok((Some(config_file), metadata))
    }

    /// Determines the path to the config, and any metadata associated with it.
    /// If no config exists, the path will be None.
    pub fn maybe_load_config(
        &self,
    ) -> Result<(Option<PathBuf>, ConfigMetadata), SecureConfigError> {
        if let Some(cache) = self.cache.borrow().as_ref() {
            return Ok(cache.clone());
        }
        let config_id_path = self.repo_dir.join(self.config_id_name);
        let value = match std::fs::read_to_string(&config_id_path).context(&config_id_path) {
            Ok(s) => {
                let s = s.trim_end();
                let config_dir = self.config_dir.join(s);
                let metadata_path = config_dir.join(METADATA_FILE);
                match std::fs::read(&metadata_path).context(&metadata_path) {
                    Ok(buf) => {
                        let (config_dir, metadata) = self.handle_metadata_path(
                            config_dir,
                            ConfigMetadata::decode(buf.as_slice())?,
                        )?;
                        (Some(config_dir.join(CONFIG_FILE)), metadata)
                    }
                    Err(e) if e.source.kind() == NotFound => {
                        let (path, metadata) = self.generate_initial_config(s)?;
                        (Some(path), metadata)
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            Err(e) if e.source.kind() == NotFound => self.maybe_migrate_legacy_config()?,
            Err(e) => return Err(SecureConfigError::PathError(e)),
        };
        *self.cache.borrow_mut() = Some(value.clone());
        Ok(value)
    }

    /// Determines the path to the config, and any metadata associated with it.
    /// If no config exists, an empty config file will be generated.
    pub fn load_config(&self) -> Result<(PathBuf, ConfigMetadata), SecureConfigError> {
        Ok(match self.maybe_load_config()? {
            (Some(path), metadata) => (path, metadata),
            (None, _) => {
                let (path, metadata) = self.generate_initial_config(&self.generate_config_id())?;
                *self.cache.borrow_mut() = Some((Some(path.clone()), metadata.clone()));
                (path, metadata)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use tempfile::TempDir;

    use super::*;

    struct TestEnv {
        _td: TempDir,
        config: SecureConfig,
        repo_dir: PathBuf,
        config_dir: PathBuf,
    }

    impl TestEnv {
        fn new() -> Self {
            let td = TempDir::new().unwrap();
            let repo_dir = td.path().join("repo");
            std::fs::create_dir(&repo_dir).unwrap();
            let config_dir = td.path().join("config");
            std::fs::create_dir(&config_dir).unwrap();
            Self {
                _td: td,
                config: SecureConfig::new(
                    repo_dir.clone(),
                    config_dir.clone(),
                    "config-id",
                    "legacy-config.toml",
                ),
                repo_dir,
                config_dir,
            }
        }

        fn secure_config_for_dir(&self, d: PathBuf) -> SecureConfig {
            SecureConfig::new(
                d,
                self.config_dir.clone(),
                "config-id",
                "legacy-config.toml",
            )
        }
    }

    #[test]
    fn test_no_initial_config() {
        let env = TestEnv::new();

        // We shouldn't generate the config.
        let (path, metadata) = env.config.maybe_load_config().unwrap();
        assert_eq!(path, None);
        assert_eq!(metadata, Default::default());
        // The cache entry should be filled.
        assert!(env.config.cache.borrow().is_some());

        // load_config should generate the config if it previously didn't exist.
        let (path, metadata) = env.config.load_config().unwrap();
        let components: Vec<_> = path.components().rev().collect();
        assert_eq!(
            components[0],
            std::path::Component::Normal(OsStr::new("config.toml"))
        );
        assert_eq!(
            components[2],
            std::path::Component::Normal(OsStr::new("config"))
        );
        assert_ne!(metadata.path, "");

        // load_config should leave it untouched if it did exist.
        // Empty the cache to ensure the function is actually being tested
        assert!(env.config.cache.borrow().is_some());
        *env.config.cache.borrow_mut() = None;
        let (path2, metadata2) = env.config.load_config().unwrap();
        assert_eq!(path2, path);
        assert_eq!(metadata2, metadata);
    }

    #[test]
    fn test_migrate_legacy_config() {
        let env = TestEnv::new();

        let legacy_config = env.repo_dir.join("legacy-config.toml");
        std::fs::write(&legacy_config, "config").unwrap();
        let (new_config, metadata) = env.config.maybe_load_config().unwrap();
        assert!(new_config.is_some());
        assert_ne!(metadata.path, "");
        assert_eq!(
            std::fs::read_to_string(new_config.as_deref().unwrap()).unwrap(),
            "config"
        );

        // On unix, it should be a symlink.
        #[cfg(unix)]
        {
            std::fs::write(new_config.as_deref().unwrap(), "new").unwrap();
            assert_eq!(std::fs::read_to_string(&legacy_config).unwrap(), "new");
        }
    }

    #[test]
    fn test_repo_moved() {
        let env = TestEnv::new();
        let (path, metadata) = env.config.load_config().unwrap();

        let dest = env.repo_dir.parent().unwrap().join("moved");
        std::fs::rename(&env.repo_dir, &dest).unwrap();
        let config = env.secure_config_for_dir(dest);
        let (path2, metadata2) = config.load_config().unwrap();
        assert_eq!(path, path2);
        assert_ne!(metadata.path, metadata2.path);
    }

    #[test]
    fn test_repo_copied() {
        let env = TestEnv::new();
        let (path, metadata) = env.config.load_config().unwrap();
        std::fs::write(&path, "config").unwrap();

        let dest = env.repo_dir.parent().unwrap().join("copied");
        std::fs::create_dir(&dest).unwrap();
        std::fs::copy(env.repo_dir.join("config-id"), dest.join("config-id")).unwrap();
        let config = env.secure_config_for_dir(dest);
        let (path2, metadata2) = config.load_config().unwrap();
        assert_ne!(path, path2);
        assert_eq!(std::fs::read_to_string(path2).unwrap(), "config");
        assert_ne!(metadata.path, metadata2.path);
    }

    // This feature works on windows as well, it just isn't easy to replicate with a
    // test.
    #[cfg(unix)]
    #[test]
    fn test_repo_aliased() {
        let env = TestEnv::new();
        let (path, metadata) = env.config.load_config().unwrap();

        let dest = env.repo_dir.parent().unwrap().join("copied");
        std::os::unix::fs::symlink(&env.repo_dir, &dest).unwrap();
        let config = env.secure_config_for_dir(dest);
        let (path2, metadata2) = config.load_config().unwrap();
        assert_eq!(path, path2);
        assert_eq!(metadata.path, metadata2.path);
    }

    #[test]
    fn test_missing_config() {
        let env = TestEnv::new();
        let (path, metadata) = env.config.load_config().unwrap();

        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        *env.config.cache.borrow_mut() = None;

        let (path2, metadata2) = env.config.load_config().unwrap();
        assert_eq!(path, path2);
        assert_eq!(metadata.path, metadata2.path);
        // It should have recreated the directory.
        assert!(path.parent().unwrap().is_dir());
    }
}
