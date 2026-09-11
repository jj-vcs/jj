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

//! Contains helpers to create a [`Signer`] from the given [`UserSettings`].

use thiserror::Error;

use crate::config::ConfigGetError;
use crate::gpg_signing::GpgBackend;
use crate::gpg_signing::GpgsmBackend;
use crate::settings::UserSettings;
use crate::signing::Signer;
use crate::signing::SigningBackend;
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

/// Creates a signer based on user settings. Uses all known backends, and
/// chooses one of them to be used for signing depending on the config.
pub fn signer_from_settings(settings: &UserSettings) -> Result<Signer, SignInitError> {
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

    Ok(Signer::new(main_backend, backends))
}
