use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use crossterm::terminal::Clear;
use crossterm::terminal::ClearType;
use jj_lib::repo_path::RepoPath;

use crate::text_util;
use crate::ui::OutputGuard;
use crate::ui::ProgressOutput;
use crate::ui::Ui;

pub const UPDATE_HZ: u32 = 30;
pub const INITIAL_DELAY: Duration = Duration::from_millis(250);

pub fn snapshot_progress(ui: &Ui) -> Option<impl Fn(&RepoPath) + use<>> {
    struct State {
        guard: Option<OutputGuard>,
        output: ProgressOutput<std::io::Stderr>,
        next_display_time: Instant,
    }

    let output = ui.progress_output()?;

    // Don't clutter the output during fast operations.
    let next_display_time = Instant::now() + INITIAL_DELAY;
    let state = Mutex::new(State {
        guard: None,
        output,
        next_display_time,
    });

    Some(move |path: &RepoPath| {
        let mut state = state.lock().unwrap();
        let now = Instant::now();
        if now < state.next_display_time {
            // Future work: Display current path after exactly, say, 250ms has elapsed, to
            // better handle large single files
            return;
        }
        state.next_display_time = now + Duration::from_secs(1) / UPDATE_HZ;

        if state.guard.is_none() {
            state.guard = Some(
                state
                    .output
                    .output_guard(format!("\r{}", Clear(ClearType::CurrentLine))),
            );
        }

        let line_width = state.output.term_width().map(usize::from).unwrap_or(80);
        let max_path_width = line_width.saturating_sub(13); // Account for "Snapshotting "
        let fs_path = path.to_fs_path_unchecked(Path::new(""));
        let (display_path, _) =
            text_util::elide_start(fs_path.to_str().unwrap(), "...", max_path_width);

        write!(
            state.output,
            "\r{}Snapshotting {display_path}",
            Clear(ClearType::CurrentLine),
        )
        .ok();
        state.output.flush().ok();
    })
}
