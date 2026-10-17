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

use jj_lib::config::ConfigLoadError;
use jj_lib::secure_config::SecureConfigError;
use thiserror::Error;

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
