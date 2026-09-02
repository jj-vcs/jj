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
    use std::path::Path;
    use std::path::PathBuf;

    use itertools::Itertools as _;
    use serde::Deserialize;
    use serde::Serialize;
    use thiserror::Error;
    use tracing::info;
    use tracing::instrument;
    use tracing::warn;
    use watchman_client::expr;
    use watchman_client::prelude::Clock as InnerClock;
    use watchman_client::prelude::ClockSpec;
    use watchman_client::prelude::NameOnly;
    use watchman_client::prelude::QueryFieldList as _;
    use watchman_client::prelude::QueryRequest;
    use watchman_client::prelude::QueryRequestCommon;
    use watchman_client::prelude::QueryResult;
    use watchman_client::prelude::TriggerCommand;
    use watchman_client::prelude::TriggerDelCommand;
    use watchman_client::prelude::TriggerDelResponse;
    use watchman_client::prelude::TriggerListCommand;
    use watchman_client::prelude::TriggerListResponse;
    use watchman_client::prelude::TriggerRequest;
    use watchman_client::prelude::TriggerResponse;

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

        #[error("Watchman failed to create a watch of the working copy root path")]
        CreateWatchError(#[source] watchman_client::Error),

        #[error("Failed to query Watchman")]
        WatchmanQueryError(#[source] watchman_client::Error),

        #[error("Failed to register Watchman trigger")]
        WatchmanTriggerError(#[source] watchman_client::Error),
    }

    /// Handle to the underlying Watchman instance.
    pub struct Fsmonitor {
        client: watchman_client::Client,
        /// Root directory of the Watchman watch that queries are issued
        /// against.
        watch_root: PathBuf,
        /// Path of the working copy root relative to `watch_root`, or `None`
        /// if the working copy root is the watch root itself.
        relative_path: Option<PathBuf>,
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
                .map_err(Error::CanonicalizeRootError)?
                .into_path_buf();
            let (watch_root, relative_path) =
                resolve_watch_root(&client, working_copy_root).await?;

            let monitor = Self {
                client,
                watch_root,
                relative_path,
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
                .generic_request(QueryRequest(
                    "query",
                    self.watch_root.clone(),
                    QueryRequestCommon {
                        since: previous_clock.map(|Clock(clock)| clock),
                        expression: Some(self.build_exclude_expr()),
                        relative_root: self.relative_path.clone(),
                        fields: NameOnly::field_list(),
                        ..Default::default()
                    },
                ))
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
            let response: TriggerListResponse = self
                .client
                .generic_request(TriggerListCommand("trigger-list", self.watch_root.clone()))
                .await
                .map_err(Error::WatchmanTriggerError)?;
            Ok(response
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
                .generic_request::<_, TriggerResponse>(TriggerCommand(
                    "trigger",
                    self.watch_root.clone(),
                    TriggerRequest {
                        name: "jj-background-monitor".to_string(),
                        // Evaluate the trigger (and run the command) relative
                        // to the working copy, not the enclosing watch root.
                        relative_root: self.relative_path.clone(),
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
                ))
                .await
                .map_err(Error::WatchmanTriggerError)?;
            Ok(())
        }

        /// Register trigger for changed files.
        #[instrument(skip(self))]
        async fn unregister_trigger(&self) -> Result<(), Error> {
            info!("Unregistering Watchman trigger...");
            self.client
                .generic_request::<_, TriggerDelResponse>(TriggerDelCommand(
                    "trigger-del",
                    self.watch_root.clone(),
                    "jj-background-monitor".to_string(),
                ))
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

    /// The Watchman `watch` command request, which creates a watch rooted
    /// exactly at the given directory. `watchman_client` only implements
    /// `watch-project`, which prefers reusing a watch of an enclosing
    /// directory.
    #[derive(Debug, Serialize)]
    struct WatchRequest(&'static str, PathBuf);

    /// The Watchman `watch` command response.
    #[derive(Debug, Deserialize)]
    #[expect(dead_code)]
    struct WatchResponse {
        version: String,
        watch: PathBuf,
    }

    /// Choose the Watchman watch to issue queries against for the working
    /// copy at `working_copy_root`, creating one if needed. Returns the watch
    /// root and the path of the working copy root relative to it (`None` if
    /// the working copy root is the watch root itself).
    async fn resolve_watch_root(
        client: &watchman_client::Client,
        working_copy_root: PathBuf,
    ) -> Result<(PathBuf, Option<PathBuf>), Error> {
        // Fast path: if a watch rooted exactly at the working copy root
        // already exists (e.g. created by an earlier run of this function),
        // use it directly. `resolve_root` (`watch-project`) prefers a watch
        // of an enclosing directory over an exact match, and probing whether
        // the enclosing watch can actually see the working copy costs a query
        // against the (potentially huge) enclosing root on every snapshot.
        let watch_exists = client
            .watch_list()
            .await
            .map_err(Error::WatchmanQueryError)?
            .roots
            .contains(&working_copy_root);
        if watch_exists {
            return Ok((working_copy_root, None));
        }

        let resolved_root = client
            .resolve_root(watchman_client::CanonicalPath::with_canonicalized_path(
                working_copy_root,
            ))
            .await
            .map_err(Error::ResolveRootError)?;
        let Some(relative_path) = resolved_root.project_relative_path() else {
            return Ok((resolved_root.path(), None));
        };

        // `resolve_root` reuses a watch of an enclosing directory if one
        // exists. Such a watch may be unable to see the working copy at all
        // (e.g. because the working copy is inside a directory listed in the
        // enclosing root's `ignore_dirs` Watchman configuration). Watchman
        // answers queries about an invisible subtree with an empty file list
        // and a valid clock, so every snapshot would silently report no
        // changes. Verify that the watch can see the working copy, and create
        // a dedicated watch otherwise.
        let watch_root = resolved_root.project_root();
        if is_path_visible(client, watch_root, relative_path).await? {
            return Ok((watch_root.to_owned(), Some(relative_path.to_owned())));
        }

        let working_copy_root = resolved_root.path();
        warn!(
            watch = %watch_root.display(),
            working_copy = %working_copy_root.display(),
            "Existing Watchman watch of an enclosing directory cannot see the \
             working copy (is the working copy inside one of the watch's \
             `ignore_dirs`?); creating a dedicated watch of the working copy \
             root (undo with `watchman watch-del <working copy root>`)"
        );
        client
            .generic_request::<_, WatchResponse>(WatchRequest("watch", working_copy_root.clone()))
            .await
            .map_err(Error::CreateWatchError)?;
        Ok((working_copy_root, None))
    }

    /// Whether the Watchman watch at `watch_root` can see the directory at
    /// `relative_path` inside it. A directory inside one of the watch's
    /// `ignore_dirs` is invisible to the watch: queries about it succeed but
    /// report no files, which must not be mistaken for a clean working copy.
    ///
    /// This probe is an optimization, not a safety requirement: callers treat
    /// "invisible" by creating a dedicated watch of the working copy root,
    /// which is always correct. Every failure mode here (a path the glob
    /// pattern cannot express, an unexpected generator behavior) yields
    /// `false` and thus merely costs a redundant watch; `true` requires
    /// Watchman to report the exact directory entry, which a blind watch
    /// cannot do.
    async fn is_path_visible(
        client: &watchman_client::Client,
        watch_root: &Path,
        relative_path: &Path,
    ) -> Result<bool, Error> {
        let result: QueryResult<NameOnly> = client
            .generic_request(QueryRequest(
                "query",
                watch_root.to_owned(),
                QueryRequestCommon {
                    // The glob generator resolves the path directly instead of
                    // filtering every file known to the watch, which matters
                    // when the watch root is a large tree.
                    glob: Some(vec![relative_path.to_string_lossy().into_owned()]),
                    // The motivating layout nests working copies under a
                    // dot-directory; don't let dotfile-globbing rules hide it.
                    glob_includedotfiles: true,
                    fields: NameOnly::field_list(),
                    ..Default::default()
                },
            ))
            .await
            .map_err(Error::WatchmanQueryError)?;
        // The glob pattern is the path as a literal string, so a path
        // containing glob metacharacters (or one that isn't valid UTF-8) may
        // match other entries. Only an exact match proves visibility;
        // treating anything else as invisible merely errs towards creating a
        // dedicated watch.
        Ok(result
            .files
            .unwrap_or_default()
            .iter()
            .any(|NameOnly { name }| name.as_path() == relative_path))
    }
}
