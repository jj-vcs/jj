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

struct Progress<'a> {
    prefix: &'a str,
    guard: Option<OutputGuard>,
    output: ProgressOutput<std::io::Stderr>,
    next_display_time: Instant,
}

// Future work: Make that the progress prints the current element we are
// currently working on, either:
// - upon change of the element we are working on and if the next display time
//   is passed (current behavior)
// - upon reaching the next display time for an element that would have not been
//   displayed yet. This
// would assure two things:
// - The first message will end-up being displayed if it takes more than the
//   initial delay to process the associated element. Assuring jj never goes
//   silent for more than the specified initial delay.
// - For the other elements, what we print is more factual regarding what we are
//   doing. Without printing too much.

impl<'a> Progress<'a> {
    fn new(ui: &Ui, prefix: &'a str) -> Option<Self> {
        let output = ui.progress_output()?;

        // Don't clutter the output during fast operations.
        let next_display_time = Instant::now() + INITIAL_DELAY;
        Some(Self {
            prefix,
            guard: None,
            output,
            next_display_time,
        })
    }

    fn display(&mut self, text: &str) {
        let now = Instant::now();
        if now < self.next_display_time {
            return;
        }

        self.next_display_time = now + Duration::from_secs(1) / UPDATE_HZ;

        if self.guard.is_none() {
            self.guard = Some(
                self.output
                    .output_guard(format!("\r{}", Clear(ClearType::CurrentLine))),
            );
        }

        let line_width = self.output.term_width().map(usize::from).unwrap_or(80);
        let max_path_width = self.prefix.len() + 1; // Take into account the empty space added after the prefix.
        let (display_text, _) =
            text_util::elide_start(text, "...", line_width.saturating_sub(max_path_width));

        write!(
            self.output,
            "\r{}{} {display_text}",
            Clear(ClearType::CurrentLine),
            self.prefix
        )
        .ok();
        self.output.flush().ok();
    }
}

pub fn snapshot_progress(ui: &Ui) -> Option<impl Fn(&RepoPath) + use<>> {
    let progress = Mutex::new(Progress::new(ui, "Snapshotting")?);

    Some(move |path: &RepoPath| {
        progress
            .lock()
            .unwrap()
            .display(path.to_fs_path_unchecked(Path::new("")).to_str().unwrap());
    })
}
