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

//! Filesystem monitor tool interface.
//!
//! Interfaces with a filesystem monitor tool to efficiently query for
//! filesystem updates, without having to crawl the entire working copy. This is
//! particularly useful for large working copies, or for working copies for
//! which it's expensive to materialize files, such those backed by a network or
//! virtualized filesystem.

#![warn(missing_docs)]

use std::error::Error;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;

use async_trait::async_trait;
use prost::Message as _;

use crate::config::ConfigGetError;
use crate::settings::UserSettings;

/// An opaque clock identifying the state observed by a filesystem monitor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FsmonitorClock {
    monitor_name: String,
    value: Vec<u8>,
}

impl FsmonitorClock {
    /// Creates a clock value owned by the named monitor implementation.
    pub fn new(monitor_name: impl Into<String>, value: Vec<u8>) -> Self {
        Self {
            monitor_name: monitor_name.into(),
            value,
        }
    }

    /// Returns the clock payload if it belongs to `monitor_name`.
    pub fn value_for(&self, monitor_name: &str) -> Option<&[u8]> {
        (self.monitor_name == monitor_name).then_some(&self.value)
    }
}

impl From<crate::protos::local_working_copy::FsmonitorClock> for FsmonitorClock {
    fn from(clock: crate::protos::local_working_copy::FsmonitorClock) -> Self {
        Self::new(clock.monitor_name, clock.value)
    }
}

impl From<FsmonitorClock> for crate::protos::local_working_copy::FsmonitorClock {
    fn from(clock: FsmonitorClock) -> Self {
        Self {
            monitor_name: clock.monitor_name,
            value: clock.value,
        }
    }
}

impl From<crate::protos::local_working_copy::WatchmanClock> for FsmonitorClock {
    fn from(clock: crate::protos::local_working_copy::WatchmanClock) -> Self {
        Self::new("watchman", clock.encode_to_vec())
    }
}

/// The result of querying a [`Fsmonitor`].
#[derive(Debug)]
pub enum FsmonitorQueryResult {
    /// No monitor state is available, so jj must scan the full working copy.
    Unmonitored,
    /// The monitor requires a full scan and supplies the clock it represents.
    FullScan {
        /// Clock to persist after the full scan succeeds.
        clock: FsmonitorClock,
    },
    /// The monitor supplies a complete incremental set of changed paths.
    Incremental {
        /// Clock to persist after the incremental scan succeeds.
        clock: FsmonitorClock,
        /// Paths which may have changed since the previous clock.
        changed_files: Vec<PathBuf>,
    },
}

/// Error returned by a filesystem monitor.
pub type FsmonitorError = Box<dyn Error + Send + Sync>;

/// Marks a filesystem-monitor error which cannot safely be handled by falling
/// back to a full working-copy scan.
#[derive(Debug)]
pub struct FatalFsmonitorError {
    source: FsmonitorError,
}

impl FatalFsmonitorError {
    /// Wraps an unrecoverable filesystem-monitor error.
    pub fn new(source: impl Error + Send + Sync + 'static) -> Self {
        Self {
            source: Box::new(source),
        }
    }
}

impl fmt::Display for FatalFsmonitorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.source.fmt(formatter)
    }
}

impl Error for FatalFsmonitorError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// Supplies paths that may have changed since the previous working-copy scan.
#[async_trait]
pub trait Fsmonitor: std::fmt::Debug + Send + Sync {
    /// Queries paths changed since `previous_clock`.
    ///
    /// Paths in an incremental result are relative to `working_copy_path`.
    /// Ordinary errors cause a safe full-scan fallback. Return a
    /// [`FatalFsmonitorError`] for configuration and other unrecoverable errors
    /// which should be reported to the caller.
    async fn query_changed_files(
        &self,
        working_copy_path: &Path,
        previous_clock: Option<&FsmonitorClock>,
    ) -> Result<FsmonitorQueryResult, FsmonitorError>;
}

/// Filesystem monitor which always requests a full working-copy scan.
#[derive(Debug, Default)]
pub struct NoFsmonitor;

#[async_trait]
impl Fsmonitor for NoFsmonitor {
    async fn query_changed_files(
        &self,
        _working_copy_path: &Path,
        _previous_clock: Option<&FsmonitorClock>,
    ) -> Result<FsmonitorQueryResult, FsmonitorError> {
        Ok(FsmonitorQueryResult::Unmonitored)
    }
}

/// Config for Watchman filesystem monitor (<https://facebook.github.io/watchman/>).
#[derive(Eq, PartialEq, Clone, Debug)]
pub struct WatchmanConfig {
    /// Whether to use triggers to monitor for changes in the background.
    pub register_trigger: bool,
}

