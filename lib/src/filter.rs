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

//! Support for the hooks on file contents when snapshotting and updating a
//! working copy, similar to the filter gitattributes feature.

use std::collections::HashMap;
use std::io::Cursor;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Output;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use bstr::BString;
use bstr::ByteSlice as _;
use itertools::Itertools as _;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt as _;

use crate::config::ConfigGetError;
use crate::gitattributes::GitAttributes;
use crate::gitattributes::State;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::settings::UserSettings;

type FilterError = Box<dyn std::error::Error + Send + Sync>;

/// The definition of a filter driver.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all(deserialize = "kebab-case"))]
pub struct FilterDriver {
    /// The command executes on snapshot, i.e., write the file from the disk to
    /// the store.
    ///
    /// The `$path` substring will be replaced with the path of the file the
    /// filter is working on. An empty [`Vec`] means this field is left
    /// unspecified.
    #[serde(default)]
    clean: Vec<String>,

    /// The command executes on update, i.e., write the file from the store to
    /// the disk.
    ///
    /// The `$path` substring will be replaced with the path of the file the
    /// filter is working on. An empty [`Vec`] means this field is left
    /// unspecified.
    #[serde(default)]
    smudge: Vec<String>,

    /// Whether the `jj` command fails if the filter command fails.
    ///
    /// If this is true, and the filter command fails, the `jj` command fails
    /// and leaves the working copy in the stale state. If this is false, the
    /// filter command failure makes the filter a no-op passthru.
    #[serde(default)]
    required: bool,
}

/// Filter user settings.
#[derive(Debug, Clone)]
pub struct FilterSettings {
    /// A killer switch on whether filter gitattributes should be enabled.
    ///
    /// When the value is `false`, the implementation shouldn't read any
    /// `gitattributes` files so that the user who doesn't need this feature
    /// pays little cost if not zero.
    pub enabled: bool,

    /// The list of filter drivers defined.
    ///
    /// The key is the name of the filter driver. We use this name to specify
    /// which filter should apply to a file.
    pub drivers: HashMap<String, FilterDriver>,
}

impl FilterSettings {
    pub(crate) fn try_from_settings(user_settings: &UserSettings) -> Result<Self, ConfigGetError> {
        let filter_drivers_key = "git.filter.drivers";
        let drivers = user_settings
            .table_keys(filter_drivers_key)
            .map(|name| {
                Ok((
                    name.to_owned(),
                    user_settings
                        .get::<FilterDriver>(format!("{filter_drivers_key}.{name}").as_str())?,
                ))
            })
            .try_collect()?;
        Ok(Self {
            enabled: user_settings.get_bool("git.filter.enabled")?,
            drivers,
        })
    }
}

/// Indicate whether the filter is a clean filter or a smudge filter.
///
/// Used in place where the same logic is shared regardless of the filter type,
/// e.g., [`IgnoreReason`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FilterType {
    Clean,
    Smudge,
}

impl std::fmt::Display for FilterType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::Clean => write!(f, "clean"),
            Self::Smudge => write!(f, "smudge"),
        }
    }
}

/// The error type for the filter child process operation.
#[derive(Debug, Clone, thiserror::Error)]
pub enum CommandError {
    /// Failed to spawn the filter process.
    #[error("Failed to spawn the filter process: {0}")]
    ProcessCreationError(Arc<std::io::Error>),
    /// The output worker thread fails to wait for the child process and send
    /// the output.
    ///
    /// This error indicates that the output worker thread returns prematurely
    /// before it can send the output of the child process back to the main
    /// thread.
    #[error("The output worker thread fails to wait for the child process and send the output")]
    OutputWorkerThreadFailed,
    /// The stdin worker thread fails to send all the message to the child
    /// process.
    ///
    /// Similar to [`Self::OutputWorkerThreadFailed`], but this error is about
    /// the stdin worker thread.
    #[error("The stdin worker thread fails to send all the message to the child process")]
    StdinWorkerThreadFailed { stderr: Vec<u8> },
    /// The filter process exits with unsuccessful status.
    #[error("The filter process exits with {status}")]
    BadExitStatus { status: ExitStatus, stderr: Vec<u8> },
}

