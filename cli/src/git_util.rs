// Copyright 2024 The Jujutsu Authors
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

//! Git utilities shared by various commands.

use std::collections::HashSet;
use std::error;
use std::fs;
use std::io;
use std::io::Write as _;
use std::iter;
use std::mem;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use std::time::Instant;

use bstr::ByteSlice as _;
use crossterm::terminal::Clear;
use crossterm::terminal::ClearType;
use indoc::writedoc;
use itertools::Itertools as _;
use jj_lib::file_util;
use jj_lib::git;
use jj_lib::git::FailedRefExportReason;
use jj_lib::git::GitExportStats;
use jj_lib::git::GitImportOptions;
use jj_lib::git::GitImportRefUpdate;
use jj_lib::git::GitImportStats;
use jj_lib::git::GitProgress;
use jj_lib::git::GitPushStats;
use jj_lib::git::GitRefKind;
use jj_lib::git::GitSettings;
use jj_lib::git::GitSidebandLineTerminator;
use jj_lib::git::GitSubprocessCallback;
use jj_lib::op_store::RemoteRefState;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo;
use jj_lib::settings::RemoteSettingsMap;
use jj_lib::workspace::Workspace;
use jj_lib::workspace_store::SimpleWorkspaceStore;
use jj_lib::workspace_store::WorkspaceStore as _;
use unicode_width::UnicodeWidthStr as _;

use crate::cleanup_guard::CleanupGuard;
use crate::cli_util::WorkspaceCommandTransaction;
use crate::cli_util::print_updated_commits;
use crate::command_error::CommandError;
use crate::command_error::cli_error;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::formatter::Formatter;
use crate::formatter::FormatterExt as _;
use crate::revset_util::parse_remote_auto_track_bookmarks_map;
use crate::ui::ProgressOutput;
use crate::ui::Ui;

pub fn is_colocated_git_workspace(workspace: &Workspace) -> bool {
    let Ok(git_backend) = git::get_git_backend(workspace.repo_loader().store()) else {
        return false;
    };
    let Some(git_workdir) = git_backend.git_workdir() else {
        return false; // Bare repository
    };
    if git_workdir == workspace.workspace_root() {
        return true;
    }
    if workspace.workspace_root().join(".git").is_file() {
        let Ok(worktree_repo) = gix::open(workspace.workspace_root()) else {
            return false;
        };
        let Ok(worktree_common_dir) = dunce::canonicalize(worktree_repo.common_dir()) else {
            return false;
        };
        let Ok(backend_common_dir) = dunce::canonicalize(git_backend.git_repo().common_dir())
        else {
            return false;
        };
        return worktree_repo.git_dir() != worktree_repo.common_dir()
            && worktree_common_dir == backend_common_dir;
    }
    // Colocated workspace should have ".git" directory, file, or symlink. Compare
    // its parent as the git_workdir might be resolved from the real ".git" path.
    let Ok(dot_git_path) = dunce::canonicalize(workspace.workspace_root().join(".git")) else {
        return false;
    };
    dunce::canonicalize(git_workdir).ok().as_deref() == dot_git_path.parent()
}

/// Parses user-specified remote URL or path to absolute form.
pub fn absolute_git_url(cwd: &Path, source: &str) -> Result<String, CommandError> {
    // Git appears to turn URL-like source to absolute path if local git directory
    // exits, and fails because '$PWD/https' is unsupported protocol. Since it would
    // be tedious to copy the exact git (or libgit2) behavior, we simply let gix
    // parse the input as URL, rcp-like, or local path.
    let mut url = gix::url::parse(source).map_err(cli_error)?;
    url.canonicalize(cwd).map_err(user_error)?;
    // As of gix 0.68.0, the canonicalized path uses platform-native directory
    // separator, which isn't compatible with libgit2 on Windows.
    if url.scheme == gix::url::Scheme::File {
        url.path = gix::path::to_unix_separators_on_windows(mem::take(&mut url.path)).into_owned();
    }
    // It's less likely that cwd isn't utf-8, so just fall back to original source.
    Ok(String::from_utf8(url.to_bstring().into()).unwrap_or_else(|_| source.to_owned()))
}

/// Converts a git remote URL to a normalized HTTPS URL for web browsing.
///
/// Returns `None` if the URL cannot be converted.
fn git_remote_url_to_web(url: &gix::Url) -> Option<String> {
    if url.scheme == gix::url::Scheme::File || url.host().is_none() {
        return None;
    }

    let host = url.host()?;
    let path = url.path.to_str().ok()?;
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);

    Some(format!("https://{host}/{path}"))
}

