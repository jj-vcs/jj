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

//! User-provided executables that jj runs at defined points in a command's
//! lifecycle. See `docs/design/hooks.md`.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Stdio;

use thiserror::Error;

use crate::config::ConfigGetError;
use crate::config::ConfigGetResultExt as _;
use crate::content_hash::blake2b_hash;
use crate::hex_util::encode_hex;
use crate::settings::UserSettings;
use crate::subprocess_util::suppress_console_window;

/// An error while resolving or running a hook.
#[derive(Debug, Error)]
pub enum HookError {
    /// The hook executable could not be spawned, or writing its stdin
    /// payload or waiting for it to exit failed.
    #[error("failed to run hook {path}")]
    Spawn {
        /// The hook executable that could not be run.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The hook ran and exited with a nonzero status.
    #[error("hook {path} exited with {exit_status}")]
    Failed {
        /// The hook executable that failed.
        path: PathBuf,
        /// The exit status it returned.
        exit_status: ExitStatus,
    },
}

/// Name of the directory holding repository-scoped hooks, relative to the
/// workspace root. A plain directory next to `.jj` rather than inside it, so
/// it is an ordinary tracked path and gets versioned like any other file in
/// the repo.
const REPO_HOOKS_DIR: &str = ".jj-hooks";

/// Name of the directory holding global hooks, relative to the jj
/// configuration directory (e.g. `root_config_dir()` in `cli/src/config.rs`).
const GLOBAL_HOOKS_DIR: &str = "hooks";

/// The kind of hook being invoked.
///
/// Deliberately small so that adding a kind later is additive.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum HookKind {
    /// Runs once per `jj git push`, before anything is sent to any remote.
    GitPrePush,
    /// Runs once per `jj git push`, after every matched remote has been
    /// pushed to successfully.
    GitPostPush,
}

impl HookKind {
    /// Every hook kind, for callers (e.g. `jj hook status`) that need to
    /// enumerate them.
    pub const ALL: [Self; 2] = [Self::GitPrePush, Self::GitPostPush];

    /// The file name a hook implementing this kind must have on disk, both
    /// under the global hooks directory and under a repository's
    /// `.jj-hooks/`.
    pub fn file_name(self) -> &'static str {
        match self {
            Self::GitPrePush => "git-pre-push",
            Self::GitPostPush => "git-post-push",
        }
    }
}

/// The path a global hook of this kind would live at, given the jj
/// configuration directory (`$config_dir/jj`).
pub fn global_hook_path(root_config_dir: &Path, kind: HookKind) -> PathBuf {
    root_config_dir
        .join(GLOBAL_HOOKS_DIR)
        .join(kind.file_name())
}

/// The path a repository-scoped hook of this kind would live at, given the
/// workspace root.
pub fn repo_hook_path(workspace_root: &Path, kind: HookKind) -> PathBuf {
    workspace_root.join(REPO_HOOKS_DIR).join(kind.file_name())
}

/// Trust state of a repository-scoped hook file, as determined by the
/// caller's trust storage. See `docs/design/hooks.md`, "Trust model".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepoHookTrust {
    /// Repository-scoped hooks are not enabled for this repository at all.
    NotEnabled,
    /// Enabled, and this file's content matches what was last approved by
    /// `jj hook enable`.
    Trusted,
    /// Enabled, but this file's content no longer matches what was
    /// approved (a local edit, or content that arrived via `jj git fetch`,
    /// a rebase, or a checkout of a different bookmark).
    Stale,
}

/// The result of resolving which hook, if any, should run for a kind.
#[derive(Debug, Eq, PartialEq)]
pub struct ResolvedHook {
    /// The executable to run, or `None` if nothing applies.
    pub path: Option<PathBuf>,
    /// Set when a repository-scoped hook file exists but was skipped
    /// because its content is [`RepoHookTrust::Stale`]. The caller should
    /// warn and point at `jj hook enable` to re-approve; this is not set
    /// (and no warning is warranted) when the repository-scoped hook is
    /// simply not enabled, since that is the ordinary default state.
    pub stale_repo_hook: bool,
}