/// Reason why we don't apply the filter to the file.
#[derive(Debug, Clone, thiserror::Error)]
pub enum IgnoreReason {
    /// The user setting disables the filter feature.
    #[error("Filters are disabled by settings")]
    DisabledBySettings,
    /// No filter is associated with the file.
    ///
    /// This happens if `FilterNameProvider::get_filter_name` returns [`None`].
    #[error("No filter is associated with the file")]
    FilterNotDefined,
    /// The filter driver associated with the path can't be found.
    #[error("The filter driver {filter_name} is not defined")]
    DriverNotFound {
        /// The name of the expected filter driver.
        filter_name: String,
    },
    /// No command is defined for clean and/or smudge.
    #[error("The {filter_name} filter doesn't define the command for {filter_type}")]
    CommandNotDefined {
        /// The name of the filter driver.
        filter_name: String,
        /// The type of the command. Clean or smudge.
        filter_type: FilterType,
    },
    /// Failed to execute the filter command.
    #[error("The filter command fails: {err}")]
    FilterCommandFailed {
        /// The path to the file when the filter command fails.
        path: RepoPathBuf,
        /// The name of the filter driver that fails.
        filter_name: String,
        /// The source of the filter command failure.
        #[source]
        err: CommandError,
    },
}

/// The [`FilterNameProvider`] trait provides the filter name associated with a
/// path.
#[async_trait]
pub trait FilterNameProvider: Send {
    /// Return the associated filter name given a path.
    async fn get_filter_name(&self, path: &RepoPath) -> Result<Option<BString>, FilterError>;
}

#[async_trait]
impl FilterNameProvider for GitAttributes {
    /// Query the associated filter driver from a `gitattributes` file.
    ///
    /// If the `filter` `gitattributes` is not set to a value, [`None`] is
    /// returned.
    async fn get_filter_name(&self, path: &RepoPath) -> Result<Option<BString>, FilterError> {
        let attr_name = "filter";
        let mut attributes = self.search(path, &[attr_name]).await?;
        let Some(State::Value(driver_name)) = attributes.remove(attr_name) else {
            tracing::trace!(
                "No filter git attributes found for {}. No filter conversion will be applied.",
                path.as_internal_file_string()
            );
            return Ok(None);
        };
        Ok(Some(driver_name.into()))
    }
}

/// The type to apply the filter conversion.
pub struct FilterStrategy {
    working_copy_path: PathBuf,
    settings: FilterSettings,
    /// An abstraction layer for child process creation and IPC. This allows us
    /// to mock process creation and write unit tests that don't spawn actual
    /// child processes.
    proc_async_adapter: Box<dyn ProcessAsyncAdapter>,
}

impl FilterStrategy {
    /// Create a [`FilterStrategy`].
    ///
    /// * `working_copy_path`: The path to the working copy root. Used to set
    ///   the current working directory of the filter command.
    /// * `settings`: The user settings that control the behavior of the filter
    ///   feature.
    pub fn new(working_copy_path: impl AsRef<Path>, settings: FilterSettings) -> Self {
        if settings.enabled {
            tracing::trace!("The filter gitattributes support is enabled.");
        } else {
            tracing::trace!(
                "The filter gitattributes support is disabled. No filter conversion will be \
                 applied."
            );
        }
        let mut proc_async_adapter = StdChildAsyncAdapter::default();
        proc_async_adapter.worker_thread_name_prefix("filter");
        Self {
            working_copy_path: working_copy_path.as_ref().to_owned(),
            settings,
            proc_async_adapter: Box::new(proc_async_adapter),
        }
    }