/// The recognized kinds of filesystem monitors.
#[derive(Eq, PartialEq, Clone, Debug)]
pub enum FsmonitorSettings {
    /// The Watchman filesystem monitor (<https://facebook.github.io/watchman/>).
    Watchman(WatchmanConfig),

    /// Only used in tests.
    Test {
        /// The set of changed files to pretend that the filesystem monitor is
        /// reporting.
        changed_files: Vec<PathBuf>,
    },

    /// No filesystem monitor. This is the default if nothing is configured, but
    /// also makes it possible to turn off the monitor on a case-by-case basis
    /// when the user gives an option like `--config=fsmonitor.backend=none`;
    /// useful when e.g. doing analysis of snapshot performance.
    None,
}

impl FsmonitorSettings {
    /// Creates an `FsmonitorSettings` from a `config`.
    pub fn from_settings(settings: &UserSettings) -> Result<Self, ConfigGetError> {
        let name = "fsmonitor.backend";
        match settings.get_string(name)?.as_ref() {
            "watchman" => Ok(Self::Watchman(WatchmanConfig {
                register_trigger: settings
                    .get_bool("fsmonitor.watchman.register-snapshot-trigger")?,
            })),
            "test" => Err(ConfigGetError::Type {
                name: name.to_owned(),
                error: "Cannot use test fsmonitor in real repository".into(),
                source_path: None,
            }),
            "none" => Ok(Self::None),
            other => Err(ConfigGetError::Type {
                name: name.to_owned(),
                error: format!("Unknown fsmonitor kind: {other}").into(),
                source_path: None,
            }),
        }
    }
}

/// Filesystem monitor backed by Watchman.
#[derive(Clone, Debug)]
pub struct WatchmanFsmonitor {
    #[cfg_attr(not(feature = "watchman"), allow(dead_code))]
    config: WatchmanConfig,
}

impl WatchmanFsmonitor {
    /// Creates a Watchman filesystem monitor.
    pub fn new(config: WatchmanConfig) -> Self {
        Self { config }
    }

    /// Queries Watchman using the generic persisted filesystem-monitor clock.
    #[cfg(feature = "watchman")]
    pub async fn query(
        &self,
        working_copy_path: &Path,
        previous_clock: Option<&FsmonitorClock>,
    ) -> Result<(watchman::Clock, Option<Vec<PathBuf>>), watchman::Error> {
        let previous_clock = previous_clock
            .and_then(|clock| clock.value_for("watchman"))
            .and_then(|value| crate::protos::local_working_copy::WatchmanClock::decode(value).ok())
            .and_then(|clock| watchman::Clock::try_from(clock).ok());
        let query = async || {
            let monitor = watchman::Fsmonitor::init(working_copy_path, &self.config).await?;
            monitor.query_changed_files(previous_clock).await
        };
        match tokio::runtime::Handle::try_current() {
            Ok(_) => Ok(query().await?),
            Err(_) => Ok(tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(watchman::Error::RuntimeCreationError)?
                .block_on(query())?),
        }
    }

    /// Returns whether the Watchman snapshot trigger is registered.
    #[cfg(feature = "watchman")]
    pub async fn is_trigger_registered(
        &self,
        working_copy_path: &Path,
    ) -> Result<bool, watchman::Error> {
        let query = async || {
            let monitor = watchman::Fsmonitor::init(working_copy_path, &self.config).await?;
            monitor.is_trigger_registered().await
        };
        match tokio::runtime::Handle::try_current() {
            Ok(_) => Ok(query().await?),
            Err(_) => Ok(tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(watchman::Error::RuntimeCreationError)?
                .block_on(query())?),
        }
    }
}

#[async_trait]
impl Fsmonitor for WatchmanFsmonitor {
    async fn query_changed_files(
        &self,
        working_copy_path: &Path,
        previous_clock: Option<&FsmonitorClock>,
    ) -> Result<FsmonitorQueryResult, FsmonitorError> {
        #[cfg(feature = "watchman")]
        {
            let (clock, changed_files) = self.query(working_copy_path, previous_clock).await?;
            let clock: crate::protos::local_working_copy::WatchmanClock = clock.into();
            let clock = FsmonitorClock::new("watchman", clock.encode_to_vec());
            Ok(match changed_files {
                Some(changed_files) => FsmonitorQueryResult::Incremental {
                    clock,
                    changed_files,
                },
                None => FsmonitorQueryResult::FullScan { clock },
            })
        }
        #[cfg(not(feature = "watchman"))]
        {
            let _ = (working_copy_path, previous_clock);
            let error = std::io::Error::other(
                "Cannot query Watchman because jj was compiled without the watchman feature \
                 (consider disabling `fsmonitor.backend`)",
            );
            Err(Box::new(FatalFsmonitorError::new(error)))
        }
    }
}

