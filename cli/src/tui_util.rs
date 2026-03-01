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
use std::sync::Mutex;

use crossterm::ExecutableCommand as _;
use crossterm::event::DisableMouseCapture;
use crossterm::event::EnableMouseCapture;
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use ratatui::buffer::Buffer;
use ratatui::layout::Margin;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Block;
use ratatui::widgets::ScrollDirection;
use ratatui::widgets::Scrollbar;
use ratatui::widgets::ScrollbarOrientation;
use ratatui::widgets::ScrollbarState;
use ratatui::widgets::StatefulWidget;
use ratatui::widgets::Widget;

type RenderWindowAtOffset<'a> = dyn FnOnce(Rect, usize, &mut Buffer) + 'a;

pub struct ScrollableItem<'a> {
    height: usize,
    render_window_at_offset: Box<RenderWindowAtOffset<'a>>,
}

impl<'a> ScrollableItem<'a> {
    pub fn new(
        height: usize,
        render_window_at_offset: impl FnOnce(Rect, usize, &mut Buffer) + 'a,
    ) -> Self {
        Self {
            height,
            render_window_at_offset: Box::new(render_window_at_offset),
        }
    }

    pub fn empty() -> Self {
        Self {
            height: 0,
            render_window_at_offset: Box::new(|_area, _offset, _buf| {}),
        }
    }

    pub fn from_text(text: impl Into<Text<'a>>) -> Self {
        let mut text = text.into();
        Self::new(text.height(), move |area, offset, buf| {
            text.lines.drain(0..offset.min(text.lines.len()));
            text.lines.truncate(area.height.into());
            text.render(area, buf);
        })
    }

    pub fn from_render(height: usize, render: impl FnOnce(Rect, &mut Buffer) + 'a) -> Self {
        Self::new(height, move |area, offset, buf| {
            let area = area.intersection(buf.area);

            // Since we are rendering using a buffer for this implementation, everything
            // must fit within a u16.
            let offset = to_u16(offset);
            let widget_height = to_u16(height);

            if offset == 0 && area.height == widget_height {
                render(area, buf);
                return;
            }

            // Since everything is synchronous, we can keep a single reusable buffer for
            // rendering widgets that are only partially visible.
            static REUSABLE_BUFFER: Mutex<Buffer> = Mutex::new(Buffer {
                area: Rect::new(0, 0, 0, 0),
                content: Vec::new(),
            });

            let mut temp_buf = REUSABLE_BUFFER.lock().unwrap();

            // Set up temporary buffer with enough space to render the widget
            let temp_buf_area = Rect::new(0, 0, area.width, widget_height);
            temp_buf.resize(temp_buf_area);

            // Copy cells in the window from the main buffer to the temporary buffer
            let y_max = area.height.saturating_add(offset).min(widget_height);
            for y in offset..y_max {
                for x in 0..area.width {
                    *temp_buf.cell_mut((x, y)).unwrap() =
                        buf.cell((x + area.x, y - offset + area.y)).unwrap().clone();
                }
            }

            render(temp_buf_area, &mut temp_buf);

            // Copy cells in the window back to the main buffer
            for y in offset..y_max {
                for x in 0..area.width {
                    *buf.cell_mut((x + area.x, y - offset + area.y)).unwrap() =
                        temp_buf.cell((x, y)).unwrap().clone();
                }
            }
        })
    }

    pub fn from_widget<W: Widget + 'a>(height: usize, get_widget: impl FnOnce() -> W + 'a) -> Self {
        Self::from_render(height, move |area, buf| get_widget().render(area, buf))
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn render_window_at_offset(self, area: Rect, offset: usize, buf: &mut Buffer) {
        (self.render_window_at_offset)(area, offset, buf);
    }
}

#[derive(Default)]
pub struct ScrollableBlock<'a> {
    pub block: Block<'a>,
    pub contents: Vec<ScrollableItem<'a>>,
}

impl<'a> ScrollableBlock<'a> {
    pub fn new<I>(items: I) -> Self
    where
        I: IntoIterator<Item = ScrollableItem<'a>>,
    {
        Self {
            block: Block::new(),
            contents: items.into_iter().collect(),
        }
    }