    async fn convert<'a, F>(
        &self,
        mut contents: impl AsyncRead + Send + Unpin + 'a,
        path: &RepoPath,
        filter_name_provider: &F,
        filter_type: FilterType,
    ) -> Result<(Box<dyn AsyncRead + Send + Unpin + 'a>, Option<IgnoreReason>), FilterError>
    where
        F: FilterNameProvider + Unpin + 'a,
    {
        if !self.settings.enabled {
            return Ok((Box::new(contents), Some(IgnoreReason::DisabledBySettings)));
        }
        let Some(driver_name) = filter_name_provider.get_filter_name(path).await? else {
            return Ok((Box::new(contents), Some(IgnoreReason::FilterNotDefined)));
        };
        let driver_name = match driver_name.to_str() {
            Ok(driver_name) => driver_name,
            // The filter drivers are defined in a toml file, which only accepts UTF-8 string. A non
            // UTF-8 string won't match any filter driver name.
            Err(e) => {
                tracing::trace!(
                    "The filter driver name {driver_name:?} is not a valid UTF-8 string: {e}."
                );
                return Ok((
                    Box::new(contents),
                    Some(IgnoreReason::DriverNotFound {
                        filter_name: driver_name.to_str_lossy().into_owned(),
                    }),
                ));
            }
        };
        let Some(filter_driver) = self.settings.drivers.get(driver_name) else {
            tracing::trace!(
                "No filter driver named {} found for {}. No filter conversion will be applied.",
                driver_name,
                path.as_internal_file_string()
            );
            return Ok((
                Box::new(contents),
                Some(IgnoreReason::DriverNotFound {
                    filter_name: driver_name.to_string(),
                }),
            ));
        };
        let command_template = match filter_type {
            FilterType::Clean => &filter_driver.clean,
            FilterType::Smudge => &filter_driver.smudge,
        };
        let Some(mut command) = self.create_filter_command(command_template, path) else {
            tracing::trace!(
                "The {} filter driver doesn't have a {filter_type} filter. No filter conversion \
                 will be applied for {}.",
                driver_name,
                path.as_internal_file_string()
            );
            return Ok((
                Box::new(contents),
                Some(IgnoreReason::CommandNotDefined {
                    filter_name: driver_name.to_string(),
                    filter_type,
                }),
            ));
        };
        let mut buf = Vec::new();
        contents
            .read_to_end(&mut buf)
            .await
            .map_err(|e| Box::new(e) as FilterError)?;

        let output = self
            .proc_async_adapter
            .spawn_and_wait_with_output(&buf, &mut command)
            .await
            .and_then(|output| {
                if !output.status.success() {
                    return Err(CommandError::BadExitStatus {
                        status: output.status,
                        stderr: output.stderr,
                    });
                }
                Ok(output)
            });
        let err = match output {
            Ok(res) => return Ok((Box::new(Cursor::new(res.stdout)), None)),
            Err(err) => err,
        };
        tracing::trace!(
            "The filter command fails on the file {}: {err}.",
            path.as_internal_file_string()
        );
        if filter_driver.required {
            return Err(Box::new(IgnoreReason::FilterCommandFailed {
                path: path.to_owned(),
                filter_name: driver_name.to_owned(),
                err,
            }));
        }
        Ok((
            Box::new(Cursor::new(buf)),
            Some(IgnoreReason::FilterCommandFailed {
                path: path.to_owned(),
                filter_name: driver_name.to_owned(),
                err,
            }),
        ))
    }

    pub async fn convert_to_store<'a, F>(
        &self,
        contents: impl AsyncRead + Send + Unpin + 'a,
        path: &RepoPath,
        filter_name_provider: &F,
    ) -> Result<(Box<dyn AsyncRead + Send + Unpin + 'a>, Option<IgnoreReason>), FilterError>
    where
        F: FilterNameProvider + Unpin + 'a,
    {
        self.convert(contents, path, filter_name_provider, FilterType::Clean)
            .await
    }

    pub async fn convert_to_working_copy<'a, F>(
        &self,
        contents: impl AsyncRead + Send + Unpin + 'a,
        path: &RepoPath,
        filter_name_provider: &F,
    ) -> Result<(Box<dyn AsyncRead + Send + Unpin + 'a>, Option<IgnoreReason>), FilterError>
    where
        F: FilterNameProvider + Unpin + 'a,
    {
        self.convert(contents, path, filter_name_provider, FilterType::Smudge)
            .await
    }

    fn create_filter_command(&self, command: &[String], path: &RepoPath) -> Option<Command> {
        let executable = command.first()?;
        let args = command
            .get(1..)?
            .iter()
            .map(|arg| arg.replace("$path", path.as_internal_file_string()));
        let mut command = Command::new(executable);
        command.args(args);
        command
            .current_dir(&self.working_copy_path)
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .stdout(Stdio::piped());
        Some(command)
    }
}

#[async_trait]
trait ProcessAsyncAdapter: Send + Sync {
    fn worker_thread_name_prefix(&mut self, _name: &str) {}
    async fn spawn_and_wait_with_output(
        &self,
        stdin_contents: &[u8],
        command: &mut Command,
    ) -> Result<Output, CommandError>;
}

#[derive(Default, Debug)]
struct StdChildAsyncAdapter {
    worker_thread_name_prefix: Option<String>,
}

#[async_trait]
impl ProcessAsyncAdapter for StdChildAsyncAdapter {
    fn worker_thread_name_prefix(&mut self, name: &str) {
        self.worker_thread_name_prefix = Some(name.to_owned());
    }

