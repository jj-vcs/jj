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
use std::io::Read as _;
use std::io::Write as _;
use std::iter;
use std::mem;
use std::path::Path;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use crossterm::terminal::Clear;
use crossterm::terminal::ClearType;
use indoc::writedoc;
use itertools::Itertools as _;
use jj_lib::fmt_util::binary_prefix;
use jj_lib::git;
use jj_lib::git::FailedRefExportReason;
use jj_lib::git::GitExportStats;
use jj_lib::git::GitImportStats;
use jj_lib::git::GitRefKind;
use jj_lib::op_store::RefTarget;
use jj_lib::op_store::RemoteRef;
use jj_lib::ref_name::RemoteRefSymbol;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo;
use jj_lib::workspace::Workspace;
use unicode_width::UnicodeWidthStr as _;

use crate::cleanup_guard::CleanupGuard;
use crate::command_error::cli_error;
use crate::command_error::user_error;
use crate::command_error::CommandError;
use crate::formatter::Formatter;
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

fn terminal_get_username(ui: &Ui, url: &str) -> Option<String> {
    ui.prompt(&format!("Username for {url}")).ok()
}

fn terminal_get_pw(ui: &Ui, url: &str) -> Option<String> {
    ui.prompt_password(&format!("Passphrase for {url}")).ok()
}

fn pinentry_get_pw(url: &str) -> Option<String> {
    // https://www.gnupg.org/documentation/manuals/assuan/Server-responses.html#Server-responses
    fn decode_assuan_data(encoded: &str) -> Option<String> {
        let encoded = encoded.as_bytes();
        let mut decoded = Vec::with_capacity(encoded.len());
        let mut i = 0;
        while i < encoded.len() {
            if encoded[i] != b'%' {
                decoded.push(encoded[i]);
                i += 1;
                continue;
            }
            i += 1;
            let byte =
                u8::from_str_radix(std::str::from_utf8(encoded.get(i..i + 2)?).ok()?, 16).ok()?;
            decoded.push(byte);
            i += 2;
        }
        String::from_utf8(decoded).ok()
    }

    let mut pinentry = std::process::Command::new("pinentry")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let mut interact = || -> std::io::Result<_> {
        #[rustfmt::skip]
        let req = format!(
            "SETTITLE jj passphrase\n\
             SETDESC Enter passphrase for {url}\n\
             SETPROMPT Passphrase:\n\
             GETPIN\n"
        );
        pinentry.stdin.take().unwrap().write_all(req.as_bytes())?;
        let mut out = String::new();
        pinentry.stdout.take().unwrap().read_to_string(&mut out)?;
        Ok(out)
    };
    let maybe_out = interact();
    _ = pinentry.wait();
    for line in maybe_out.ok()?.split('\n') {
        if !line.starts_with("D ") {
            continue;
        }
        let (_, encoded) = line.split_at(2);
        return decode_assuan_data(encoded);
    }
    None
}

#[tracing::instrument]
fn get_ssh_keys(_username: &str) -> Vec<PathBuf> {
    let mut paths = vec![];
    if let Ok(home_dir) = etcetera::home_dir() {
        let ssh_dir = home_dir.join(".ssh");
        for filename in ["id_ed25519_sk", "id_ed25519", "id_rsa"] {
            let key_path = ssh_dir.join(filename);
            if key_path.is_file() {
                tracing::info!(path = ?key_path, "found ssh key");
                paths.push(key_path);
            }
        }
    }
    if paths.is_empty() {
        tracing::info!("no ssh key found");
    }
    paths
}

// Based on Git's implementation: https://github.com/git/git/blob/43072b4ca132437f21975ac6acc6b72dc22fd398/sideband.c#L178
pub struct GitSidebandProgressMessageWriter {
    display_prefix: &'static [u8],
    suffix: &'static [u8],
    scratch: Vec<u8>,
}

impl GitSidebandProgressMessageWriter {
    pub fn new(ui: &Ui) -> Self {
        let is_terminal = ui.use_progress_indicator();

        GitSidebandProgressMessageWriter {
            display_prefix: "remote: ".as_bytes(),
            suffix: if is_terminal { "\x1B[K" } else { "        " }.as_bytes(),
            scratch: Vec::new(),
        }
    }

