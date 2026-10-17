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

//! Running the user's text editor.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::io::Write as _;
use std::num::NonZero;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitStatus;

use itertools::Itertools as _;
use jj_lib::config::ConfigGetError;
use jj_lib::converge::ConvergeError;
use jj_lib::file_util::IoResultExt as _;
use jj_lib::file_util::PathError;
use jj_lib::settings::UserSettings;
use thiserror::Error;

use crate::config_util::CommandNameAndArgs;
use crate::config_util::find_all_variables;

/// 1-based line number.
pub type LineNumber = NonZero<u32>;

/// Error from running the text editor.
#[derive(Debug, Error)]
pub enum TextEditError {
    /// The editor couldn't be started.
    #[error("Failed to run editor '{name}'")]
    FailedToRun {
        /// The name of the editor command.
        name: String,
        /// The underlying error.
        source: io::Error,
    },
    /// The editor exited with a failure status.
    #[error("Editor '{command}' exited with {status}")]
    ExitStatus {
        /// The editor command.
        command: String,
        /// The exit status.
        status: ExitStatus,
    },
}

/// Error from editing a temporary file with [`TextEditor::edit_str()`].
#[derive(Debug, Error)]
#[error("Failed to edit {name}", name = name.as_deref().unwrap_or("file"))]
pub struct TempTextEditError {
    /// The underlying error.
    #[source]
    pub error: Box<dyn std::error::Error + Send + Sync>,
    /// Short description of the edited content.
    pub name: Option<String>,
    /// Path to the temporary file.
    pub path: Option<PathBuf>,
}

impl From<TempTextEditError> for ConvergeError {
    fn from(err: TempTextEditError) -> Self {
        Self::Other(Box::new(err))
    }
}

impl TempTextEditError {
    fn new(error: Box<dyn std::error::Error + Send + Sync>, path: Option<PathBuf>) -> Self {
        Self {
            error,
            name: None,
            path,
        }
    }

    /// Adds short description of the edited content.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

/// Configured text editor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextEditor {
    editor: CommandNameAndArgs,
}

impl TextEditor {
    /// Loads the editor configured in `ui.editor`.
    pub fn from_settings(settings: &UserSettings) -> Result<Self, ConfigGetError> {
        let editor = settings.get("ui.editor")?;
        Ok(Self { editor })
    }

    /// Opens the given `path` in editor.
    pub fn edit_file(&self, path: impl AsRef<Path>, line: LineNumber) -> Result<(), TextEditError> {
        let path = path.as_ref();
        let (name, args) = self.editor.split_name_and_args();
        let vars: HashMap<&str, OsString> =
            HashMap::from([("path", path.into()), ("line", line.to_string().into())]);
        let mut cmd = self.editor.to_command_with_variables(&vars);
        if !find_all_variables(&args).contains("path") {
            cmd.arg(path);
        }
        tracing::info!(?cmd, "running editor");
        let status = cmd.status().map_err(|source| TextEditError::FailedToRun {
            name: name.into_owned(),
            source,
        })?;
        if status.success() {
            Ok(())
        } else {
            let command = self.editor.to_string();
            Err(TextEditError::ExitStatus { command, status })
        }
    }

    /// Writes the given `content` to temporary file and opens it in editor.
    pub fn edit_str(
        &self,
        content: impl AsRef<[u8]>,
        suffix: Option<&str>,
        line: LineNumber,
    ) -> Result<String, TempTextEditError> {
        let path = self
            .write_temp_file(content.as_ref(), suffix)
            .map_err(|err| TempTextEditError::new(err.into(), None))?;
        self.edit_file(&path, line)
            .map_err(|err| TempTextEditError::new(err.into(), Some(path.clone())))?;
        let edited = fs::read_to_string(&path)
            .context(&path)
            .map_err(|err| TempTextEditError::new(err.into(), Some(path.clone())))?;
        // Delete the file only if everything went well.
        fs::remove_file(path).ok();
        Ok(edited)
    }

    fn write_temp_file(&self, content: &[u8], suffix: Option<&str>) -> Result<PathBuf, PathError> {
        let dir = tempfile::env::temp_dir();
        let mut file = tempfile::Builder::new()
            .prefix("editor-")
            .suffix(suffix.unwrap_or(""))
            .tempfile_in(&dir)
            .context(&dir)?;
        file.write_all(content).context(file.path())?;
        let (_, path) = file
            .keep()
            .or_else(|err| Err(err.error).context(err.file.path()))?;
        Ok(path)
    }
}