    async fn spawn_and_wait_with_output(
        &self,
        stdin_contents: &[u8],
        command: &mut Command,
    ) -> Result<Output, CommandError> {
        let executable = command.get_program().display().to_string();
        let args = command
            .get_args()
            .map(|arg| arg.display().to_string())
            .collect_vec();
        let mut child = command
            .spawn()
            .map_err(|e| CommandError::ProcessCreationError(Arc::new(e)))?;
        tracing::trace!("Execute the command with executable {executable:?}, arguments: {args:?}.");
        fn try_kill_and_wait_for_child_process(child: &mut Child) {
            match child.kill() {
                Ok(()) => {
                    tracing::trace!("Successfully killed the child process.");
                    if let Err(e) = child.wait() {
                        tracing::warn!("Failed to wait for the killed child process: {e}.");
                    }
                }
                Err(e) => tracing::warn!("Failed to kill the child process: {e}."),
            }
        }

        let stdin_rx = match child.stdin.take() {
            Some(mut child_stdin) => {
                let (stdin_tx, stdin_rx) = tokio::sync::oneshot::channel::<()>();
                let worker_thread_name = self
                    .worker_thread_name_prefix
                    .as_ref()
                    .map(|prefix| format!("{prefix} stdin worker"))
                    .unwrap_or_else(|| "stdin worker".to_owned());
                let res = std::thread::scope(move |s| {
                    std::thread::Builder::new()
                        .name(worker_thread_name)
                        .spawn_scoped(s, move || {
                            if let Err(e) = child_stdin.write_all(stdin_contents) {
                                tracing::error!(
                                    "Failed to write all the contents to the stdin of the child \
                                     process: {e}."
                                );
                                return;
                            }
                            let res = stdin_tx.send(());
                            let error_message = "The original future is dropped before the stdin \
                                                 worker thread exits. It is likely that this \
                                                 thread is incorrectly leaked.";
                            debug_assert!(res.is_ok(), "{error_message}");
                            if res.is_err() {
                                tracing::warn!("{error_message}");
                            }
                        })
                        .map(|_| ())
                });
                if let Err(e) = res {
                    tracing::warn!("Failed to spawn the filter stdin worker thread.");
                    try_kill_and_wait_for_child_process(&mut child);
                    panic!("Failed to spawn the stdin worker threads: {e}.");
                }
                Some(stdin_rx)
            }
            None => {
                tracing::trace!("No child stdin pipe received, won't write to the child stdin.");
                None
            }
        };

        let child = Arc::new(Mutex::new(Some(child)));
        let (output_tx, output_rx) = tokio::sync::oneshot::channel();
        let worker_thread_name = self
            .worker_thread_name_prefix
            .as_ref()
            .map(|prefix| format!("{prefix} output worker"))
            .unwrap_or_else(|| "output worker".to_owned());
        let res = std::thread::Builder::new().name(worker_thread_name).spawn({
            let child = Arc::clone(&child);
            move || {
                let Some(child) = child
                    .lock()
                    .expect(
                        "The original caller thread panics when holding the lock, which is \
                         unlikely.",
                    )
                    .take()
                else {
                    tracing::error!(
                        "No child process received. Can't wait for the output of the child \
                         process."
                    );
                    return;
                };
                let output = match child.wait_with_output() {
                    Ok(output) => output,
                    Err(e) => {
                        tracing::warn!("Failed to wait for the child process to exit: {e}.");
                        return;
                    }
                };
                let res = output_tx.send(output);
                let error_message = "The original future is dropped before the output worker \
                                     thread exits. It is likely that this thread is incorrectly \
                                     leaked.";
                debug_assert!(res.is_ok(), "{error_message}");
                if res.is_err() {
                    tracing::warn!("{error_message}");
                }
            }
        });
        if let Err(e) = res {
            tracing::warn!("Failed to spawn the filter output worker thread.");
            let mut child = child
                .lock()
                .expect("The worker thread panics when holding the lock, which is unlikely.")
                .take()
                .expect("child should be some if we fail to spawn the output worker thread.");
            try_kill_and_wait_for_child_process(&mut child);
            panic!("Failed to spawn the stdout worker threads: {e}.");
        }
        // We should avoid await, panicking or return before we await output_rx to
        // ensure that the returned future is only ready when we make our best
        // effort to wait for the child process to avoid leaking resource.
        let Ok(output) = output_rx.await else {
            return Err(CommandError::OutputWorkerThreadFailed);
        };
        tracing::trace!(
            "The child process with executable {executable:?} arguments: {args:?} exits with {}.",
            output.status
        );
        if !output.stderr.is_empty() && tracing::enabled!(tracing::Level::TRACE) {
            tracing::trace!(
                "filter child process stderr:\n\n{}",
                output.stderr.to_str_lossy().as_ref()
            );
        }
        if let Some(stdin_rx) = stdin_rx
            && stdin_rx.await.is_err()
        {
            return Err(CommandError::StdinWorkerThreadFailed {
                stderr: output.stderr,
            });
        }
        Ok(output)
    }
}