/// Returns the web URL for a git remote.
///
/// Attempts to convert the remote's URL to an HTTPS web URL.
/// Returns `None` if the remote doesn't exist or its URL cannot be converted.
pub fn get_remote_web_url(repo: &ReadonlyRepo, remote_name: &str) -> Option<String> {
    let git_repo = git::get_git_repo(repo.store()).ok()?;
    let remote = git_repo.try_find_remote(remote_name)?.ok()?;
    let url = remote
        .url(gix::remote::Direction::Fetch)
        .or_else(|| remote.url(gix::remote::Direction::Push))?;
    git_remote_url_to_web(url)
}

/// [`Ui`] adapter to forward Git command outputs.
pub struct GitSubprocessUi<'a> {
    // Don't hold locked ui.status() which could block tracing output in
    // different threads.
    ui: &'a Ui,
    progress_output: Option<ProgressOutput<io::Stderr>>,
    progress: Progress,
    // Sequence to erase line towards end.
    erase_end: &'static [u8],
}

impl<'a> GitSubprocessUi<'a> {
    pub fn new(ui: &'a Ui) -> Self {
        let progress_output = ui.progress_output();
        let is_terminal = progress_output.is_some();
        Self {
            ui,
            progress_output,
            progress: Progress::new(Instant::now()),
            erase_end: if is_terminal { b"\x1B[K" } else { b"        " },
        }
    }

    fn write_sideband(
        &self,
        prefix: &[u8],
        message: &[u8],
        term: Option<GitSidebandLineTerminator>,
    ) -> io::Result<()> {
        // TODO: maybe progress should be temporarily cleared if there are
        // sideband lines to write.
        let mut scratch =
            Vec::with_capacity(prefix.len() + message.len() + self.erase_end.len() + 1);
        scratch.extend_from_slice(prefix);
        scratch.extend_from_slice(message);
        // Do not erase the current line by new empty line: For progress
        // reporting, we may receive a bunch of percentage updates followed by
        // '\r' to remain on the same line, and at the end receive a single '\n'
        // to move to the next line. We should preserve the final status report
        // line by not appending erase_end sequence to this single line break.
        if !message.is_empty() {
            scratch.extend_from_slice(self.erase_end);
        }
        // It's unlikely, but don't leave message without newline.
        scratch.push(term.map_or(b'\n', |t| t.as_byte()));
        self.ui.status().write_all(&scratch)
    }
}

impl GitSubprocessCallback for GitSubprocessUi<'_> {
    fn needs_progress(&self) -> bool {
        self.progress_output.is_some()
    }

    fn progress(&mut self, progress: &GitProgress) -> io::Result<()> {
        if let Some(output) = &mut self.progress_output {
            self.progress.update(Instant::now(), progress, output)
        } else {
            Ok(())
        }
    }

    fn local_sideband(
        &mut self,
        message: &[u8],
        term: Option<GitSidebandLineTerminator>,
    ) -> io::Result<()> {
        self.write_sideband(b"git: ", message, term)
    }

    fn remote_sideband(
        &mut self,
        message: &[u8],
        term: Option<GitSidebandLineTerminator>,
    ) -> io::Result<()> {
        self.write_sideband(b"remote: ", message, term)
    }
}

pub fn load_git_import_options(
    ui: &Ui,
    git_settings: &GitSettings,
    remote_settings: &RemoteSettingsMap,
) -> Result<GitImportOptions, CommandError> {
    Ok(GitImportOptions {
        abandon_unreachable_commits: git_settings.abandon_unreachable_commits,
        record_synthetic_predecessors: git_settings.record_synthetic_predecessors,
        remote_auto_track_bookmarks: parse_remote_auto_track_bookmarks_map(ui, remote_settings)?,
    })
}

pub fn print_git_import_stats(
    ui: &Ui,
    tx: &WorkspaceCommandTransaction<'_>,
    stats: &GitImportStats,
) -> Result<(), CommandError> {
    if let Some(mut formatter) = ui.status_formatter() {
        print_imported_changes(formatter.as_mut(), tx, stats)?;
    }
    print_failed_git_import(ui, stats)?;
    Ok(())
}

