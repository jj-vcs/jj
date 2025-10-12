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

use std::io;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigSource;
use jj_lib::content_hash::blake2b_hash;
use jj_lib::file_util;
use jj_lib::file_util::BadPathEncoding;
use jj_lib::file_util::IoResultExt as _;
use jj_lib::file_util::PathError;
use jj_lib::file_util::persist_temp_file;
use jj_lib::hex_util;
use jj_lib::repo_path::RepoPathBuf;
use tempfile::NamedTempFile;

use crate::command_error::CommandError;
use crate::ui::Ui;

pub const MANAGED_PATH: &str = ".config/jj/config.toml";
pub const LAST_APPROVED: &str = "last_approved";

/// Manages the repo-managed config. The file structure for the repo-managed
/// config for a repo contained at /path/to/repo is:
/// ~/.local/state/jj/repos/hash(/path/to/repo)/last_approved:
///   hash(config1)
/// ~/.local/state/jj/repos/hash(/path/to/repo)/last_approved_hash(workspace):
///   hash(config1)
/// ~/.local/state/jj/repos/hash(/path/to/repo)/hash(config1):
///   approved content for config1
/// ~/.local/state/jj/repos/hash(/path/to/repo)/hash(config2):
///   approved content for config2
pub struct RepoManagedConfig {
    config_dir: PathBuf,
    repo_path: PathBuf,
    last_approved_name: String,
}

static PRINTED_WARNING: AtomicBool = AtomicBool::new(false);

impl RepoManagedConfig {
    pub fn new(
        state_dir: &Path,
        repo_path: &Path,
        workspace_path: &Path,
    ) -> Result<Self, BadPathEncoding> {
        Ok(Self {
            config_dir: state_dir
                .join("repos")
                .join(Self::digest(file_util::path_to_bytes(repo_path)?)),
            repo_path: repo_path.to_path_buf(),
            last_approved_name: format!(
                "{LAST_APPROVED}_{}",
                Self::digest(file_util::path_to_bytes(workspace_path)?)
            ),
        })
    }

    fn write(path: &Path, content: &[u8]) -> Result<(), PathError> {
        let mut temp_file = NamedTempFile::new_in(path.parent().unwrap()).context(path)?;
        temp_file.as_file_mut().write_all(content).context(path)?;
        persist_temp_file(temp_file, path).context(path)?;
        Ok(())
    }

    fn approved_config(&self, digest: &str) -> PathBuf {
        self.config_dir.join(format!("{digest}.toml"))
    }

    pub fn last_approved(&self) -> Result<Option<PathBuf>, PathError> {
        let read = |path: &Path| match std::fs::read_to_string(path) {
            Ok(content) => Ok(Some(self.approved_config(&content))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).context(path),
        };

        Ok(
            if let Some(last_approved) = read(&self.config_dir.join(&self.last_approved_name))? {
                Some(last_approved)
            } else {
                read(&self.config_dir.join(LAST_APPROVED))?
            },
        )
    }

    /// Updates last-approved to the current digest.
    fn approve(&self, digest: &str) -> Result<(), PathError> {
        // Do not use a symlink since they don't play nice with windows.
        // Apparently it's requires privilege escalation on windows.
        Self::write(
            &self.config_dir.join(&self.last_approved_name),
            digest.as_bytes(),
        )?;
        Self::write(&self.config_dir.join(LAST_APPROVED), digest.as_bytes())
    }

    /// Sets the approved config to content for the given digest.
    pub fn approve_content(&self, digest: &str, content: &[u8]) -> Result<(), PathError> {
        // This never removes old ones. This is by design. In the future, we
        // might want to do some kind of TTL or some way to cleanup old ones.
        // It doesn't matter that much though, since configs are likely small
        // and infrequently changed.
        std::fs::create_dir_all(&self.config_dir).context(&self.config_dir)?;
        // This isn't used right now, but it may be used in the future to clean
        // up deleted repos.
        Self::write(
            &self.config_dir.join("repo_path"),
            self.repo_path.as_os_str().as_encoded_bytes(),
        )?;
        Self::write(&self.approved_config(digest), content)?;
        self.approve(digest)
    }

    pub fn get_vcs_config(&self, workspace_root: &Path) -> Result<Option<Vec<u8>>, CommandError> {
        let managed = RepoPathBuf::from_internal_string(MANAGED_PATH)
            .unwrap()
            .to_fs_path_unchecked(workspace_root);
        Ok(match std::fs::read(&managed) {
            Ok(val) => {
                if !val.is_empty() {
                    Some(val)
                } else {
                    None
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => Err(e).context(&managed)?,
        })
    }

    pub fn digest(content: &[u8]) -> String {
        let hash: &[u8] = &blake2b_hash(content);
        // There are two digests in a path (digest of repo path and digest of config
        // file). Each digest is 512 bits, and is encoded as hex, so takes up
        // 128 characters. This means that the pathname has 256 characters of
        // hashes. On windows, the maximum path length is 260 characters, so we
        // use xor to compress 512 bits into 256 so that it fits within the
        // maximum path length.
        #[cfg(windows)]
        let hash = &hash
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[lhs, rhs]| lhs ^ rhs)
            .collect::<Vec<_>>();
        hex_util::encode_hex(hash)
    }

    /// If the digest of MANAGED_PATH exists in the cache, returns
    /// (config_file, true). Otherwise, returns
    /// (the most recent content passed to `approve_content`, false)
    pub fn get_config_file(
        &self,
        workspace_root: &Path,
    ) -> Result<(Option<PathBuf>, bool), CommandError> {
        let config = self.get_vcs_config(workspace_root)?;
        if let Some(config) = config
            && !config.is_empty()
        {
            let digest = Self::digest(&config);
            let out = self.approved_config(&digest);
            let last_approved = self.last_approved()?;
            if out.try_exists()? {
                // This particular config was previously approved, but we need to reset
                // last_approved.
                if Some(&out) != last_approved.as_ref() {
                    self.approve(&digest)?;
                }
                return Ok((Some(out), true));
            } else {
                return Ok((last_approved, false));
            }
        }
        Ok((None, true))
    }

    /// Creates the configuration layer for the repo managed config.
    /// This layer is best-effort based on the approved repo config.
    /// If we don't get a perfect match, it will print a warning that your
    /// config is out of date.
    pub fn create_layer(
        &self,
        ui: &Ui,
        workspace_root: &Path,
    ) -> Result<Option<ConfigLayer>, CommandError> {
        let (last_approved, up_to_date) = self.get_config_file(workspace_root)?;
        if !up_to_date && !PRINTED_WARNING.swap(true, std::sync::atomic::Ordering::AcqRel) {
            writeln!(
                ui.warning_default(),
                "Your repo-managed config is out of date"
            )?;
            writeln!(ui.hint_default(), "Run `jj config review-managed`")?;
        }

        let last_approved = match last_approved {
            Some(x) => x,
            None => return Ok(None),
        };
        match ConfigLayer::load_from_file(ConfigSource::RepoManaged, last_approved) {
            Ok(layer) => Ok(Some(layer)),
            Err(e) => Err(e.into()),
        }
    }
}
