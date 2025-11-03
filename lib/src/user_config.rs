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

//! A wrapper around retrieval and storage of secure user configuration.

use std::fmt::Debug;
use std::fs;
use std::io;
use std::path::Path;

use digest::Key;
use digest::Mac as _;
use hmac::Hmac;
use io::Write as _;
use prost::Message as _;
use rand::RngCore as _;
use sha2::Sha256;
use tempfile::NamedTempFile;
use thiserror::Error;

use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;
use crate::protos::user_config::SecureUserConfig;
use crate::protos::user_config::Signature;
use crate::protos::user_config::UserConfig;

/// The filename for the secure config file for the repository.
pub const REPO_CONFIG_FILE: &str = "secure-repo-config.binpb";
/// The filename for the secure config file for the workspace.
pub const WORKSPACE_CONFIG_FILE: &str = "secure-workspace-config.binpb";

type Signer = Hmac<Sha256>;
/// The type for the key used to sign messages as a given user.
pub type UserConfigKey = Key<Signer>;

/// Error occurred while dealing with user configs
#[derive(Error, Debug)]
pub enum UserConfigError {
    /// Failed to read / write to the specified path
    #[error(transparent)]
    PathError(#[from] PathError),

    /// Failed to decode the user configuration file.
    #[error(transparent)]
    DecodeError(#[from] prost::DecodeError),

    /// Missing the user configuration file.
    #[error("Secure config not found")]
    NotFound,

    /// Repository has moved from one location to another.
    #[error("This repo has moved from {from} to {to}")]
    RepoMoved {
        /// The old path to the repo, encoded as a string.
        from: String,
        /// The new path to the repo, encoded as a string.
        to: String,
    },

    /// Repository had the wrong signature.
    #[error("Invalid signature")]
    InvalidSignature,
}

/// Reads the user configuration from a repository for the current user.
pub fn read_user_config(
    secure_config: &Path,
    key: Option<&UserConfigKey>,
) -> Result<UserConfig, UserConfigError> {
    let dir = secure_config.parent().unwrap();
    let buf = match fs::read(secure_config).context(secure_config) {
        Ok(buf) => buf,
        Err(e) if e.source.kind() == io::ErrorKind::NotFound => {
            if dir.exists() {
                return Err(UserConfigError::NotFound);
            } else {
                // If the user runs `jj git init`, for example, this can be run
                // from a nonexistent repo, and that's ok.
                return Ok(Default::default());
            }
        }
        Err(e) => return Err(UserConfigError::PathError(e)),
    };
    let secure_config = SecureUserConfig::decode(&*buf)?;
    let config = UserConfig::decode(&*secure_config.storage)?;

    let key = if let Some(key) = key {
        key
    } else {
        // If we don't have a key, we have to trust the config.
        return Ok(config);
    };

    let sign = |s: &str| -> Vec<u8> {
        let mut signer = Signer::new(key);
        signer.update(&secure_config.storage);
        signer.update(&secure_config.salt);
        signer.update(s.as_bytes());
        signer.finalize().into_bytes().to_vec()
    };

    let canonical = dunce::canonicalize(dir)
        .context(dir)
        .map_err(UserConfigError::PathError)?
        .to_string_lossy()
        .to_string();
    let mut err = None;
    // Do it in reverse order since the front should be the most stale.
    for signature in secure_config.signatures.iter().rev() {
        if sign(&canonical) == *signature.signature {
            return Ok(config);
        } else if err.is_none() && sign(&signature.path) == *signature.signature {
            err = Some(UserConfigError::RepoMoved {
                from: signature.path.clone(),
                to: canonical.clone(),
            });
        }
    }
    Err(err.unwrap_or(UserConfigError::InvalidSignature))
}

/// Writes a given configuration for a repository or workspace.
pub fn write_user_config(
    path: &Path,
    config: &UserConfig,
    key: Option<&UserConfigKey>,
) -> Result<(), UserConfigError> {
    let dir = path.parent().unwrap();

    let content = config.encode_to_vec();

    let canonical = dunce::canonicalize(dir)
        .context(dir)?
        .to_string_lossy()
        .to_string();

    let mut secure_config = if let Ok(buf) = std::fs::read(path)
        && let Ok(secure_config) = SecureUserConfig::decode(&*buf)
        && secure_config.storage == content
    {
        secure_config
    } else {
        let mut salt = [0u8; 8];
        rand::rng().fill_bytes(&mut salt);
        SecureUserConfig {
            storage: content,
            salt: salt.to_vec(),
            signatures: vec![],
        }
    };

    let signature = key.map(|key| {
        let mut signer = Hmac::<Sha256>::new(key);
        signer.update(&secure_config.storage);
        signer.update(&secure_config.salt);
        signer.update(canonical.as_bytes());
        signer.finalize().into_bytes().to_vec()
    });

    secure_config.signatures.push(Signature {
        path: canonical,
        signature: signature.unwrap_or_default(),
    });

    // Atomically write to the file.
    let mut f = NamedTempFile::new_in(dir).context(dir)?;
    f.write_all(&secure_config.encode_to_vec()).context(dir)?;
    fs::rename(f.path(), path).context(path)?;

    Ok(())
}