fn print_imported_changes(
    formatter: &mut dyn Formatter,
    tx: &WorkspaceCommandTransaction<'_>,
    stats: &GitImportStats,
) -> Result<(), CommandError> {
    for (kind, changes) in [
        (GitRefKind::Bookmark, &stats.changed_remote_bookmarks),
        (GitRefKind::Tag, &stats.changed_remote_tags),
    ] {
        let refs_stats = changes
            .iter()
            .map(|update| RefStatus::new(kind, update, tx.repo()))
            .collect_vec();
        let Some(max_width) = refs_stats.iter().map(|x| x.symbol.width()).max() else {
            continue;
        };
        for status in refs_stats {
            status.output(max_width, formatter)?;
        }
    }

    if !stats.abandoned_commits.is_empty() {
        writeln!(
            formatter,
            "Abandoned {} commits that are no longer reachable:",
            stats.abandoned_commits.len()
        )?;
        let template = tx.commit_summary_template();
        print_updated_commits(formatter, &template, &stats.abandoned_commits)?;
    }
    if !stats.rewritten_commit_ids.is_empty() {
        writeln!(
            formatter,
            "Updated {} rewritten commits.",
            stats.rewritten_commit_ids.len()
        )?;
    }

    Ok(())
}

fn print_failed_git_import(ui: &Ui, stats: &GitImportStats) -> Result<(), CommandError> {
    if !stats.failed_ref_names.is_empty() {
        writeln!(ui.warning_default(), "Failed to import some Git refs:")?;
        let mut formatter = ui.stderr_formatter();
        for name in &stats.failed_ref_names {
            write!(formatter, "  ")?;
            write!(formatter.labeled("git_ref"), "{name}")?;
            writeln!(formatter)?;
        }
    }
    if stats
        .failed_ref_names
        .iter()
        .any(|name| name.starts_with(git::RESERVED_REMOTE_REF_NAMESPACE.as_bytes()))
    {
        writedoc!(
            ui.hint_default(),
            "
            Git remote named '{name}' is reserved for local Git repository.
            Use `jj git remote rename` to give a different name.
            ",
            name = git::REMOTE_NAME_FOR_LOCAL_GIT_REPO.as_symbol(),
        )?;
    }
    Ok(())
}

/// Prints only the summary of git import stats (abandoned count, failed refs).
/// Use this when a WorkspaceCommandTransaction is not available.
pub fn print_git_import_stats_summary(ui: &Ui, stats: &GitImportStats) -> Result<(), CommandError> {
    if let Some(mut formatter) = ui.status_formatter() {
        if !stats.abandoned_commits.is_empty() {
            writeln!(
                formatter,
                "Abandoned {} commits that are no longer reachable.",
                stats.abandoned_commits.len()
            )?;
        }
        if !stats.rewritten_commit_ids.is_empty() {
            writeln!(
                formatter,
                "Updated {} rewritten commits.",
                stats.rewritten_commit_ids.len()
            )?;
        }
    }
    print_failed_git_import(ui, stats)?;
    Ok(())
}

pub struct Progress {
    next_print: Instant,
    buffer: String,
    guard: Option<CleanupGuard>,
}

impl Progress {
    pub fn new(now: Instant) -> Self {
        Self {
            next_print: now + crate::progress::INITIAL_DELAY,
            buffer: String::new(),
            guard: None,
        }
    }

    pub fn update<W: std::io::Write>(
        &mut self,
        now: Instant,
        progress: &GitProgress,
        output: &mut ProgressOutput<W>,
    ) -> io::Result<()> {
        use std::fmt::Write as _;

        if progress.overall() == 1.0 {
            write!(output, "\r{}", Clear(ClearType::CurrentLine))?;
            output.flush()?;
            return Ok(());
        }

        if now < self.next_print {
            return Ok(());
        }
        self.next_print = now + Duration::from_secs(1) / crate::progress::UPDATE_HZ;
        if self.guard.is_none() {
            let guard = output.output_guard(crossterm::cursor::Show.to_string());
            let guard = CleanupGuard::new(move || {
                drop(guard);
            });
            write!(output, "{}", crossterm::cursor::Hide).ok();
            self.guard = Some(guard);
        }

        self.buffer.clear();
        // Overwrite the current local or sideband progress line if any.
        self.buffer.push('\r');
        let control_chars = self.buffer.len();
        write!(self.buffer, "{: >3.0}% ", 100.0 * progress.overall()).unwrap();

        let bar_width = output
            .term_width()
            .map(usize::from)
            .unwrap_or(0)
            .saturating_sub(self.buffer.len() - control_chars + 2);
        self.buffer.push('[');
        draw_progress(progress.overall(), &mut self.buffer, bar_width);
        self.buffer.push(']');

        write!(self.buffer, "{}", Clear(ClearType::UntilNewLine)).unwrap();
        // Move cursor back to the first column so the next sideband message
        // will overwrite the current progress.
        self.buffer.push('\r');
        write!(output, "{}", self.buffer)?;
        output.flush()?;
        Ok(())
    }
}