/// Resolves which hook file, if any, should run for `kind`.
///
/// A repository-scoped hook takes precedence over the global hook of the
/// same name if it exists and `repo_trust` reports [`RepoHookTrust::Trusted`]
/// for it. This is a full override, not a merge: a repository that wants the
/// global hook's behavior needs to invoke it explicitly. Otherwise, the
/// global hook runs if present. If neither applies, nothing runs.
pub fn resolve_hook(
    root_config_dir: &Path,
    workspace_root: &Path,
    kind: HookKind,
    repo_trust: impl FnOnce(&Path) -> RepoHookTrust,
) -> ResolvedHook {
    let repo_path = repo_hook_path(workspace_root, kind);
    let global = global_hook_path(root_config_dir, kind);
    let global = global.is_file().then_some(global);

    if !repo_path.is_file() {
        return ResolvedHook {
            path: global,
            stale_repo_hook: false,
        };
    }
    match repo_trust(&repo_path) {
        RepoHookTrust::Trusted => ResolvedHook {
            path: Some(repo_path),
            stale_repo_hook: false,
        },
        RepoHookTrust::NotEnabled => ResolvedHook {
            path: global,
            stale_repo_hook: false,
        },
        RepoHookTrust::Stale => ResolvedHook {
            path: global,
            stale_repo_hook: true,
        },
    }
}

/// A hex-encoded content hash of a file, suitable for detecting whether a
/// repository-scoped hook's content has changed since it was last approved
/// by `jj hook enable`. Uses the same BLAKE2b hash the rest of jj uses for
/// content addressing, rather than pulling in a separate hash function.
pub fn hash_hook_file(path: &Path) -> std::io::Result<String> {
    let content = std::fs::read(path)?;
    Ok(encode_hex(&blake2b_hash(content.as_slice())))
}

/// Dotted config key holding the repository-scoped hook enablement flag.
const ENABLED_KEY: &str = "hooks.enabled";
/// Dotted config key holding the file-name-to-content-hash table of
/// repository-scoped hooks approved by `jj hook enable`.
const TRUSTED_HASHES_KEY: &str = "hooks.trusted-hashes";

/// The repository-scoped hook trust state: whether repo-scoped hooks are
/// enabled, and which content hash was last approved for each hook file
/// name.
///
/// These `hooks.*` keys are intended to live in the repository-scoped
/// secure config (`SecureConfig`, keyed by the repository's config-id and
/// bound to its path), written only by `jj hook enable`/`jj hook disable`.
/// Reading and writing goes through the existing config machinery rather
/// than a dedicated storage layer: reads via [`UserSettings`], writes via
/// `jj_lib::config::ConfigFile` at the CLI layer.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HookTrustState {
    /// Whether the user has run `jj hook enable` for this repository.
    pub enabled: bool,
    /// File name (as returned by `HookKind::file_name()`, or any other file
    /// present under `.jj-hooks/`) to the hex-encoded content hash recorded
    /// the last time it was approved.
    pub trusted_hashes: BTreeMap<String, String>,
}

impl HookTrustState {
    /// Reads the current trust state from `hooks.enabled` and
    /// `hooks.trusted-hashes`.
    pub fn from_settings(settings: &UserSettings) -> Result<Self, ConfigGetError> {
        let enabled = settings.get_bool(ENABLED_KEY).optional()?.unwrap_or(false);
        let trusted_hashes = settings
            .get::<BTreeMap<String, String>>(TRUSTED_HASHES_KEY)
            .optional()?
            .unwrap_or_default();
        Ok(Self {
            enabled,
            trusted_hashes,
        })
    }

    /// Determines the [`RepoHookTrust`] of a specific repository-scoped hook
    /// file, for use with [`resolve_hook`].
    ///
    /// A file with no recorded hash is untrusted by construction (it either
    /// didn't exist at enable-time, or its name was never approved), the
    /// same as a file whose content no longer matches what was recorded.
    pub fn check(&self, path: &Path) -> RepoHookTrust {
        if !self.enabled {
            return RepoHookTrust::NotEnabled;
        }
        let file_name = path.file_name().and_then(|name| name.to_str());
        let expected = file_name.and_then(|name| self.trusted_hashes.get(name));
        match (expected, hash_hook_file(path)) {
            (Some(expected), Ok(actual)) if *expected == actual => RepoHookTrust::Trusted,
            _ => RepoHookTrust::Stale,
        }
    }
}

