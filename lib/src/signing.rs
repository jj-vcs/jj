// Copyright 2023 The Jujutsu Authors
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

//! Generic APIs to work with cryptographic signatures created and verified by
//! various backends.

pub use jj_core::signing::SigStatus;
pub use jj_core::signing::SignError;
pub use jj_core::signing::SignResult;
use jj_core::signing::Signer as CoreSigner;
pub use jj_core::signing::SigningBackend;
pub use jj_core::signing::Verification;
use thiserror::Error;

use crate::backend::CommitId;
use crate::config::ConfigGetError;
use crate::gpg_signing::GpgBackend;
use crate::gpg_signing::GpgsmBackend;
use crate::settings::UserSettings;
use crate::ssh_signing::SshBackend;
#[cfg(feature = "testing")]
use crate::test_signing_backend::TestSigningBackend;

/// An error type for the signing backend initialization.
#[derive(Debug, Error)]
pub enum SignInitError {
    /// If the backend name specified in the config is not known.
    #[error("Unknown signing backend configured: {0}")]
    UnknownBackend(String),
    /// Failed to load backend configuration.
    #[error("Failed to configure signing backend")]
    BackendConfig(#[source] ConfigGetError),
}

/// A enum that describes if a created/rewritten commit should be signed or not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SignBehavior {
    /// Drop existing signatures.
    /// This is what jj did before signing support or does now when a signing
    /// backend is not configured.
    Drop,
    /// Only sign commits that were authored by self and already signed,
    /// "preserving" the signature across rewrites.
    /// This is what jj does when a signing backend is configured.
    Keep,
    /// Sign/re-sign commits that were authored by self and drop them for
    /// others. This is what jj does when configured to always sign.
    Own,
    /// Always sign commits, regardless of who authored or signed them before.
    /// This is what jj does on `jj sign -f`.
    Force,
}

/// Wraps low-level signing backends and adds caching, similar to `Store`.
#[derive(Debug)]
pub struct Signer {
    /// The CoreSigner contains all fields.
    inner: CoreSigner,
}

impl Signer {
    /// Creates a signer based on user settings. Uses all known backends, and
    /// chooses one of them to be used for signing depending on the config.
    pub fn from_settings(settings: &UserSettings) -> Result<Self, SignInitError> {
        let mut backends: Vec<Box<dyn SigningBackend>> = vec![
            Box::new(GpgBackend::from_settings(settings).map_err(SignInitError::BackendConfig)?),
            Box::new(GpgsmBackend::from_settings(settings).map_err(SignInitError::BackendConfig)?),
            Box::new(SshBackend::from_settings(settings).map_err(SignInitError::BackendConfig)?),
            #[cfg(feature = "testing")]
            Box::new(TestSigningBackend),
        ];

        let main_backend = settings
            .signing_backend()
            .map_err(SignInitError::BackendConfig)?
            .map(|backend| {
                backends
                    .iter()
                    .position(|b| b.name() == backend)
                    .map(|i| backends.remove(i))
                    .ok_or(SignInitError::UnknownBackend(backend))
            })
            .transpose()?;

        Ok(Self::new(main_backend, backends))
    }

    /// Creates a signer with the given backends.
    pub fn new(
        main_backend: Option<Box<dyn SigningBackend>>,
        other_backends: Vec<Box<dyn SigningBackend>>,
    ) -> Self {
        let inner = CoreSigner::new(main_backend, other_backends);
        Self { inner }
    }

    /// Checks if the signer can sign, i.e. if a main backend is configured.
    pub fn can_sign(&self) -> bool {
        self.inner.can_sign()
    }

    /// This is just a pass-through to the main backend that unconditionally
    /// creates a signature.
    pub fn sign(&self, data: &[u8], key: Option<&str>) -> SignResult<Vec<u8>> {
        self.inner.sign(data, key)
    }

    /// Looks for backend that can verify the signature and returns the result
    /// of its verification.
    pub fn verify(
        &self,
        commit_id: &CommitId,
        data: &[u8],
        signature: &[u8],
    ) -> SignResult<Verification> {
        self.inner.verify(commit_id, data, signature)
    }
}