fn draw_progress(progress: f32, buffer: &mut String, width: usize) {
    const CHARS: [char; 9] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
    const RESOLUTION: usize = CHARS.len() - 1;
    let ticks = (width as f32 * progress.clamp(0.0, 1.0) * RESOLUTION as f32).round() as usize;
    let whole = ticks / RESOLUTION;
    for _ in 0..whole {
        buffer.push(CHARS[CHARS.len() - 1]);
    }
    if whole < width {
        let fraction = ticks % RESOLUTION;
        buffer.push(CHARS[fraction]);
    }
    for _ in (whole + 1)..width {
        buffer.push(CHARS[0]);
    }
}

struct RefStatus {
    ref_kind: GitRefKind,
    symbol: String,
    remote_ref_state: RemoteRefState,
    import_status: ImportStatus,
}

impl RefStatus {
    fn new(ref_kind: GitRefKind, update: &GitImportRefUpdate, repo: &dyn Repo) -> Self {
        let new_remote_ref = match ref_kind {
            GitRefKind::Bookmark => repo.view().get_remote_bookmark(update.symbol.as_ref()),
            GitRefKind::Tag => repo.view().get_remote_tag(update.symbol.as_ref()),
        };

        let import_status = match (
            update.old_remote_ref.target.is_absent(),
            update.new_target.is_absent(),
        ) {
            (true, false) => ImportStatus::New,
            (false, true) => ImportStatus::Deleted,
            _ => ImportStatus::Updated,
        };

        Self {
            symbol: update.symbol.to_string(),
            remote_ref_state: new_remote_ref.state,
            import_status,
            ref_kind,
        }
    }

    fn output(&self, max_symbol_width: usize, out: &mut dyn Formatter) -> std::io::Result<()> {
        let tracking_status = match self.remote_ref_state {
            RemoteRefState::New => "untracked",
            RemoteRefState::Tracked => "tracked",
        };

        let import_status = match self.import_status {
            ImportStatus::New => "new",
            ImportStatus::Deleted => "deleted",
            ImportStatus::Updated => "updated",
        };

        let symbol_width = self.symbol.width();
        let pad_width = max_symbol_width.saturating_sub(symbol_width);
        let padded_symbol = format!("{}{:>pad_width$}", self.symbol, "", pad_width = pad_width);

        let label = match self.ref_kind {
            GitRefKind::Bookmark => "bookmark",
            GitRefKind::Tag => "tag",
        };

        write!(out, "{label}: ")?;
        write!(out.labeled(label), "{padded_symbol}")?;
        writeln!(out, " [{import_status}] {tracking_status}")
    }
}

enum ImportStatus {
    New,
    Deleted,
    Updated,
}

pub fn print_git_export_stats(ui: &Ui, stats: &GitExportStats) -> Result<(), std::io::Error> {
    if !stats.failed_bookmarks.is_empty() {
        writeln!(ui.warning_default(), "Failed to export some bookmarks:")?;
        let mut formatter = ui.stderr_formatter();
        for (symbol, reason) in &stats.failed_bookmarks {
            write!(formatter, "  ")?;
            write!(formatter.labeled("bookmark"), "{symbol}")?;
            for err in iter::successors(Some(reason as &dyn error::Error), |err| err.source()) {
                write!(formatter, ": {err}")?;
            }
            writeln!(formatter)?;
        }
    }
    if !stats.failed_tags.is_empty() {
        writeln!(ui.warning_default(), "Failed to export some tags:")?;
        let mut formatter = ui.stderr_formatter();
        for (symbol, reason) in &stats.failed_tags {
            write!(formatter, "  ")?;
            write!(formatter.labeled("tag"), "{symbol}")?;
            for err in iter::successors(Some(reason as &dyn error::Error), |err| err.source()) {
                write!(formatter, ": {err}")?;
            }
            writeln!(formatter)?;
        }
    }
    if itertools::chain(&stats.failed_bookmarks, &stats.failed_tags)
        .any(|(_, reason)| matches!(reason, FailedRefExportReason::FailedToSet(_)))
    {
        writedoc!(
            ui.hint_default(),
            r#"
            Git doesn't allow a branch/tag name that looks like a parent directory of
            another (e.g. `foo` and `foo/bar`). Try to rename the bookmarks/tags that failed
            to export or their "parent" bookmarks/tags.
            "#,
        )?;
    }
    Ok(())
}

