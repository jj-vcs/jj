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

use std::error;
use std::io;
use std::io::Write as _;
use std::iter;
use std::mem;
use std::num::NonZero;
use std::path::Path;
use std::time::Duration;
use std::time::Instant;

use bstr::ByteSlice as _;
use crossterm::terminal::Clear;
use crossterm::terminal::ClearType;
use indoc::writedoc;
use itertools::Itertools as _;
use jj_lib::commit::Commit;
use jj_lib::git;
use jj_lib::git::FailedRefExportReason;
use jj_lib::git::GitExportStats;
use jj_lib::git::GitImportOptions;
use jj_lib::git::GitImportStats;
use jj_lib::git::GitProgress;
use jj_lib::git::GitPushStats;
use jj_lib::git::GitRefKind;
use jj_lib::git::GitSettings;
use jj_lib::git::GitSidebandLineTerminator;
use jj_lib::git::GitSubprocessCallback;
use jj_lib::op_store::RefTarget;
use jj_lib::op_store::RemoteRef;
use jj_lib::ref_name::RemoteRefSymbol;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo;
use jj_lib::settings::RemoteSettingsMap;
use jj_lib::workspace::Workspace;
use unicode_width::UnicodeWidthStr as _;

use crate::cleanup_guard::CleanupGuard;
use crate::cli_util::WorkspaceCommandTransaction;
use crate::cli_util::print_updated_commits;
use crate::command_error::CommandError;
use crate::command_error::cli_error;
use crate::command_error::user_error;
use crate::formatter::Formatter;
use crate::formatter::FormatterExt as _;
use crate::revset_util::parse_remote_auto_track_bookmarks_map;
use crate::ui::ProgressOutput;
use crate::ui::Ui;

pub fn is_colocated_git_workspace(workspace: &Workspace, repo: &ReadonlyRepo) -> bool {
    let Ok(git_backend) = git::get_git_backend(repo.store()) else {
        return false;
    };
    let Some(git_workdir) = git_backend.git_workdir() else {
        return false; // Bare repository
    };
    if git_workdir == workspace.workspace_root() {
        return true;
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
    let mut url = gix::url::parse(source.as_ref()).map_err(cli_error)?;
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
        auto_local_bookmark: git_settings.auto_local_bookmark,
        abandon_unreachable_commits: git_settings.abandon_unreachable_commits,
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
            .map(|(symbol, (remote_ref, ref_target))| {
                RefStatus::new(kind, symbol.as_ref(), remote_ref, ref_target, tx.repo())
            })
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
        let abandoned_commits: Vec<Commit> = stats
            .abandoned_commits
            .iter()
            .map(|id| tx.repo().store().get_commit(id))
            .try_collect()?;
        let template = tx.commit_summary_template();
        print_updated_commits(formatter, &template, &abandoned_commits)?;
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
    if !stats.abandoned_commits.is_empty()
        && let Some(mut formatter) = ui.status_formatter()
    {
        writeln!(
            formatter,
            "Abandoned {} commits that are no longer reachable.",
            stats.abandoned_commits.len()
        )?;
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

        let line_width: usize = output.term_width().map_or(0, NonZero::get).into();
        let bar_width = line_width.saturating_sub(self.buffer.len() - control_chars + 2);
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
    tracking_status: TrackingStatus,
    import_status: ImportStatus,
}

impl RefStatus {
    fn new(
        ref_kind: GitRefKind,
        symbol: RemoteRefSymbol<'_>,
        remote_ref: &RemoteRef,
        ref_target: &RefTarget,
        repo: &dyn Repo,
    ) -> Self {
        let tracking_status = match ref_kind {
            GitRefKind::Bookmark => {
                if repo.view().get_remote_bookmark(symbol).is_tracked() {
                    TrackingStatus::Tracked
                } else {
                    TrackingStatus::Untracked
                }
            }
            GitRefKind::Tag => TrackingStatus::NotApplicable,
        };

        let import_status = match (remote_ref.target.is_absent(), ref_target.is_absent()) {
            (true, false) => ImportStatus::New,
            (false, true) => ImportStatus::Deleted,
            _ => ImportStatus::Updated,
        };

        Self {
            symbol: symbol.to_string(),
            tracking_status,
            import_status,
            ref_kind,
        }
    }

    fn output(&self, max_symbol_width: usize, out: &mut dyn Formatter) -> std::io::Result<()> {
        let tracking_status = match self.tracking_status {
            TrackingStatus::Tracked => "tracked",
            TrackingStatus::Untracked => "untracked",
            TrackingStatus::NotApplicable => "",
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

enum TrackingStatus {
    Tracked,
    Untracked,
    NotApplicable, // for tags
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
