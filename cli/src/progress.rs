// Copyright 2022-2026 The Jujutsu Authors
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

use std::io;
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

pub struct ProgressWriter<'a> {
    prefix: &'a str,
    guard: Option<OutputGuard>,
    output: ProgressOutput<io::Stderr>,
    next_display_time: Instant,
}

// Callers call `display` every time they have new progress, and this will
// intermittently print the message synchronously, discarding too-frequent
// updates. There is an initial delay to avoid printing anything on fast
// commands. For example, `jj status` may be fast or slow depending on
// repository size, and in slow runs the progress updates help users see that jj
// is working.
//
// This deliberately doesn't try to be too smart about when to display progress,
// such as trying to display the latest message. Printing synchronously without
// any delay to a terminal would be a bottleneck; asynchronous output would both
// be complex due to threading, and also need to address callers that have their
// own output (for example, commit signing may subprocess and prompt for
// passphrases).
//
// When something is slow, this means the displayed message may be from prior to
// a slow step. Progress messages are an approximate indicator.
impl<'a> ProgressWriter<'a> {
    pub fn new(ui: &Ui, prefix: &'a str) -> Option<Self> {
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

    pub fn display(&mut self, text: &str) -> io::Result<()> {
        let now = Instant::now();
        if now < self.next_display_time {
            return Ok(());
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
        )?;
        self.output.flush()
    }
}

pub fn snapshot_progress(ui: &Ui) -> Option<impl Fn(&RepoPath) + use<>> {
    let writer = Mutex::new(ProgressWriter::new(ui, "Snapshotting")?);

    Some(move |path: &RepoPath| {
        // When the lock is held, skip updates to reduce contention.
        //
        // Executing the "future work" above may change this locking. Check
        // performance with output going to a tty so that writes are slower and
        // any lock contention is more visible.
        if let Ok(mut progress) = writer.try_lock() {
            progress
                .display(path.to_fs_path_unchecked(Path::new("")).to_str().unwrap())
                .ok();
        }
    })
}
