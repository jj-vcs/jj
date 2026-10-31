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

use std::path::PathBuf;

use crate::config::ConfigGetError;
use crate::settings::UserSettings;

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

/// Filesystem monitor integration using Watchman
/// (<https://facebook.github.io/watchman/>). Requires `watchman` to already be
/// installed on the system.
#[cfg(feature = "watchman")]
pub mod watchman {
    use std::env;
    use std::fs;
    use std::io;
    use std::io::ErrorKind;
    use std::io::Write as _;
    use std::path::Path;
    use std::path::PathBuf;

    use etcetera::BaseStrategy as _;
    use itertools::Itertools as _;
    use tempfile::NamedTempFile;
    use thiserror::Error;
    use tracing::Instrument as _;
    use tracing::info;
    use tracing::instrument;
    use watchman_client::expr;
    use watchman_client::prelude::Clock as InnerClock;
    use watchman_client::prelude::ClockSpec;
    use watchman_client::prelude::NameOnly;
    use watchman_client::prelude::QueryRequestCommon;
    use watchman_client::prelude::QueryResult;
    use watchman_client::prelude::TriggerRequest;

    const WATCHMAN_COMMAND: &str = "watchman";

    fn endpoint_cache_path() -> Option<PathBuf> {
        let strategy = etcetera::choose_base_strategy().ok()?;
        Some(strategy.cache_dir().join("jj").join("watchman-endpoint"))
    }

    fn read_cached_endpoint(path: &Path) -> Option<PathBuf> {
        match fs::read_to_string(path) {
            Ok(endpoint) => Some(endpoint.into()),
            Err(err) if err.kind() == ErrorKind::NotFound => None,
            Err(err) => {
                tracing::debug!(?path, ?err, "failed to read cached Watchman endpoint");
                None
            }
        }
    }

    fn write_cached_endpoint(path: &Path, endpoint: &Path) -> io::Result<()> {
        let cache_dir = path
            .parent()
            .ok_or_else(|| io::Error::other("Watchman cache path has no parent"))?;
        fs::create_dir_all(cache_dir)?;
        // Multiple jj processes can refresh this shared cache concurrently. Write to
        // a temporary file in the same directory, then atomically replace the cache.
        let mut temp_file = NamedTempFile::new_in(cache_dir)?;
        let endpoint = endpoint
            .to_str()
            .ok_or_else(|| io::Error::other("Watchman endpoint is not valid UTF-8"))?;
        temp_file.write_all(endpoint.as_bytes())?;
        crate::file_util::persist_temp_file(temp_file, path)?;
        Ok(())
    }

    fn discovery_error(reason: impl ToString, stderr: &[u8]) -> watchman_client::Error {
        watchman_client::Error::ConnectionDiscovery {
            watchman_path: PathBuf::from(WATCHMAN_COMMAND),
            reason: reason.to_string(),
            stderr: String::from_utf8_lossy(stderr).into_owned(),
        }
    }

    async fn discover_endpoint() -> Result<PathBuf, watchman_client::Error> {
        // Connector performs discovery internally but does not expose the resolved
        // endpoint. Run the same Watchman command here so the result can be cached.
        let output = tokio::process::Command::new(WATCHMAN_COMMAND)
            .args(["--output-encoding", "bser-v2", "get-sockname"])
            .output()
            .instrument(tracing::trace_span!("run watchman get-sockname"))
            .await
            .map_err(|err| discovery_error(err, &[]))?;
        // Use Watchman's native encoding, matching watchman_client's own discovery.
        let response: watchman_client::pdu::GetSockNameResponse =
            serde_bser::from_slice(&output.stdout)
                .map_err(|err| discovery_error(err, &output.stderr))?;
        if let Some(message) = response.error {
            return Err(watchman_client::Error::WatchmanServerError {
                message,
                command: "get-sockname".to_owned(),
            });
        }
        let response_debug = format!("{response:#?}");
        response
            .sockname
            .ok_or_else(|| watchman_client::Error::MissingField {
                fieldname: "sockname",
                command: "get-sockname".to_owned(),
                response: response_debug,
            })
    }

    async fn connect() -> Result<watchman_client::Client, watchman_client::Error> {
        // A nonempty WATCHMAN_SOCK is an explicit user override. Preserve
        // watchman_client's existing behavior and do not read or update the cache.
        if env::var_os("WATCHMAN_SOCK").is_some_and(|value| !value.is_empty()) {
            return watchman_client::Connector::new().connect().await;
        }

        let cache_path = endpoint_cache_path();
        if let Some(endpoint) = cache_path.as_deref().and_then(read_cached_endpoint) {
            // Watchman can replace its socket when the server restarts. Treat the
            // cached endpoint as a hint and verify that it belongs to a responsive
            // Watchman server before returning the client.
            match watchman_client::Connector::new()
                .unix_domain_socket(&endpoint)
                .connect()
                .await
            {
                Ok(client) => match client.version().await {
                    Ok(_) => return Ok(client),
                    Err(err) => tracing::debug!(
                        ?endpoint,
                        ?err,
                        "cached Watchman endpoint failed validation"
                    ),
                },
                Err(err) => tracing::debug!(
                    ?endpoint,
                    ?err,
                    "failed to connect to cached Watchman endpoint"
                ),
            }
        }

        // The cached endpoint is absent or stale. Ask Watchman for its current
        // endpoint and validate the connection before making it the new cache entry.
        let endpoint = discover_endpoint().await?;
        let client = watchman_client::Connector::new()
            .unix_domain_socket(&endpoint)
            .connect()
            .await?;
        client.version().await?;
        // Caching is only an optimization. A cache write failure must not make an
        // otherwise valid Watchman connection unusable.
        if let Some(path) = cache_path
            && let Err(err) = write_cached_endpoint(&path, &endpoint)
        {
            tracing::debug!(?path, ?err, "failed to cache Watchman endpoint");
        }
        Ok(client)
    }

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

    impl From<crate::protos::local_working_copy::WatchmanClock> for Clock {
        fn from(clock: crate::protos::local_working_copy::WatchmanClock) -> Self {
            use crate::protos::local_working_copy::watchman_clock::WatchmanClock;
            let watchman_clock = clock.watchman_clock.unwrap();
            let clock = match watchman_clock {
                WatchmanClock::StringClock(string_clock) => {
                    InnerClock::Spec(ClockSpec::StringClock(string_clock))
                }
                WatchmanClock::UnixTimestamp(unix_timestamp) => {
                    InnerClock::Spec(ClockSpec::UnixTimestamp(unix_timestamp))
                }
            };
            Self(clock)
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
            let client = connect().await.map_err(Error::WatchmanConnectError)?;
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