pub fn print_push_stats(ui: &Ui, stats: &GitPushStats) -> io::Result<()> {
    if !stats.rejected.is_empty() {
        writeln!(
            ui.warning_default(),
            "The following references unexpectedly moved on the remote:"
        )?;
        let mut formatter = ui.stderr_formatter();
        for (reference, reason) in &stats.rejected {
            write!(formatter, "  ")?;
            write!(formatter.labeled("git_ref"), "{}", reference.as_symbol())?;
            if let Some(r) = reason {
                write!(formatter, " (reason: {r})")?;
            }
            writeln!(formatter)?;
        }
        drop(formatter);
        writeln!(
            ui.hint_default(),
            "Try fetching from the remote, then make the bookmark point to where you want it to \
             be, and push again.",
        )?;
    }
    if !stats.remote_rejected.is_empty() {
        writeln!(
            ui.warning_default(),
            "The remote rejected the following updates:"
        )?;
        let mut formatter = ui.stderr_formatter();
        for (reference, reason) in &stats.remote_rejected {
            write!(formatter, "  ")?;
            write!(formatter.labeled("git_ref"), "{}", reference.as_symbol())?;
            if let Some(r) = reason {
                write!(formatter, " (reason: {r})")?;
            }
            writeln!(formatter)?;
        }
        drop(formatter);
        writeln!(
            ui.hint_default(),
            "Try checking if you have permission to push to all the bookmarks."
        )?;
    }
    if !stats.unexported_bookmarks.is_empty() {
        writeln!(
            ui.warning_default(),
            "The following bookmarks couldn't be updated locally:"
        )?;
        let mut formatter = ui.stderr_formatter();
        for (symbol, reason) in &stats.unexported_bookmarks {
            write!(formatter, "  ")?;
            write!(formatter.labeled("bookmark"), "{symbol}")?;
            for err in iter::successors(Some(reason as &dyn error::Error), |err| err.source()) {
                write!(formatter, ": {err}")?;
            }
            writeln!(formatter)?;
        }
    }
    Ok(())
}