/// Lists the `.jj-hooks/*` files currently present in the workspace, paired
/// with their content hash.
///
/// This is what `jj hook enable` records as newly-trusted: it is deliberate
/// that this lists every file under the directory, not just the ones
/// matching a known [`HookKind`], since a future hook kind (or a file a
/// user keeps there for their own purposes) should not need re-approval
/// just because jj didn't recognize its name yet.
pub fn current_repo_hook_hashes(
    workspace_root: &Path,
) -> std::io::Result<BTreeMap<String, String>> {
    let dir = workspace_root.join(REPO_HOOKS_DIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(err) => return Err(err),
    };
    let mut hashes = BTreeMap::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        hashes.insert(name, hash_hook_file(&entry.path())?);
    }
    Ok(hashes)
}

/// A single hook invocation, ready to run: which executable, in what
/// directory, with what environment, and what to write to its stdin.
#[derive(Debug)]
pub struct HookInvocation {
    kind: HookKind,
    executable: PathBuf,
    workspace_root: PathBuf,
    stdin: Vec<u8>,
}

impl HookInvocation {
    /// Creates a hook invocation. `workspace_root` is used both as the
    /// working directory and as the `JJ_REPO_ROOT` environment variable.
    /// `stdin` is the JSON payload described in the invocation contract.
    pub fn new(
        kind: HookKind,
        executable: PathBuf,
        workspace_root: PathBuf,
        stdin: Vec<u8>,
    ) -> Self {
        Self {
            kind,
            executable,
            workspace_root,
            stdin,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        suppress_console_window(&mut command);
        command
            .current_dir(&self.workspace_root)
            .env("JJ_HOOK", self.kind.file_name())
            .env("JJ_REPO_ROOT", &self.workspace_root)
            .stdin(Stdio::piped());
        command
    }

    /// Runs the hook, writing the stdin payload and waiting for it to exit.
    /// Stdout/stderr are inherited, so the hook can talk directly to the
    /// user's terminal the same way git's own output is streamed during
    /// `jj git push`.
    pub fn run(&self) -> Result<(), HookError> {
        let spawn_error = |source| HookError::Spawn {
            path: self.executable.clone(),
            source,
        };
        let mut child = self.command().spawn().map_err(spawn_error)?;
        let write_result = child.stdin.take().unwrap().write_all(&self.stdin);
        let status = child.wait().map_err(spawn_error)?;
        match write_result {
            Ok(()) => {}
            // A hook that exits (and thus closes stdin) before reading all of
            // it is the hook's decision to make, not a jj-side error.
            Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {}
            Err(err) => return Err(spawn_error(err)),
        }
        if status.success() {
            Ok(())
        } else {
            Err(HookError::Failed {
                path: self.executable.clone(),
                exit_status: status,
            })
        }
    }
}

/// What happened when [`run_hook`] was asked to run a hook.
#[derive(Debug, Eq, PartialEq)]
pub struct HookOutcome {
    /// Whether a hook was actually invoked.
    pub ran: bool,
    /// Set when a repository-scoped hook file exists but was skipped
    /// because it is no longer trusted; the caller should warn and point at
    /// `jj hook enable`. See [`ResolvedHook::stale_repo_hook`].
    pub stale_repo_hook: bool,
}

/// Resolves and runs the appropriate hook for `kind`, honoring `no_hooks`.
///
/// When `no_hooks` is set, this returns immediately, as if no hook file
/// existed: it does not resolve paths, does not call `repo_trust`, and does
/// not touch the trust check. This is the single place `--no-hooks` is
/// enforced, so every future hook-bearing call site gets its behavior by
/// construction rather than needing to remember to check the flag itself.
pub fn run_hook(
    root_config_dir: &Path,
    workspace_root: &Path,
    kind: HookKind,
    stdin: Vec<u8>,
    no_hooks: bool,
    repo_trust: impl FnOnce(&Path) -> RepoHookTrust,
) -> Result<HookOutcome, HookError> {
    if no_hooks {
        return Ok(HookOutcome {
            ran: false,
            stale_repo_hook: false,
        });
    }
    let resolved = resolve_hook(root_config_dir, workspace_root, kind, repo_trust);
    let Some(path) = resolved.path else {
        return Ok(HookOutcome {
            ran: false,
            stale_repo_hook: resolved.stale_repo_hook,
        });
    };
    HookInvocation::new(kind, path, workspace_root.to_path_buf(), stdin).run()?;
    Ok(HookOutcome {
        ran: true,
        stale_repo_hook: resolved.stale_repo_hook,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigLayer;
    use crate::config::ConfigSource;
    use crate::config::StackedConfig;

    // Not using testutils::user_settings(): jj-lib depends on testutils only
    // as a dev-dependency, and testutils depends on jj-lib normally, so this
    // module's `UserSettings` and testutils' `UserSettings` are two distinct
    // types from two separate compilations of the crate. See the identical
    // comment on `git_backend::tests::user_settings()`.
    fn settings_with_hooks_config(hooks_toml: &str) -> UserSettings {
        let mut config = StackedConfig::with_defaults();
        config.add_layer(ConfigLayer::parse(ConfigSource::User, hooks_toml).unwrap());
        UserSettings::from_config(config).unwrap()
    }

    #[test]
    fn test_hook_trust_state_defaults_to_disabled() {
        let settings = settings_with_hooks_config("");
        assert_eq!(
            HookTrustState::from_settings(&settings).unwrap(),
            HookTrustState::default()
        );
    }

    #[test]
    fn test_hook_trust_state_reads_config() {
        let settings = settings_with_hooks_config(
            r#"
            [hooks]
            enabled = true
            [hooks.trusted-hashes]
            git-pre-push = "abc123"
            "#,
        );
        let state = HookTrustState::from_settings(&settings).unwrap();
        assert!(state.enabled);
        assert_eq!(state.trusted_hashes.get("git-pre-push").unwrap(), "abc123");
    }

    #[test]
    fn test_hook_trust_state_check_not_enabled() {
        let dir = crate::tests::new_temp_dir();
        let path = dir.path().join("git-pre-push");
        std::fs::write(&path, "content").unwrap();
        let state = HookTrustState::default();
        assert_eq!(state.check(&path), RepoHookTrust::NotEnabled);
    }

    #[test]
    fn test_hook_trust_state_check_trusted() {
        let dir = crate::tests::new_temp_dir();
        let path = dir.path().join("git-pre-push");
        std::fs::write(&path, "content").unwrap();
        let hash = hash_hook_file(&path).unwrap();
        let state = HookTrustState {
            enabled: true,
            trusted_hashes: BTreeMap::from([("git-pre-push".to_string(), hash)]),
        };
        assert_eq!(state.check(&path), RepoHookTrust::Trusted);
    }

    #[test]
    fn test_hook_trust_state_check_stale_on_content_change() {
        let dir = crate::tests::new_temp_dir();
        let path = dir.path().join("git-pre-push");
        std::fs::write(&path, "content").unwrap();
        let hash = hash_hook_file(&path).unwrap();
        std::fs::write(&path, "edited").unwrap();
        let state = HookTrustState {
            enabled: true,
            trusted_hashes: BTreeMap::from([("git-pre-push".to_string(), hash)]),
        };
        assert_eq!(state.check(&path), RepoHookTrust::Stale);
    }

    #[test]
    fn test_hook_trust_state_check_stale_when_never_approved() {
        let dir = crate::tests::new_temp_dir();
        let path = dir.path().join("git-post-push");
        std::fs::write(&path, "content").unwrap();
        let state = HookTrustState {
            enabled: true,
            trusted_hashes: BTreeMap::new(),
        };
        assert_eq!(state.check(&path), RepoHookTrust::Stale);
    }

    #[test]
    fn test_current_repo_hook_hashes_no_dir() {
        let dir = crate::tests::new_temp_dir();
        assert_eq!(
            current_repo_hook_hashes(dir.path()).unwrap(),
            BTreeMap::new()
        );
    }

    #[test]
    fn test_current_repo_hook_hashes_lists_files() {
        let dir = crate::tests::new_temp_dir();
        let hooks_dir = dir.path().join(REPO_HOOKS_DIR);
        std::fs::create_dir(&hooks_dir).unwrap();
        std::fs::write(hooks_dir.join("git-pre-push"), "a").unwrap();
        std::fs::write(hooks_dir.join("git-post-push"), "b").unwrap();
        std::fs::create_dir(hooks_dir.join("a-subdirectory")).unwrap();

        let hashes = current_repo_hook_hashes(dir.path()).unwrap();
        assert_eq!(
            hashes,
            BTreeMap::from([
                (
                    "git-pre-push".to_string(),
                    hash_hook_file(&hooks_dir.join("git-pre-push")).unwrap()
                ),
                (
                    "git-post-push".to_string(),
                    hash_hook_file(&hooks_dir.join("git-post-push")).unwrap()
                ),
            ])
        );
    }

    // Spawning a real executable cross-platform without a shell needs either a
    // compiled binary or a Windows batch file; a `#!/bin/sh` script is the
    // simplest thing that works on unix. Same precedent as
    // `secure_config::tests::test_repo_aliased`: unix-only, feature works
    // identically on Windows.
    #[cfg(unix)]
    fn write_test_hook(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;

        let path = dir.join("hook");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn test_hook_invocation_success() {
        let dir = crate::tests::new_temp_dir();
        let hook = write_test_hook(dir.path(), "cat > \"$JJ_REPO_ROOT/stdin\"; exit 0");
        let invocation = HookInvocation::new(
            HookKind::GitPrePush,
            hook,
            dir.path().to_path_buf(),
            b"payload".to_vec(),
        );
        invocation.run().unwrap();
        assert_eq!(std::fs::read(dir.path().join("stdin")).unwrap(), b"payload");
    }

    #[cfg(unix)]
    #[test]
    fn test_hook_invocation_failure() {
        let dir = crate::tests::new_temp_dir();
        let hook = write_test_hook(dir.path(), "exit 7");
        let invocation = HookInvocation::new(
            HookKind::GitPrePush,
            hook.clone(),
            dir.path().to_path_buf(),
            vec![],
        );
        let err = invocation.run().unwrap_err();
        assert_matches::assert_matches!(
            err,
            HookError::Failed { path, .. } if path == hook
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_hook_invocation_env_vars() {
        let dir = crate::tests::new_temp_dir();
        let hook = write_test_hook(
            dir.path(),
            "printf '%s\\n%s' \"$JJ_HOOK\" \"$JJ_REPO_ROOT\" > \"$JJ_REPO_ROOT/env\"",
        );
        let invocation = HookInvocation::new(
            HookKind::GitPostPush,
            hook,
            dir.path().to_path_buf(),
            vec![],
        );
        invocation.run().unwrap();
        let content = std::fs::read_to_string(dir.path().join("env")).unwrap();
        assert_eq!(content, format!("git-post-push\n{}", dir.path().display()));
    }

    #[test]
    fn test_hook_paths() {
        let config_dir = Path::new("/config/jj");
        let workspace_root = Path::new("/repo");
        assert_eq!(
            global_hook_path(config_dir, HookKind::GitPrePush),
            Path::new("/config/jj/hooks/git-pre-push"),
        );
        assert_eq!(
            repo_hook_path(workspace_root, HookKind::GitPostPush),
            Path::new("/repo/.jj-hooks/git-post-push"),
        );
    }

    fn setup_resolve_dirs() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = crate::tests::new_temp_dir();
        let config_dir = dir.path().join("config");
        let workspace_root = dir.path().join("repo");
        std::fs::create_dir_all(config_dir.join(GLOBAL_HOOKS_DIR)).unwrap();
        std::fs::create_dir_all(workspace_root.join(REPO_HOOKS_DIR)).unwrap();
        (dir, config_dir, workspace_root)
    }

    #[test]
    fn test_resolve_hook_nothing_configured() {
        let (_dir, config_dir, workspace_root) = setup_resolve_dirs();
        let resolved = resolve_hook(&config_dir, &workspace_root, HookKind::GitPrePush, |_| {
            panic!("repo hook file does not exist, trust should not be checked")
        });
        assert_eq!(
            resolved,
            ResolvedHook {
                path: None,
                stale_repo_hook: false,
            }
        );
    }

    #[test]
    fn test_resolve_hook_global_only() {
        let (_dir, config_dir, workspace_root) = setup_resolve_dirs();
        let global = global_hook_path(&config_dir, HookKind::GitPrePush);
        std::fs::write(&global, "").unwrap();
        let resolved = resolve_hook(&config_dir, &workspace_root, HookKind::GitPrePush, |_| {
            panic!("repo hook file does not exist, trust should not be checked")
        });
        assert_eq!(
            resolved,
            ResolvedHook {
                path: Some(global),
                stale_repo_hook: false,
            }
        );
    }

    #[test]
    fn test_resolve_hook_repo_not_enabled_falls_back_to_global() {
        let (_dir, config_dir, workspace_root) = setup_resolve_dirs();
        let global = global_hook_path(&config_dir, HookKind::GitPrePush);
        std::fs::write(&global, "").unwrap();
        std::fs::write(repo_hook_path(&workspace_root, HookKind::GitPrePush), "").unwrap();
        let resolved = resolve_hook(&config_dir, &workspace_root, HookKind::GitPrePush, |_| {
            RepoHookTrust::NotEnabled
        });
        assert_eq!(
            resolved,
            ResolvedHook {
                path: Some(global),
                stale_repo_hook: false,
            }
        );
    }

    #[test]
    fn test_resolve_hook_repo_trusted_overrides_global() {
        let (_dir, config_dir, workspace_root) = setup_resolve_dirs();
        std::fs::write(global_hook_path(&config_dir, HookKind::GitPrePush), "").unwrap();
        let repo = repo_hook_path(&workspace_root, HookKind::GitPrePush);
        std::fs::write(&repo, "").unwrap();
        let resolved = resolve_hook(&config_dir, &workspace_root, HookKind::GitPrePush, |_| {
            RepoHookTrust::Trusted
        });
        assert_eq!(
            resolved,
            ResolvedHook {
                path: Some(repo),
                stale_repo_hook: false,
            }
        );
    }

    #[test]
    fn test_resolve_hook_repo_stale_falls_back_and_warns() {
        let (_dir, config_dir, workspace_root) = setup_resolve_dirs();
        let global = global_hook_path(&config_dir, HookKind::GitPrePush);
        std::fs::write(&global, "").unwrap();
        std::fs::write(repo_hook_path(&workspace_root, HookKind::GitPrePush), "").unwrap();
        let resolved = resolve_hook(&config_dir, &workspace_root, HookKind::GitPrePush, |_| {
            RepoHookTrust::Stale
        });
        assert_eq!(
            resolved,
            ResolvedHook {
                path: Some(global),
                stale_repo_hook: true,
            }
        );
    }

    #[test]
    fn test_run_hook_no_hooks_short_circuits() {
        let (_dir, config_dir, workspace_root) = setup_resolve_dirs();
        std::fs::write(global_hook_path(&config_dir, HookKind::GitPrePush), "").unwrap();
        let outcome = run_hook(
            &config_dir,
            &workspace_root,
            HookKind::GitPrePush,
            vec![],
            true,
            |_| panic!("--no-hooks must not evaluate trust at all"),
        )
        .unwrap();
        assert_eq!(
            outcome,
            HookOutcome {
                ran: false,
                stale_repo_hook: false,
            }
        );
    }

    #[test]
    fn test_run_hook_nothing_configured_is_a_noop() {
        let (_dir, config_dir, workspace_root) = setup_resolve_dirs();
        let outcome = run_hook(
            &config_dir,
            &workspace_root,
            HookKind::GitPrePush,
            vec![],
            false,
            |_| panic!("repo hook file does not exist, trust should not be checked"),
        )
        .unwrap();
        assert_eq!(
            outcome,
            HookOutcome {
                ran: false,
                stale_repo_hook: false,
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_run_hook_runs_resolved_hook_and_propagates_failure() {
        let (_dir, config_dir, workspace_root) = setup_resolve_dirs();
        let global = global_hook_path(&config_dir, HookKind::GitPrePush);
        std::fs::remove_file(&global).ok();
        std::fs::write(&global, "#!/bin/sh\nexit 3\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&global, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let err = run_hook(
            &config_dir,
            &workspace_root,
            HookKind::GitPrePush,
            vec![],
            false,
            |_| panic!("repo hook file does not exist, trust should not be checked"),
        )
        .unwrap_err();
        assert_matches::assert_matches!(err, HookError::Failed { path, .. } if path == global);
    }

    #[test]
    fn test_hash_hook_file_detects_content_change() {
        let dir = crate::tests::new_temp_dir();
        let path = dir.path().join("hook");
        std::fs::write(&path, "original").unwrap();
        let original_hash = hash_hook_file(&path).unwrap();

        // Same content, re-read: same hash.
        assert_eq!(hash_hook_file(&path).unwrap(), original_hash);

        // Content changed: different hash. This is the mechanism that
        // detects a `.jj-hooks/*` file changing after `jj hook enable`,
        // whether from a local edit or content pulled in by a fetch.
        std::fs::write(&path, "edited").unwrap();
        assert_ne!(hash_hook_file(&path).unwrap(), original_hash);
    }
}