/// Filesystem monitor integration using Watchman
/// (<https://facebook.github.io/watchman/>). Requires `watchman` to already be
/// installed on the system.
#[cfg(feature = "watchman")]
pub mod watchman {
    use std::path::Path;
    use std::path::PathBuf;

    use itertools::Itertools as _;
    use thiserror::Error;
    use tracing::info;
    use tracing::instrument;
    use watchman_client::expr;
    use watchman_client::prelude::Clock as InnerClock;
    use watchman_client::prelude::ClockSpec;
    use watchman_client::prelude::NameOnly;
    use watchman_client::prelude::QueryRequestCommon;
    use watchman_client::prelude::QueryResult;
    use watchman_client::prelude::TriggerRequest;

    /// Represents an instance in time from the perspective of the filesystem
    /// monitor.
    ///
    /// This can be used to perform incremental queries. When making a query,
    /// the result will include an associated "clock" representing the time
    /// that the query was made. By passing the same clock into a future
    /// query, we inform the filesystem monitor that we only wish to get
    /// changed files since the previous point in time.
    #[derive(Clone, Debug)]
    pub struct Clock(InnerClock);

    /// Error returned when a Watchman clock protobuf has no clock value.
    #[derive(Debug, Error)]
    #[error("Watchman clock protobuf has no clock value")]
    pub struct MissingWatchmanClockValue;

    impl TryFrom<crate::protos::local_working_copy::WatchmanClock> for Clock {
        type Error = MissingWatchmanClockValue;

        fn try_from(
            clock: crate::protos::local_working_copy::WatchmanClock,
        ) -> Result<Self, Self::Error> {
            use crate::protos::local_working_copy::watchman_clock::WatchmanClock;
            let watchman_clock = clock.watchman_clock.ok_or(MissingWatchmanClockValue)?;
            let clock = match watchman_clock {
                WatchmanClock::StringClock(string_clock) => {
                    InnerClock::Spec(ClockSpec::StringClock(string_clock))
                }
                WatchmanClock::UnixTimestamp(unix_timestamp) => {
                    InnerClock::Spec(ClockSpec::UnixTimestamp(unix_timestamp))
                }
            };
            Ok(Self(clock))
        }
    }

    impl From<Clock> for crate::protos::local_working_copy::WatchmanClock {
        fn from(clock: Clock) -> Self {
            use crate::protos::local_working_copy::watchman_clock;
            let Clock(clock) = clock;
            let watchman_clock = match clock {
                InnerClock::Spec(ClockSpec::StringClock(string_clock)) => {
                    watchman_clock::WatchmanClock::StringClock(string_clock)
                }
                InnerClock::Spec(ClockSpec::UnixTimestamp(unix_timestamp)) => {
                    watchman_clock::WatchmanClock::UnixTimestamp(unix_timestamp)
                }
                InnerClock::ScmAware(_) => {
                    unimplemented!("SCM-aware Watchman clocks not supported")
                }
            };
            Self {
                watchman_clock: Some(watchman_clock),
            }
        }
    }