    pub fn write(&mut self, ui: &Ui, progress_message: &[u8]) -> std::io::Result<()> {
        let mut index = 0;
        // Append a suffix to each nonempty line to clear the end of the screen line.
        loop {
            let Some(i) = progress_message[index..]
                .iter()
                .position(|&c| c == b'\r' || c == b'\n')
                .map(|i| index + i)
            else {
                break;
            };
            let line_length = i - index;

            // For messages sent across the packet boundary, there would be a nonempty
            // "scratch" buffer from last call of this function, and there may be a leading
            // CR/LF in this message. For this case we should add a clear-to-eol suffix to
            // clean leftover letters we previously have written on the same line.
            if !self.scratch.is_empty() && line_length == 0 {
                self.scratch.extend_from_slice(self.suffix);
            }

            if self.scratch.is_empty() {
                self.scratch.extend_from_slice(self.display_prefix);
            }

            // Do not add the clear-to-eol suffix to empty lines:
            // For progress reporting we may receive a bunch of percentage updates
            // followed by '\r' to remain on the same line, and at the end receive a single
            // '\n' to move to the next line. We should preserve the final
            // status report line by not appending clear-to-eol suffix to this single line
            // break.
            if line_length > 0 {
                self.scratch.extend_from_slice(&progress_message[index..i]);
                self.scratch.extend_from_slice(self.suffix);
            }
            self.scratch.extend_from_slice(&progress_message[i..i + 1]);

            ui.status().write_all(&self.scratch)?;
            self.scratch.clear();

            index = i + 1;
        }

        // Add leftover message to "scratch" buffer to be printed in next call.
        if index < progress_message.len() {
            if self.scratch.is_empty() {
                self.scratch.extend_from_slice(self.display_prefix);
            }
            self.scratch.extend_from_slice(&progress_message[index..]);
        }

        Ok(())
    }

    pub fn flush(&mut self, ui: &Ui) -> std::io::Result<()> {
        if !self.scratch.is_empty() {
            self.scratch.push(b'\n');
            ui.status().write_all(&self.scratch)?;
            self.scratch.clear();
        }

        Ok(())
    }
}

