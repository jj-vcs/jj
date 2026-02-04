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

use std::io;
use std::time::Duration;

use crossterm::ExecutableCommand as _;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use crossterm::event::{self};
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use ratatui::Terminal;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::prelude::CrosstermBackend;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::internal_error;
use crate::ui::Ui;

/// Interactively rearrange the commit graph.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct ArrangeArgs {}

#[instrument(skip_all)]
pub(crate) fn cmd_arrange(
    ui: &mut Ui,
    _command: &CommandHelper,
    _args: &ArrangeArgs,
) -> Result<(), CommandError> {
    // Set up the terminal
    io::stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;

    let result = run_tui(ui, &mut terminal);

    // Restore the terminal
    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

    result
}

fn run_tui<B: ratatui::backend::Backend>(
    _ui: &mut Ui,
    terminal: &mut Terminal<B>,
) -> Result<(), CommandError> {
    let help_items = [("q", "quit")];
    let mut help_spans = Vec::new();
    for (i, (key, desc)) in help_items.iter().enumerate() {
        if i > 0 {
            help_spans.push(Span::raw(" "));
        }
        help_spans.push(Span::styled(*key, Style::default().fg(Color::Magenta)));
        help_spans.push(Span::raw(format!(" {desc}")));
    }
    let help_line = Line::from(help_spans);

    loop {
        terminal
            .draw(|frame| {
                let layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Fill(1), Constraint::Length(1)])
                    .split(frame.area());
                let _main_area = layout[0];
                let help_area = layout[1];

                frame.render_widget(&help_line, help_area);
            })
            .map_err(|e| internal_error(format!("Failed to draw TUI: {e}")))?;

        if event::poll(Duration::from_millis(100))
            .map_err(|e| internal_error(format!("Failed to poll for TUI events: {e}")))?
            && let Event::Key(KeyEvent {
                code, modifiers, ..
            }) = event::read()
                .map_err(|e| internal_error(format!("Failed to read TUI events: {e}")))?
        {
            #[expect(clippy::single_match)] // There will soon be more matches
            match (code, modifiers) {
                (KeyCode::Char('q'), KeyModifiers::NONE) => {
                    return Ok(());
                }
                _ => {}
            }
        }
    }
}