    #[expect(missing_docs)]
    #[derive(Debug, Error)]
    pub enum Error {
        #[error("Could not connect to Watchman")]
        WatchmanConnectError(#[source] watchman_client::Error),

        #[error("Could not canonicalize working copy root path")]
        CanonicalizeRootError(#[source] std::io::Error),

        #[error("Watchman failed to resolve the working copy root path")]
        ResolveRootError(#[source] watchman_client::Error),

        #[error("Failed to query Watchman")]
        WatchmanQueryError(#[source] watchman_client::Error),

        #[error("Failed to register Watchman trigger")]
        WatchmanTriggerError(#[source] watchman_client::Error),

        #[error("Failed to create a runtime for Watchman")]
        RuntimeCreationError(#[source] std::io::Error),
    }

    /// Handle to the underlying Watchman instance.
    pub struct Fsmonitor {
        client: watchman_client::Client,
        resolved_root: watchman_client::ResolvedRoot,
    }

    impl Fsmonitor {
        /// Initialize the Watchman filesystem monitor. If it's not already
        /// running, this will start it and have it crawl the working
        /// copy to build up its in-memory representation of the
        /// filesystem, which may take some time.
        #[instrument]
        pub async fn init(
            working_copy_path: &Path,
            config: &super::WatchmanConfig,
        ) -> Result<Self, Error> {
            info!("Initializing Watchman filesystem monitor...");
            let connector = watchman_client::Connector::new();
            let client = connector
                .connect()
                .await
                .map_err(Error::WatchmanConnectError)?;
            let working_copy_root = watchman_client::CanonicalPath::canonicalize(working_copy_path)
                .map_err(Error::CanonicalizeRootError)?;
            let resolved_root = client
                .resolve_root(working_copy_root)
                .await
                .map_err(Error::ResolveRootError)?;

            let monitor = Self {
                client,
                resolved_root,
            };

            // Registering the trigger causes an unconditional evaluation of the query, so
            // test if it is already registered first.
            if !config.register_trigger {
                monitor.unregister_trigger().await?;
            } else if !monitor.is_trigger_registered().await? {
                monitor.register_trigger().await?;
            }
            Ok(monitor)
        }

        /// Query for changed files since the previous point in time.
        ///
        /// The returned list of paths is relative to the `working_copy_path`.
        /// If it is `None`, then the caller must crawl the entire working copy
        /// themselves.
        #[instrument(skip(self))]
        pub async fn query_changed_files(
            &self,
            previous_clock: Option<Clock>,
        ) -> Result<(Clock, Option<Vec<PathBuf>>), Error> {
            // TODO: might be better to specify query options by caller, but we
            // shouldn't expose the underlying watchman API too much.
            info!("Querying Watchman for changed files...");
            let QueryResult {
                version: _,
                is_fresh_instance,
                files,
                clock,
                state_enter: _,
                state_leave: _,
                state_metadata: _,
                saved_state_info: _,
                debug: _,
            }: QueryResult<NameOnly> = self
                .client
                .query(
                    &self.resolved_root,
                    QueryRequestCommon {
                        since: previous_clock.map(|Clock(clock)| clock),
                        expression: Some(self.build_exclude_expr()),
                        ..Default::default()
                    },
                )
                .await
                .map_err(Error::WatchmanQueryError)?;

            let clock = Clock(clock);
            if is_fresh_instance {
                // The Watchman documentation states that if it was a fresh
                // instance, we need to delete any tree entries that didn't appear
                // in the returned list of changed files. For now, the caller will
                // handle this by manually crawling the working copy again.
                Ok((clock, None))
            } else {
                let paths = files
                    .unwrap_or_default()
                    .into_iter()
                    .map(|NameOnly { name }| name.into_inner())
                    .collect_vec();
                Ok((clock, Some(paths)))
            }
        }

        /// Return whether or not a trigger has been registered already.
        #[instrument(skip(self))]
        pub async fn is_trigger_registered(&self) -> Result<bool, Error> {
            info!("Checking for an existing Watchman trigger...");
            Ok(self
                .client
                .list_triggers(&self.resolved_root)
                .await
                .map_err(Error::WatchmanTriggerError)?
                .triggers
                .iter()
                .any(|t| t.name == "jj-background-monitor"))
        }

        /// Register trigger for changed files.
        #[instrument(skip(self))]
        async fn register_trigger(&self) -> Result<(), Error> {
            info!("Registering Watchman trigger...");
            let null = if cfg!(windows) { ">NUL" } else { ">/dev/null" };
            self.client
                .register_trigger(
                    &self.resolved_root,
                    TriggerRequest {
                        name: "jj-background-monitor".to_string(),
                        command: vec![
                            "jj".to_string(),
                            "--quiet".to_string(),
                            "util".to_string(),
                            "snapshot".to_string(),
                        ],
                        expression: Some(self.build_exclude_expr()),
                        stderr: Some(null.into()),
                        stdout: Some(null.into()),
                        ..Default::default()
                    },
                )
                .await
                .map_err(Error::WatchmanTriggerError)?;
            Ok(())
        }

        /// Register trigger for changed files.
        #[instrument(skip(self))]
        async fn unregister_trigger(&self) -> Result<(), Error> {
            info!("Unregistering Watchman trigger...");
            self.client
                .remove_trigger(&self.resolved_root, "jj-background-monitor")
                .await
                .map_err(Error::WatchmanTriggerError)?;
            Ok(())
        }

        /// Build an exclude expr for `working_copy_path`.
        fn build_exclude_expr(&self) -> expr::Expr {
            // TODO: consider parsing `.gitignore`.
            let exclude_dirs = [Path::new(".git"), Path::new(".jj")];
            let excludes = itertools::chain(
                // the directories themselves
                [expr::Expr::Name(expr::NameTerm {
                    paths: exclude_dirs.iter().map(|&name| name.to_owned()).collect(),
                    wholename: true,
                })],
                // and all files under the directories
                exclude_dirs.iter().map(|&name| {
                    expr::Expr::DirName(expr::DirNameTerm {
                        path: name.to_owned(),
                        depth: None,
                    })
                }),
            )
            .collect();
            expr::Expr::Not(Box::new(expr::Expr::Any(excludes)))
        }
    }
}