pub fn with_remote_git_callbacks<T>(ui: &Ui, f: impl FnOnce(git::RemoteCallbacks<'_>) -> T) -> T {
    let mut callbacks = git::RemoteCallbacks::default();

    let mut progress_callback;
    if let Some(mut output) = ui.progress_output() {
        let mut progress = Progress::new(Instant::now());
        progress_callback = move |x: &git::Progress| {
            _ = progress.update(Instant::now(), x, &mut output);
        };
        callbacks.progress = Some(&mut progress_callback);
    }

    let mut sideband_progress_writer = GitSidebandProgressMessageWriter::new(ui);
    let mut sideband_progress_callback = |progress_message: &[u8]| {
        _ = sideband_progress_writer.write(ui, progress_message);
    };
    callbacks.sideband_progress = Some(&mut sideband_progress_callback);

    let mut get_ssh_keys = get_ssh_keys; // Coerce to unit fn type
    callbacks.get_ssh_keys = Some(&mut get_ssh_keys);
    let mut get_pw =
        |url: &str, _username: &str| pinentry_get_pw(url).or_else(|| terminal_get_pw(ui, url));
    callbacks.get_password = Some(&mut get_pw);
    let mut get_user_pw =
        |url: &str| Some((terminal_get_username(ui, url)?, terminal_get_pw(ui, url)?));
    callbacks.get_username_password = Some(&mut get_user_pw);

    let result = f(callbacks);
    _ = sideband_progress_writer.flush(ui);
    result
}

pub fn print_git_import_stats(
    ui: &Ui,
    repo: &dyn Repo,
    stats: &GitImportStats,
    show_ref_stats: bool,
) -> Result<(), CommandError> {
    let Some(mut formatter) = ui.status_formatter() else {
        return Ok(());
    };
    if show_ref_stats {
        let refs_stats = [
            (GitRefKind::Bookmark, &stats.changed_remote_bookmarks),
            (GitRefKind::Tag, &stats.changed_remote_tags),
        ]
        .into_iter()
        .flat_map(|(kind, changes)| {
            changes
                .iter()
                .map(move |(symbol, (remote_ref, ref_target))| {
                    RefStatus::new(kind, symbol.as_ref(), remote_ref, ref_target, repo)
                })
        })
        .collect_vec();

        let has_both_ref_kinds =
            !stats.changed_remote_bookmarks.is_empty() && !stats.changed_remote_tags.is_empty();
        let max_width = refs_stats.iter().map(|x| x.symbol.width()).max();
        if let Some(max_width) = max_width {
            for status in refs_stats {
                status.output(max_width, has_both_ref_kinds, &mut *formatter)?;
            }
        }
    }

    if !stats.abandoned_commits.is_empty() {
        writeln!(
            formatter,
            "Abandoned {} commits that are no longer reachable.",
            stats.abandoned_commits.len()
        )?;
    }

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

pub struct Progress {
    next_print: Instant,
    rate: RateEstimate,
    buffer: String,
    guard: Option<CleanupGuard>,
}

impl Progress {
    pub fn new(now: Instant) -> Self {
        Self {
            next_print: now + crate::progress::INITIAL_DELAY,
            rate: RateEstimate::new(),
            buffer: String::new(),
            guard: None,
        }
    }

    pub fn update<W: std::io::Write>(
        &mut self,
        now: Instant,
        progress: &git::Progress,
        output: &mut ProgressOutput<W>,
    ) -> io::Result<()> {
        use std::fmt::Write as _;

        if progress.overall == 1.0 {
            write!(output, "\r{}", Clear(ClearType::CurrentLine))?;
            output.flush()?;
            return Ok(());
        }

        let rate = progress
            .bytes_downloaded
            .and_then(|x| self.rate.update(now, x));
        if now < self.next_print {
            return Ok(());
        }
        self.next_print = now + Duration::from_secs(1) / crate::progress::UPDATE_HZ;
        if self.guard.is_none() {
            let guard = output.output_guard(crossterm::cursor::Show.to_string());
            let guard = CleanupGuard::new(move || {
                drop(guard);
            });
            _ = write!(output, "{}", crossterm::cursor::Hide);
            self.guard = Some(guard);
        }

        self.buffer.clear();
        // Overwrite the current local or sideband progress line if any.
        self.buffer.push('\r');
        let control_chars = self.buffer.len();
        write!(self.buffer, "{: >3.0}% ", 100.0 * progress.overall).unwrap();
        if let Some(total) = progress.bytes_downloaded {
            let (scaled, prefix) = binary_prefix(total as f32);
            write!(self.buffer, "{scaled: >5.1} {prefix}B ").unwrap();
        }
        if let Some(estimate) = rate {
            let (scaled, prefix) = binary_prefix(estimate);
            write!(self.buffer, "at {scaled: >5.1} {prefix}B/s ").unwrap();
        }

        let bar_width = output
            .term_width()
            .map(usize::from)
            .unwrap_or(0)
            .saturating_sub(self.buffer.len() - control_chars + 2);
        self.buffer.push('[');
        draw_progress(progress.overall, &mut self.buffer, bar_width);
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

struct RateEstimate {
    state: Option<RateEstimateState>,
}

impl RateEstimate {
    pub fn new() -> Self {
        RateEstimate { state: None }
    }

    /// Compute smoothed rate from an update
    pub fn update(&mut self, now: Instant, total: u64) -> Option<f32> {
        if let Some(ref mut state) = self.state {
            return Some(state.update(now, total));
        }

        self.state = Some(RateEstimateState {
            total,
            avg_rate: None,
            last_sample: now,
        });
        None
    }
}

struct RateEstimateState {
    total: u64,
    avg_rate: Option<f32>,
    last_sample: Instant,
}

impl RateEstimateState {
    fn update(&mut self, now: Instant, total: u64) -> f32 {
        let delta = total - self.total;
        self.total = total;
        let dt = now - self.last_sample;
        self.last_sample = now;
        let sample = delta as f32 / dt.as_secs_f32();
        match self.avg_rate {
            None => *self.avg_rate.insert(sample),
            Some(ref mut avg_rate) => {
                // From Algorithms for Unevenly Spaced Time Series: Moving
                // Averages and Other Rolling Operators (Andreas Eckner, 2019)
                const TIME_WINDOW: f32 = 2.0;
                let alpha = 1.0 - (-dt.as_secs_f32() / TIME_WINDOW).exp();
                *avg_rate += alpha * (sample - *avg_rate);
                *avg_rate
            }
        }
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

    fn output(
        &self,
        max_symbol_width: usize,
        has_both_ref_kinds: bool,
        out: &mut dyn Formatter,
    ) -> std::io::Result<()> {
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

        let ref_kind = match self.ref_kind {
            GitRefKind::Bookmark => "bookmark: ",
            GitRefKind::Tag if !has_both_ref_kinds => "tag: ",
            GitRefKind::Tag => "tag:    ",
        };

        write!(out, "{ref_kind}")?;
        write!(out.labeled("bookmark"), "{padded_symbol}")?;
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
        drop(formatter);
        if stats
            .failed_bookmarks
            .iter()
            .any(|(_, reason)| matches!(reason, FailedRefExportReason::FailedToSet(_)))
        {
            writeln!(
                ui.hint_default(),
                r#"Git doesn't allow a branch name that looks like a parent directory of
another (e.g. `foo` and `foo/bar`). Try to rename the bookmarks that failed to
export or their "parent" bookmarks."#,
            )?;
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
        let mut update = |duration, overall| -> String {
            current_time += duration;
            let mut buf = vec![];
            let mut output = ProgressOutput::for_test(&mut buf, 25);
            progress
                .update(
                    current_time,
                    &jj_lib::git::Progress {
                        bytes_downloaded: None,
                        overall,
                    },
                    &mut output,
                )
                .unwrap();
            String::from_utf8(buf).unwrap()
        };
        // First output is after the initial delay
        assert_snapshot!(update(crate::progress::INITIAL_DELAY - Duration::from_millis(1), 0.1), @"");
        assert_snapshot!(update(Duration::from_millis(1), 0.10), @"\u{1b}[?25l\r 10% [█▊                ]\u{1b}[K");
        // No updates for the next 30 milliseconds
        assert_snapshot!(update(Duration::from_millis(10), 0.11), @"");
        assert_snapshot!(update(Duration::from_millis(10), 0.12), @"");
        assert_snapshot!(update(Duration::from_millis(10), 0.13), @"");
        // We get an update now that we go over the threshold
        assert_snapshot!(update(Duration::from_millis(100), 0.30), @" 30% [█████▍            ][K");
        // Even though we went over by quite a bit, the new threshold is relative to the
        // previous output, so we don't get an update here
        assert_snapshot!(update(Duration::from_millis(30), 0.40), @"");
    }
}