pub fn create_git_worktree(
    ui: &Ui,
    git_settings: &GitSettings,
    main_workspace_root: &Path,
    destination: &Path,
) -> Result<bool, CommandError> {
    let output = Command::new(&git_settings.executable_path)
        .args(["worktree", "add", "--detach", "--no-checkout"])
        .arg(destination)
        .arg("HEAD")
        .current_dir(main_workspace_root)
        .output()
        .map_err(|err| {
            user_error(format!(
                "Failed to run `git worktree add`: {err}. Is `git` installed?"
            ))
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(user_error(format!(
            "Failed to create Git worktree: {stderr}"
        )));
    }
    writeln!(ui.status(), "Created Git worktree for the new workspace.")?;
    Ok(true)
}

pub fn remove_git_worktree(
    ui: &Ui,
    git_settings: &GitSettings,
    main_workspace_root: &Path,
    worktree_path: &Path,
) -> Result<(), CommandError> {
    let dot_git = worktree_path.join(".git");
    if !dot_git.is_file() {
        return Ok(());
    }
    // --force is needed because jj manages the working copy independently
    // from git, so git may consider the worktree dirty.
    let output = Command::new(&git_settings.executable_path)
        .args(["worktree", "remove", "--force"])
        .arg(worktree_path)
        .current_dir(main_workspace_root)
        .output();
    match output {
        Ok(o) if o.status.success() => {
            writeln!(
                ui.status(),
                r#"Removed Git worktree at "{}"."#,
                worktree_path.display()
            )?;
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            writeln!(
                ui.warning_default(),
                r#"Failed to remove Git worktree at "{}": {stderr}"#,
                worktree_path.display()
            )?;
        }
        Err(err) => {
            writeln!(
                ui.warning_default(),
                "Failed to run `git worktree remove`: {err}"
            )?;
        }
    }
    Ok(())
}

pub struct GitWorktreePaths {
    pub worktree_root: PathBuf,
    pub common_git_dir: PathBuf,
    pub workspace_name: jj_lib::ref_name::WorkspaceNameBuf,
}

pub fn discover_git_worktree_paths(
    git_settings: &GitSettings,
    cwd: &Path,
) -> Result<Option<GitWorktreePaths>, CommandError> {
    let git_executable = &git_settings.executable_path;
    let Some(worktree_root) = git_rev_parse_path(git_executable, cwd, "--show-toplevel")? else {
        return Ok(None);
    };
    let Some(git_dir) = git_rev_parse_path(git_executable, cwd, "--git-dir")? else {
        return Ok(None);
    };
    let Some(common_git_dir) = git_rev_parse_path(git_executable, cwd, "--git-common-dir")? else {
        return Ok(None);
    };
    if git_dir == common_git_dir {
        return Ok(None);
    }
    let Some(workspace_name) = git_dir.file_name().and_then(|name| name.to_str()) else {
        return Ok(None);
    };
    if workspace_name.is_empty() {
        return Ok(None);
    }
    Ok(Some(GitWorktreePaths {
        worktree_root,
        common_git_dir,
        workspace_name: workspace_name.into(),
    }))
}

pub(crate) fn git_rev_parse_path(
    git_executable: &Path,
    cwd: &Path,
    arg: &str,
) -> Result<Option<PathBuf>, CommandError> {
    let Ok(output) = Command::new(git_executable)
        .arg("rev-parse")
        .arg(arg)
        .current_dir(cwd)
        .output()
    else {
        return Ok(None);
    };
    if !output.status.success() {
        return Ok(None);
    }
    let Ok(path) = String::from_utf8(output.stdout) else {
        return Ok(None);
    };
    let path = Path::new(path.trim());
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        cwd.join(path)
    };
    match dunce::canonicalize(&path) {
        Ok(path) => Ok(Some(path)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(user_error_with_message(
            format!("Failed to resolve git path '{}'", path.display()),
            err,
        )),
    }
}

pub(crate) struct GitWorktreeRepoContext {
    pub(crate) workspace_store: SimpleWorkspaceStore,
    pub(crate) main_workspace_root: PathBuf,
    pub(crate) git_executable: PathBuf,
}

pub(crate) fn git_head_resolves(git_executable: &Path, cwd: &Path) -> Result<bool, CommandError> {
    let output = std::process::Command::new(git_executable)
        .args(["rev-parse", "--verify", "--quiet", "HEAD"])
        .current_dir(cwd)
        .output()
        .map_err(|err| user_error(format!("Failed to run `git rev-parse`: {err}")))?;
    Ok(output.status.success())
}

pub(crate) fn workspace_abs_path(
    repo_path: &Path,
    workspace_store: &SimpleWorkspaceStore,
    workspace_name: &WorkspaceName,
) -> Result<Option<PathBuf>, CommandError> {
    let Some(path) = workspace_store.get_workspace_path(workspace_name)? else {
        return Ok(None);
    };
    let path = if path.is_absolute() {
        path
    } else {
        repo_path.join(path)
    };
    match dunce::canonicalize(&path) {
        Ok(path) => Ok(Some(path)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Some(path)),
        Err(err) => Err(user_error_with_message(
            format!("Failed to resolve workspace path '{}'", path.display()),
            err,
        )),
    }
}

pub(crate) fn repair_jj_repo_link(
    workspace_root: &Path,
    repo_path: &Path,
) -> Result<(), CommandError> {
    let jj_dir = workspace_root.join(".jj");
    let repo_file_path = jj_dir.join("repo");
    if !repo_file_path.is_file() {
        return Ok(());
    }
    let jj_dir_abs = dunce::canonicalize(&jj_dir).map_err(|err| {
        user_error_with_message(format!("Failed to resolve '{}'", jj_dir.display()), err)
    })?;
    let repo_dir = dunce::canonicalize(repo_path).map_err(|err| {
        user_error_with_message(format!("Failed to resolve '{}'", repo_path.display()), err)
    })?;
    let path_to_store = file_util::relative_path(&jj_dir_abs, &repo_dir);
    let path_to_store = if path_to_store.is_relative() {
        file_util::slash_path(&path_to_store).into_owned()
    } else {
        path_to_store
    };
    let repo_dir_bytes = file_util::path_to_bytes(&path_to_store)
        .map_err(|err| user_error_with_message("Failed to encode jj repo path", err))?;
    if fs::read(&repo_file_path).ok().as_deref() == Some(repo_dir_bytes) {
        return Ok(());
    }
    fs::write(&repo_file_path, repo_dir_bytes)
        .map_err(|err| user_error_with_message("Failed to repair jj workspace link", err))?;
    Ok(())
}

pub(crate) fn git_worktree_paths(
    git_executable: &Path,
    main_workspace_root: &Path,
) -> Result<HashSet<PathBuf>, CommandError> {
    let output = std::process::Command::new(git_executable)
        .args(["worktree", "list", "--porcelain", "-z"])
        .current_dir(main_workspace_root)
        .output()
        .map_err(|err| user_error(format!("Failed to run `git worktree list`: {err}")))?;
    if !output.status.success() {
        return Err(user_error(format!(
            "Failed to list Git worktrees: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let mut paths = HashSet::new();
    for field in output.stdout.split_str(b"\0") {
        let Some(path) = field.strip_prefix(b"worktree ") else {
            continue;
        };
        let Ok(path) = path.to_str() else {
            continue;
        };
        if let Ok(path) = dunce::canonicalize(path) {
            paths.insert(path);
        }
    }
    Ok(paths)
}

pub(crate) fn create_git_worktree_in_existing_workspace(
    git_executable: &Path,
    main_workspace_root: &Path,
    workspace_root: &Path,
) -> Result<(), CommandError> {
    let parent = workspace_root.parent().unwrap_or(workspace_root);
    let temp_dir = tempfile::Builder::new()
        .prefix(".jj-git-worktree-")
        .tempdir_in(parent)
        .map_err(|err| user_error_with_message("Failed to create temporary Git worktree", err))?;
    let output = std::process::Command::new(git_executable)
        .args(["worktree", "add", "--detach", "--no-checkout"])
        .arg(temp_dir.path())
        .arg("HEAD")
        .current_dir(main_workspace_root)
        .output()
        .map_err(|err| user_error(format!("Failed to run `git worktree add`: {err}")))?;
    if !output.status.success() {
        return Err(user_error(format!(
            "Failed to create Git worktree: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let temp_dot_git = temp_dir.path().join(".git");
    let git_file = fs::read_to_string(&temp_dot_git)
        .map_err(|err| user_error_with_message("Failed to read temporary Git worktree", err))?;
    let Some(git_dir) = git_file.trim().strip_prefix("gitdir: ") else {
        return Err(user_error(
            "Temporary Git worktree has unexpected .git file",
        ));
    };
    let git_dir = Path::new(git_dir);
    let git_dir = if git_dir.is_absolute() {
        git_dir.to_owned()
    } else {
        temp_dir.path().join(git_dir)
    };
    let dot_git = workspace_root.join(".git");
    fs::rename(&temp_dot_git, &dot_git)
        .map_err(|err| user_error_with_message("Failed to install Git worktree file", err))?;
    fs::write(git_dir.join("gitdir"), dot_git.to_string_lossy().as_bytes())
        .map_err(|err| user_error_with_message("Failed to update Git worktree metadata", err))?;
    temp_dir
        .close()
        .map_err(|err| user_error_with_message("Failed to remove temporary Git worktree", err))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::MAIN_SEPARATOR;

    use insta::assert_snapshot;

    use super::*;

    #[test]
    fn test_absolute_git_url() {
        // gix::Url::canonicalize() works even if the path doesn't exist.
        // However, we need to ensure that no symlinks exist at the test paths.
        let temp_dir = testutils::new_temp_dir();
        let cwd = dunce::canonicalize(temp_dir.path()).unwrap();
        let cwd_slash = cwd.to_str().unwrap().replace(MAIN_SEPARATOR, "/");

        // Local path
        assert_eq!(
            absolute_git_url(&cwd, "foo").unwrap(),
            format!("{cwd_slash}/foo")
        );
        assert_eq!(
            absolute_git_url(&cwd, r"foo\bar").unwrap(),
            if cfg!(windows) {
                format!("{cwd_slash}/foo/bar")
            } else {
                format!(r"{cwd_slash}/foo\bar")
            }
        );
        assert_eq!(
            absolute_git_url(&cwd.join("bar"), &format!("{cwd_slash}/foo")).unwrap(),
            format!("{cwd_slash}/foo")
        );

        // rcp-like
        assert_eq!(
            absolute_git_url(&cwd, "git@example.org:foo/bar.git").unwrap(),
            "git@example.org:foo/bar.git"
        );
        // URL
        assert_eq!(
            absolute_git_url(&cwd, "https://example.org/foo.git").unwrap(),
            "https://example.org/foo.git"
        );
        // Custom scheme isn't an error
        assert_eq!(
            absolute_git_url(&cwd, "custom://example.org/foo.git").unwrap(),
            "custom://example.org/foo.git"
        );
        // Password shouldn't be redacted (gix::Url::to_string() would do)
        assert_eq!(
            absolute_git_url(&cwd, "https://user:pass@example.org/").unwrap(),
            "https://user:pass@example.org/"
        );

        // %-encoded paths: %20 ' ', %25 '%'
        assert_eq!(
            absolute_git_url(&cwd, "https://example.org/%20%25").unwrap(),
            "https://example.org/%20%25"
        );
        // No exact match because "/" isn't an absolute path on Windows
        assert!(
            absolute_git_url(&cwd, "file:///%20%25")
                .unwrap()
                .ends_with("/%20%25")
        );
    }

    #[test]
    fn test_git_remote_url_to_web() {
        let to_web = |s| git_remote_url_to_web(&gix::Url::try_from(s).unwrap());

        // SSH URL
        assert_eq!(
            to_web("git@github.com:owner/repo"),
            Some("https://github.com/owner/repo".to_owned())
        );
        // HTTPS URL with .git suffix
        assert_eq!(
            to_web("https://github.com/owner/repo.git"),
            Some("https://github.com/owner/repo".to_owned())
        );
        // SSH URL with ssh:// scheme
        assert_eq!(
            to_web("ssh://git@github.com/owner/repo"),
            Some("https://github.com/owner/repo".to_owned())
        );
        // git:// protocol
        assert_eq!(
            to_web("git://github.com/owner/repo.git"),
            Some("https://github.com/owner/repo".to_owned())
        );
        // File URL returns None
        assert_eq!(to_web("file:///path/to/repo"), None);
        // Local path returns None
        assert_eq!(to_web("/path/to/repo"), None);
    }

    #[test]
    fn test_bar() {
        let mut buf = String::new();
        draw_progress(0.0, &mut buf, 10);
        assert_eq!(buf, "          ");
        buf.clear();
        draw_progress(1.0, &mut buf, 10);
        assert_eq!(buf, "██████████");
        buf.clear();
        draw_progress(0.5, &mut buf, 10);
        assert_eq!(buf, "█████     ");
        buf.clear();
        draw_progress(0.54, &mut buf, 10);
        assert_eq!(buf, "█████▍    ");
        buf.clear();
    }

    #[test]
    fn test_update() {
        let start = Instant::now();
        let mut progress = Progress::new(start);
        let mut current_time = start;
        let mut update = |duration, overall: u64| -> String {
            current_time += duration;
            let mut buf = vec![];
            let mut output = ProgressOutput::for_test(&mut buf, 25);
            progress
                .update(
                    current_time,
                    &GitProgress {
                        deltas: (overall, 100),
                        objects: (0, 0),
                        counted_objects: (0, 0),
                        compressed_objects: (0, 0),
                    },
                    &mut output,
                )
                .unwrap();
            String::from_utf8(buf).unwrap()
        };
        // First output is after the initial delay
        assert_snapshot!(update(crate::progress::INITIAL_DELAY - Duration::from_millis(1), 1), @"");
        assert_snapshot!(update(Duration::from_millis(1), 10), @"\u{1b}[?25l\r 10% [█▊                ]\u{1b}[K");
        // No updates for the next 30 milliseconds
        assert_snapshot!(update(Duration::from_millis(10), 11), @"");
        assert_snapshot!(update(Duration::from_millis(10), 12), @"");
        assert_snapshot!(update(Duration::from_millis(10), 13), @"");
        // We get an update now that we go over the threshold
        assert_snapshot!(update(Duration::from_millis(100), 30), @"\r 30% [█████▍            ]\u{1b}[K");
        // Even though we went over by quite a bit, the new threshold is relative to the
        // previous output, so we don't get an update here
        assert_snapshot!(update(Duration::from_millis(30), 40), @"");
    }
}