    pub fn from_text(text: impl Into<Text<'a>>) -> Self {
        Self::new([ScrollableItem::from_text(text)])
    }

    pub fn block(mut self, block: Block<'a>) -> Self {
        self.block = block;
        self
    }
}

pub type ScrollOffset = usize;

impl StatefulWidget for ScrollableBlock<'_> {
    type State = ScrollOffset;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let inner_area = self.block.inner(area);

        self.block.render(area, buf);

        let content_height = self.contents.iter().map(|item| item.height).sum();
        render_scrollbar(content_height, area.inner(Margin::new(0, 1)), buf, state);

        let window_start = *state;
        let window_end = window_start.saturating_add(usize::from(inner_area.height));

        let mut current_content_offset: usize = 0;
        for item in self.contents {
            // If we already moved past the end of the visible window, we're done.
            if current_content_offset >= window_end {
                break;
            }

            let content_offset_start = current_content_offset;
            let content_offset_end = content_offset_start.saturating_add(item.height);
            current_content_offset = content_offset_end;

            // We don't need to render anything if we haven't reached the visible window.
            if content_offset_end <= window_start {
                continue;
            }

            let item_start = window_start.max(content_offset_start);
            let item_end = window_end.min(content_offset_end);

            let item_height = to_u16(item_end - item_start);
            let item_y = to_u16(item_start - window_start).saturating_add(inner_area.y);
            let item_area = Rect::new(inner_area.x, item_y, inner_area.width, item_height);

            let item_offset = item_start - content_offset_start;
            item.render_window_at_offset(item_area, item_offset, buf);
        }
    }
}

pub fn render_scrollbar(content_height: usize, area: Rect, buf: &mut Buffer, offset: &mut usize) {
    if content_height <= area.height.into() {
        *offset = 0;
        return;
    }

    let max_scroll = content_height.saturating_sub(usize::from(area.height).saturating_sub(1));
    *offset = (*offset).min(max_scroll.saturating_sub(1));

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None);
    let mut state = ScrollbarState::default()
        .position(*offset)
        .content_length(max_scroll);

    scrollbar.render(area, buf, &mut state);
}

pub fn scroll_offset_by(offset: &mut usize, direction: ScrollDirection, amount: u16) {
    *offset = match direction {
        ScrollDirection::Backward => offset.saturating_sub(amount.into()),
        ScrollDirection::Forward => offset.saturating_add(amount.into()),
    };
}

pub fn to_u16(position: usize) -> u16 {
    position.try_into().unwrap_or(u16::MAX)
}

fn sliding_window<T: PartialEq>(mut items: Vec<T>, current: &T, context: usize) -> Vec<T> {
    let current_index = items
        .iter()
        .position(|selection| selection == current)
        .unwrap_or(0);

    let window_size = context * 2 + 1;
    let start_index = current_index
        .saturating_sub(context)
        .min(items.len().saturating_sub(window_size));
    items.drain(0..start_index);
    items.truncate(window_size);
    items
}

pub fn render_horizontal_list<T: PartialEq>(
    items: Vec<T>,
    current: &T,
    item_to_string: impl Fn(&T) -> String,
    area: Rect,
    buf: &mut Buffer,
) {
    let items = sliding_window(items, current, 2);

    let mut line = Line::default().right_aligned();
    if items.first() != Some(current) {
        line.push_span("🞀");
    }
    for item in &items {
        let style = if item == current {
            Style::new().reversed()
        } else {
            Style::new()
        };
        line.push_span(Span::styled(" ", style));
        line.push_span(Span::styled(item_to_string(item), style));
        line.push_span(Span::styled(" ", style));
    }
    if items.last() != Some(current) {
        line.push_span("🞂 ");
    } else {
        line.push_span("  ");
    }

    line.render(area, buf);
}

pub fn enter_tui() -> io::Result<()> {
    io::stdout().execute(EnterAlternateScreen)?;
    io::stdout().execute(EnableMouseCapture)?;
    enable_raw_mode()?;
    Ok(())
}

pub fn exit_tui() -> io::Result<()> {
    disable_raw_mode()?;
    io::stdout().execute(DisableMouseCapture)?;
    io::stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}
