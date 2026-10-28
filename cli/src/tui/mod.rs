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

//! Shared infrastructure for interactive terminal user interfaces.

use std::io;
use std::io::Stdout;
use std::iter;

use crossterm::ExecutableCommand as _;
use crossterm::event::Event;
use crossterm::event::KeyEvent;
use crossterm::event::{self};
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use itertools::Itertools as _;
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::prelude::CrosstermBackend;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use unicode_width::UnicodeWidthChar as _;

use crate::command_error::CommandError;
use crate::command_error::internal_error;

pub(crate) mod annotate;

/// Terminal in raw mode on the alternate screen. The terminal is restored when
/// this is dropped, including when unwinding from a panic.
pub(crate) struct TuiTerminal {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    _restore_guard: RestoreGuard,
}

impl TuiTerminal {
    pub fn new() -> io::Result<Self> {
        io::stdout().execute(EnterAlternateScreen)?;
        let restore_guard = RestoreGuard;
        enable_raw_mode()?;
        let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        terminal.clear()?;
        Ok(Self {
            terminal,
            _restore_guard: restore_guard,
        })
    }

    pub fn draw(&mut self, render: impl FnOnce(&mut Frame)) -> Result<(), CommandError> {
        self.terminal
            .draw(render)
            .map_err(|e| internal_error(format!("Failed to draw TUI: {e}")))?;
        Ok(())
    }
}

struct RestoreGuard;

impl Drop for RestoreGuard {
    fn drop(&mut self) {
        disable_raw_mode().ok();
        io::stdout().execute(LeaveAlternateScreen).ok();
    }
}

/// Blocks until the next terminal event. Returns the key event if a key was
/// pressed. Returns `None` for other events, such as resizes, after which the
/// caller should redraw.
pub(crate) fn read_key_press() -> Result<Option<KeyEvent>, CommandError> {
    let event =
        event::read().map_err(|e| internal_error(format!("Failed to read TUI events: {e}")))?;
    // On Windows, we get Press and Release (and maybe Repeat) events, but on Linux
    // we only get Press.
    match event {
        Event::Key(event) if !event.is_release() => Ok(Some(event)),
        _ => Ok(None),
    }
}

/// Builds a line describing key bindings, such as "q quit • c confirm".
pub(crate) fn help_line(items: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, (key, desc)) in items.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" • "));
        }
        spans.push(Span::styled(
            key.to_string(),
            Style::default().fg(Color::Magenta),
        ));
        spans.push(Span::raw(format!(" {desc}")));
    }
    Line::from(spans)
}

/// Converts output from a `Formatter` (which may contain ANSI escape codes)
/// to styled text.
pub(crate) fn ansi_to_text(bytes: &[u8]) -> Text<'static> {
    ansi_to_tui::IntoText::into_text(&bytes).unwrap()
}

/// Replaces tabs in the `line` by spaces up to the next tab stop. Tab stops
/// are every 8 columns.
pub(crate) fn expand_tabs(line: Line<'static>) -> Line<'static> {
    const TAB_WIDTH: usize = 8;
    let mut column = 0;
    let spans = line
        .spans
        .into_iter()
        .map(|span| {
            if !span.content.contains('\t') {
                column += span.width();
                return span;
            }
            let mut content = String::new();
            for c in span.content.chars() {
                if c == '\t' {
                    let num_spaces = TAB_WIDTH - column % TAB_WIDTH;
                    content.extend(iter::repeat_n(' ', num_spaces));
                    column += num_spaces;
                } else {
                    content.push(c);
                    column += c.width().unwrap_or(0);
                }
            }
            Span::styled(content, span.style)
        })
        .collect_vec();
    let mut expanded = Line::from(spans).style(line.style);
    expanded.alignment = line.alignment;
    expanded
}

/// Returns the index of the first visible item such that `selected` is visible
/// in a viewport of `visible_count` items. Scrolls as little as possible from
/// `scroll_top`.
pub(crate) fn scroll_to_selection(
    scroll_top: usize,
    selected: usize,
    visible_count: usize,
    total_count: usize,
) -> usize {
    let visible_count = visible_count.max(1);
    let scroll_top = if selected < scroll_top {
        selected
    } else if selected >= scroll_top.saturating_add(visible_count) {
        selected.saturating_sub(visible_count - 1)
    } else {
        scroll_top
    };
    let max_scroll = total_count.saturating_sub(visible_count);
    scroll_top.min(max_scroll)
}

#[cfg(test)]
pub(crate) fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
    let mut res = String::new();
    for y in 0..buf.area.height {
        let mut line = String::new();
        for x in 0..buf.area.width {
            let cell = &buf[(x, y)];
            line.push_str(cell.symbol());
        }
        res.push_str(line.trim_end());
        res.push('\n');
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_tabs() {
        let expand = |spans: Vec<Span<'static>>| {
            expand_tabs(Line::from(spans))
                .spans
                .iter()
                .map(|span| span.content.to_string())
                .collect_vec()
        };
        assert_eq!(expand(vec![Span::raw("\tx")]), ["        x"]);
        assert_eq!(expand(vec![Span::raw("ab\tx\ty")]), ["ab      x       y"]);
        // Columns are counted across spans
        assert_eq!(
            expand(vec![Span::raw("abc"), Span::raw("\tx")]),
            ["abc", "     x"]
        );
        // Wide characters take two columns
        assert_eq!(expand(vec![Span::raw("日本\tx")]), ["日本    x"]);
    }

    #[test]
    fn test_scroll_to_selection() {
        // Selection already visible
        assert_eq!(scroll_to_selection(0, 3, 5, 10), 0);
        assert_eq!(scroll_to_selection(2, 6, 5, 10), 2);
        // Selection above the viewport
        assert_eq!(scroll_to_selection(4, 1, 5, 10), 1);
        // Selection below the viewport
        assert_eq!(scroll_to_selection(0, 7, 5, 10), 3);
        // Doesn't scroll past the end, e.g. after the viewport grew
        assert_eq!(scroll_to_selection(8, 9, 5, 10), 5);
        assert_eq!(scroll_to_selection(3, 2, 20, 10), 0);
        // Empty viewport is treated as one item
        assert_eq!(scroll_to_selection(0, 4, 0, 10), 4);
    }
}
