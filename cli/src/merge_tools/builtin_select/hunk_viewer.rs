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

use std::borrow::Cow;
use std::cmp::Ordering;
use std::iter;
use std::sync::Arc;

use bstr::BStr;
use bstr::BString;
use bstr::ByteSlice as _;
use bstr::ByteVec as _;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use itertools::Itertools as _;
use jj_lib::backend::BackendResult;
use jj_lib::backend::CopyId;
use jj_lib::backend::TreeValue;
use jj_lib::conflict_labels::ConflictLabels;
use jj_lib::conflicts;
use jj_lib::conflicts::MaterializedFileConflictValue;
use jj_lib::diff_presentation::DiffTokenType;
use jj_lib::diff_presentation::LineCompareMode;
use jj_lib::diff_presentation::unified::DiffLineType;
use jj_lib::diff_presentation::unified::unified_diff_hunks;
use jj_lib::files;
use jj_lib::files::FromMergeHunks;
use jj_lib::files::MergeHunk;
use jj_lib::files::MergeResult;
use jj_lib::merge::Diff;
use jj_lib::merge::Merge;
use jj_lib::merge::MergedTreeValue;
use jj_lib::store::Store;
use pollster::FutureExt as _;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::layout::Size;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::style::Stylize as _;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::Borders;
use ratatui::widgets::Paragraph;
use ratatui::widgets::ScrollDirection;
use ratatui::widgets::Widget as _;

use crate::merge_tools::builtin_select::Conflict;
use crate::merge_tools::builtin_select::SelectToolResult;
use crate::merge_tools::builtin_select::TermSelection;
use crate::merge_tools::builtin_select::wrap_text;
use crate::text_util;
use crate::tui_util;
use crate::tui_util::ScrollableBlock;
use crate::tui_util::ScrollableItem;

pub struct HunkConflictResolution {
    edited_contents: Option<Merge<BString>>,
    selections: Vec<Option<HunkSelection>>,
}

pub struct HunkConflictViewerState {
    store: Arc<Store>,
    file_conflict: MaterializedFileConflictValue,
    edited_contents: Option<Merge<BString>>,
    hunks: Vec<ConflictHunk>,
    hunk_index: usize,
    context_after: BString,
    current_mode: HunkViewerMode,
    scroll_offset: usize,
    window_size: Size,
}

impl HunkConflictViewerState {
    pub fn new(
        store: Arc<Store>,
        file_conflict: MaterializedFileConflictValue,
        resolution: Option<&HunkConflictResolution>,
    ) -> Self {
        let edited_contents = resolution.and_then(|resolution| resolution.edited_contents.clone());
        let contents = edited_contents.as_ref().unwrap_or(&file_conflict.contents);

        let (mut hunks, context_after) = match files::merge_hunks(contents, store.merge_options()) {
            MergeResult::Resolved(contents) => (Vec::new(), contents),
            MergeResult::Conflict(hunks) => {
                let mut current_resolved = BString::default();
                let mut conflict_hunks = Vec::new();
                for hunk in hunks {
                    if let Some(resolved) = hunk.as_resolved() {
                        current_resolved.push_str(resolved);
                    } else {
                        let all_bases_identical = hunk.removes().all_equal();
                        let snapshot_index =
                            conflicts::pick_snapshot_index(&hunk, |term| term.as_bstr());
                        conflict_hunks.push(ConflictHunk {
                            context_before: current_resolved,
                            contents: hunk,
                            selection: None,
                            all_bases_identical,
                            snapshot_index,
                        });
                        current_resolved = BString::default();
                    }
                }
                (conflict_hunks, current_resolved)
            }
        };
        if let Some(resolution) = resolution
            && !resolution.selections.is_empty()
        {
            for (hunk, selection) in hunks.iter_mut().zip_eq(&resolution.selections) {
                hunk.selection = *selection;
            }
        }
        let hunk_index = hunks
            .iter()
            .position(|hunk| !hunk.is_resolved())
            .unwrap_or(0);
        let current_term = hunks
            .get(hunk_index)
            .and_then(|hunk| hunk.selection?.to_term_selection())
            .unwrap_or(TermSelection::Added(0));
        Self {
            store,
            file_conflict,
            edited_contents,
            hunks,
            hunk_index,
            context_after,
            current_mode: HunkViewerMode::ShowTerms(current_term),
            scroll_offset: 0,
            window_size: Size::default(),
        }
    }

    pub fn handle_press(
        &mut self,
        event: KeyEvent,
        conflict: &Conflict,
    ) -> Option<SelectToolResult> {
        match (event.code, event.modifiers) {
            // Navigate between hunks with up/down
            (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE) => self.prev_hunk(),
            (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE) => self.next_hunk(),
            // Navigate between terms with left/right
            (KeyCode::Left | KeyCode::Char('h'), KeyModifiers::NONE) => self.prev_term(),
            (KeyCode::Right | KeyCode::Char('l'), KeyModifiers::NONE) => self.next_term(),
            // Toggle current hunk selected with enter or space
            (KeyCode::Enter | KeyCode::Char(' '), KeyModifiers::NONE) => {
                let new_selection = HunkSelection::Term(self.current_mode.to_term_selection());
                if let Some(current_hunk) = self.current_hunk_mut() {
                    if current_hunk.selection.is_some() {
                        current_hunk.selection = None;
                    } else {
                        current_hunk.selection = Some(new_selection);
                    }
                }
            }
            // Use current selection for all remaining hunks with 'r'
            (KeyCode::Char('r'), KeyModifiers::NONE) => {
                let new_selection = HunkSelection::Term(self.current_mode.to_term_selection());
                for hunk in &mut self.hunks {
                    if hunk.selection.is_none() {
                        hunk.selection = Some(new_selection);
                    }
                }
            }
            (KeyCode::Char('a'), KeyModifiers::NONE) => {
                if let Some(current_hunk) = self.current_hunk_mut() {
                    current_hunk.selection = Some(HunkSelection::CombineAdded);
                }
            }
            (KeyCode::Backspace | KeyCode::Delete, KeyModifiers::NONE) => {
                if let Some(current_hunk) = self.current_hunk_mut() {
                    current_hunk.selection = Some(HunkSelection::Delete);
                }
            }
            (KeyCode::Char('s'), KeyModifiers::NONE) => {
                if let HunkViewerMode::ShowDiffs { add_index } = self.current_mode
                    && let Some(current_hunk) = self.current_hunk_mut()
                {
                    current_hunk.snapshot_index = add_index;
                }
            }
            (KeyCode::Char('b'), KeyModifiers::NONE) => self.toggle_base(),
            (KeyCode::Char('d'), KeyModifiers::NONE) => self.toggle_diff(),
            (KeyCode::Char('e'), KeyModifiers::NONE) => {
                return Some(SelectToolResult::EditConflict {
                    file_name: conflict.file_name().to_owned(),
                    contents: self.get_merged_contents(),
                    labels: self.labels().clone(),
                });
            }
            _ => {}
        }
        None
    }

    pub fn scroll_by(&mut self, direction: ScrollDirection, amount: u16) {
        tui_util::scroll_offset_by(&mut self.scroll_offset, direction, amount);
    }

    pub fn set_contents(self, new_contents: Merge<BString>) -> Self {
        let resolution = HunkConflictResolution {
            edited_contents: (new_contents != self.file_conflict.contents).then_some(new_contents),
            selections: Vec::new(),
        };
        let mut new_state = Self::new(self.store, self.file_conflict, Some(&resolution));
        new_state.scroll_offset = self.scroll_offset;
        new_state
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect, conflict: &Conflict) {
        let conflict_summary = {
            let resolved_count = self.hunks.iter().filter(|hunk| hunk.is_resolved()).count();
            let total_count = self.hunks.len();
            let color = if resolved_count == total_count {
                Color::Green
            } else if resolved_count == 0 && self.edited_contents.is_none() {
                Color::Red
            } else {
                Color::Yellow
            };

            if total_count == 0 {
                "(resolved by editor)".fg(color)
            } else {
                let edited = if self.edited_contents.is_some() {
                    ", edited"
                } else {
                    ""
                };
                format!("({resolved_count}/{total_count} conflicts resolved{edited})",).fg(color)
            }
        };

        let block = Block::bordered().title(
            Line::from(vec![
                " ".into(),
                conflict.file_name().into(),
                " ".into(),
                conflict_summary,
                " ".into(),
            ])
            .bold(),
        );

        let new_window_size = block.inner(area).as_size();
        if self.window_size != new_window_size {
            self.window_size = new_window_size;
            self.scroll_to_current_hunk(true);
        }
        let width = new_window_size.width;

        let scrollable_items = self
            .hunks
            .iter()
            .enumerate()
            .flat_map(|(index, hunk)| hunk.scrollable_items(self, index))
            .chain(iter::once(context_hunk(
                self.context_after.as_bstr(),
                width,
            )));

        let scrollable_block = ScrollableBlock::new(scrollable_items).block(block);
        let mut scroll_offset = self.scroll_offset;
        frame.render_stateful_widget(scrollable_block, area, &mut scroll_offset);
        // We have to update this after rendering to prevent a lifetime issue
        self.scroll_offset = scroll_offset;
    }

    pub fn prev_hunk(&mut self) {
        // If no hunks are present (when resolved by editor), scroll instead
        if self.hunks.is_empty() {
            self.scroll_by(ScrollDirection::Backward, 1);
            return;
        }
        if self.hunk_index > 0 {
            self.hunk_index -= 1;
            if let Some(HunkSelection::Term(selection)) = self.current_hunk().unwrap().selection {
                self.current_mode.update_from_term_selection(selection);
            }
        }
        self.scroll_to_current_hunk(true);
    }

    pub fn next_hunk(&mut self) {
        // If no hunks are present (when resolved by editor), scroll instead
        if self.hunks.is_empty() {
            self.scroll_by(ScrollDirection::Forward, 1);
            return;
        }
        if self.hunk_index + 1 < self.hunks.len() {
            self.hunk_index += 1;
            if let Some(HunkSelection::Term(selection)) = self.current_hunk().unwrap().selection {
                self.current_mode.update_from_term_selection(selection);
            }
        }
        self.scroll_to_current_hunk(true);
    }

    pub fn prev_term(&mut self) {
        if let Some(current_hunk) = self.current_hunk() {
            if !current_hunk.is_resolved() {
                self.current_mode.prev();
            }
            self.scroll_to_current_hunk(false);
        }
    }

    pub fn next_term(&mut self) {
        if let Some(current_hunk) = self.current_hunk() {
            if !current_hunk.is_resolved() {
                self.current_mode.next(self.num_sides());
            }
            self.scroll_to_current_hunk(false);
        }
    }

    pub fn toggle_base(&mut self) {
        if let Some(current_hunk) = self.current_hunk()
            && !current_hunk.is_resolved()
            && let HunkViewerMode::ShowTerms(current_term) = &mut self.current_mode
        {
            current_term.toggle_base();
            self.scroll_to_current_hunk(false);
        }
    }

    pub fn toggle_diff(&mut self) {
        let Some(current_hunk) = self.current_hunk() else {
            return;
        };
        if current_hunk.is_resolved() {
            return;
        }
        self.current_mode = match self.current_mode {
            HunkViewerMode::ShowTerms(term) => {
                let TermSelection::Added(add_index) = term else {
                    return;
                };

                HunkViewerMode::ShowDiffs { add_index }
            }
            HunkViewerMode::ShowDiffs { add_index, .. } => {
                HunkViewerMode::ShowTerms(TermSelection::Added(add_index))
            }
        };
        self.scroll_to_current_hunk(false);
    }

    pub fn scroll_to_current_hunk(&mut self, enforce_context: bool) {
        let Some(current_hunk) = self.current_hunk() else {
            return;
        };
        let start_offset: usize = self
            .hunks
            .iter()
            .take(self.hunk_index)
            .enumerate()
            .flat_map(|(index, hunk)| hunk.scrollable_items(self, index))
            .chain(iter::once(
                current_hunk.context_scrollable_item(self.window_size.width),
            ))
            .map(|item| item.height())
            .sum();
        let end_offset = start_offset
            + current_hunk
                .hunk_scrollable_item(self, self.hunk_index)
                .height();

        let window_height = usize::from(self.window_size.height);
        let context = if enforce_context {
            let available_space = window_height.saturating_sub(end_offset - start_offset);
            (available_space / 2).min(5)
        } else {
            0
        };

        let align_start_offset = start_offset.saturating_sub(context);
        let align_end_offset = (end_offset + context).saturating_sub(window_height);

        if align_start_offset >= align_end_offset {
            self.scroll_offset = self
                .scroll_offset
                .clamp(align_end_offset, align_start_offset);
        } else if enforce_context {
            self.scroll_offset = align_start_offset;
        } else {
            self.scroll_offset = self
                .scroll_offset
                .clamp(align_start_offset, align_end_offset);
        }
    }

    fn current_hunk(&self) -> Option<&ConflictHunk> {
        self.hunks.get(self.hunk_index)
    }

    fn current_hunk_mut(&mut self) -> Option<&mut ConflictHunk> {
        self.hunks.get_mut(self.hunk_index)
    }

    fn file_contents(&self) -> &Merge<BString> {
        self.edited_contents
            .as_ref()
            .unwrap_or(&self.file_conflict.contents)
    }

    fn num_sides(&self) -> usize {
        self.file_contents().num_sides()
    }

    fn labels(&self) -> &ConflictLabels {
        &self.file_conflict.labels
    }

    fn get_merged_contents(&self) -> Merge<BString> {
        FromMergeHunks::from_hunks(
            self.hunks
                .iter()
                .flat_map(|hunk| {
                    [
                        MergeHunk::resolved(hunk.context_before.as_bstr().into()),
                        hunk.resolved_contents().map_or_else(
                            || MergeHunk::Borrowed(hunk.contents.map(|term| term.as_bstr())),
                            MergeHunk::resolved,
                        ),
                    ]
                })
                .chain(iter::once(MergeHunk::resolved(
                    self.context_after.as_bstr().into(),
                ))),
        )
    }

    pub fn confirm(
        &self,
        conflict: &Conflict,
    ) -> BackendResult<Option<(MergedTreeValue, HunkConflictResolution)>> {
        // If no changes were made, we shouldn't record any resolution.
        if self.edited_contents.is_none() && !self.hunks.iter().any(|hunk| hunk.is_resolved()) {
            return Ok(None);
        }
        let simplified_file_ids = self
            .get_merged_contents()
            .try_map_async(async |term| {
                self.store
                    .write_file(&conflict.path, &mut &term[..])
                    .await
                    .map(Some)
            })
            .block_on()?;
        let new_file_ids = if simplified_file_ids.is_resolved()
            || simplified_file_ids.num_sides() == self.file_conflict.unsimplified_ids.num_sides()
        {
            simplified_file_ids
        } else {
            self.file_conflict
                .unsimplified_ids
                .clone()
                .update_from_simplified(simplified_file_ids)
        };
        // Since deletions are resolved using the other view, we will never have an
        // executable bit conflict.
        let executable = self.file_conflict.executable.unwrap_or(false);
        // TODO: if the conflict is only partially resolved, we may want to preserve the
        // executable bit from the original terms.
        let resolved_value = new_file_ids.map(|id| {
            id.as_ref().map(|id| {
                TreeValue::File {
                    id: id.clone(),
                    executable,
                    // TODO: allow selecting copy ID
                    copy_id: self
                        .file_conflict
                        .copy_id
                        .clone()
                        .unwrap_or_else(CopyId::placeholder),
                }
            })
        });
        Ok(Some((
            resolved_value,
            HunkConflictResolution {
                edited_contents: self.edited_contents.clone(),
                selections: self.hunks.iter().map(|hunk| hunk.selection).collect_vec(),
            },
        )))
    }
}

struct ConflictHunk {
    context_before: BString,
    contents: Merge<BString>,
    selection: Option<HunkSelection>,
    // If every base is identical, we can show a diff against the common base for each side, so
    // there's no need to show one of them as a snapshot. This especially helps in the common case
    // where there's only 2 sides.
    all_bases_identical: bool,
    snapshot_index: usize,
}

impl ConflictHunk {
    fn resolved_contents(&self) -> Option<Cow<'_, BStr>> {
        self.selection
            .map(|selection| selection.select_from(&self.contents))
    }

    fn is_resolved(&self) -> bool {
        self.selection.is_some()
    }

    fn context_scrollable_item(&self, width: u16) -> ScrollableItem<'_> {
        context_hunk(self.context_before.as_bstr(), width)
    }

    fn hunk_scrollable_item<'a>(
        &'a self,
        state: &'a HunkConflictViewerState,
        index: usize,
    ) -> ScrollableItem<'a> {
        let is_current_hunk = state.hunk_index == index;

        // TODO: trailing newline handling
        if let Some(selected) = self.resolved_contents() {
            let text = content_to_text(selected, state.window_size.width);
            ScrollableItem::from_widget(text.height() + 2, move || {
                let style = if is_current_hunk {
                    Style::new().light_blue()
                } else {
                    Style::new()
                };
                let block = Block::new()
                    .borders(Borders::TOP | Borders::BOTTOM)
                    .border_style(Style::new().dim())
                    .border_type(BorderType::LightDoubleDashed);
                Paragraph::new(text).block(block).style(style)
            })
        } else if is_current_hunk {
            self.current_hunk_scrollable_item(state, index)
        } else {
            ScrollableItem::from_widget(3, move || {
                Paragraph::new(format!("🞂 Conflict {}/{}", index + 1, state.hunks.len()).bold())
                    .block(
                        Block::bordered()
                            .borders(Borders::TOP | Borders::BOTTOM)
                            .border_type(BorderType::Thick),
                    )
                    .style(Style::new().red())
            })
        }
    }

    fn current_hunk_scrollable_item<'a>(
        &'a self,
        state: &'a HunkConflictViewerState,
        index: usize,
    ) -> ScrollableItem<'a> {
        let is_diff_mode = matches!(state.current_mode, HunkViewerMode::ShowDiffs { .. });

        let (base_term, term) = state
            .current_mode
            .panels(self.all_bases_identical, self.snapshot_index);
        let (base_text, text) = if let Some(base_term) = base_term {
            let removed = base_term.select_from(&self.contents);
            let added = term.select_from(&self.contents);
            let width = state.window_size.width.saturating_sub(1) / 2;
            let (left, right) = diff_to_text_columns(removed.as_bstr(), added.as_bstr(), width);
            (Some(left), right)
        } else {
            let text = content_to_text(
                term.select_from(&self.contents).into(),
                state.window_size.width,
            );
            (None, text)
        };

        let label_height: u16 = if is_diff_mode { 2 } else { 1 };
        let body_height = if let Some(base_text) = &base_text {
            text.height().max(base_text.height())
        } else {
            text.height()
        };

        let height = body_height + usize::from(label_height) + 4;
        ScrollableItem::from_render(height, move |area, buf| {
            let block = Block::new()
                .borders(Borders::TOP | Borders::BOTTOM)
                .border_type(BorderType::Thick)
                .style(Style::new().light_blue());

            let inner_area = block.inner(area);
            block.render(area, buf);

            let [header_top_area, mut header_bottom_area, body_area] = Layout::default()
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(label_height),
                    Constraint::Min(1),
                ])
                .areas(inner_area);

            format!("🞃 Conflict {}/{}", index + 1, state.hunks.len())
                .bold()
                .render(header_top_area, buf);

            let options = state
                .current_mode
                .options_for_current_mode(self.contents.num_sides());
            tui_util::render_horizontal_list(
                options,
                &state.current_mode,
                |mode| mode.option_label(self.contents.num_sides()),
                header_top_area,
                buf,
            );

            // Don't use the first 2 columns since we want the text to align.
            header_bottom_area.x += 2;
            header_bottom_area.width = header_bottom_area.width.saturating_sub(2);

            let term_label = term.select_from_labels(state.labels());
            let label_text = if let Some(base_term) = base_term {
                let base_label = base_term.select_from_labels(state.labels());
                Text::from(vec![
                    Line::from(vec!["diff from: ".into(), base_label.into()]),
                    Line::from(vec!["       to: ".into(), term_label.into()]),
                ])
            } else if is_diff_mode {
                Text::from(vec![
                    term_label.into(),
                    "(using this side as the snapshot for viewing diffs)"
                        .dim()
                        .into(),
                ])
            } else {
                term_label.into()
            };

            label_text.render(header_bottom_area, buf);

            let body_block = Block::bordered().borders(Borders::TOP);
            let body_inner_area = body_block.inner(body_area);
            body_block.render(body_area, buf);

            if let Some(base_text) = base_text {
                let [left_area, separator_area, right_area] = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Fill(1),
                        Constraint::Length(1),
                        Constraint::Fill(1),
                    ])
                    .areas(body_inner_area);

                Block::new()
                    .borders(Borders::LEFT)
                    .render(separator_area, buf);

                base_text.render(left_area, buf);
                text.render(right_area, buf);
            } else {
                text.render(body_inner_area, buf);
            }
        })
    }

    fn scrollable_items<'a>(
        &'a self,
        state: &'a HunkConflictViewerState,
        index: usize,
    ) -> impl IntoIterator<Item = ScrollableItem<'a>> {
        let context_item = self.context_scrollable_item(state.window_size.width);
        let hunk_item = self.hunk_scrollable_item(state, index);
        [context_item, hunk_item]
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum HunkViewerMode {
    ShowTerms(TermSelection),
    ShowDiffs { add_index: usize },
}

impl HunkViewerMode {
    fn to_term_selection(self) -> TermSelection {
        match self {
            Self::ShowTerms(term) => term,
            Self::ShowDiffs { add_index } => TermSelection::Added(add_index),
        }
    }

    fn update_from_term_selection(&mut self, new_term: TermSelection) {
        if let Self::ShowDiffs { add_index } = self
            && let TermSelection::Added(new_add_index) = new_term
        {
            *add_index = new_add_index;
        } else {
            *self = Self::ShowTerms(new_term);
        }
    }

    fn options_for_current_mode(&self, num_sides: usize) -> Vec<Self> {
        match *self {
            Self::ShowTerms(term) => term
                .options_for_selection(num_sides)
                .into_iter()
                .map(Self::ShowTerms)
                .collect_vec(),
            Self::ShowDiffs { .. } => (0..num_sides)
                .map(|add_index| Self::ShowDiffs { add_index })
                .collect_vec(),
        }
    }

    fn option_label(&self, num_sides: usize) -> String {
        match self {
            Self::ShowTerms(term) => term.option_label(num_sides),
            Self::ShowDiffs { add_index } => (add_index + 1).to_string(),
        }
    }

    fn panels(
        &self,
        all_bases_identical: bool,
        snapshot_index: usize,
    ) -> (Option<TermSelection>, TermSelection) {
        match *self {
            Self::ShowTerms(term) => (None, term),
            Self::ShowDiffs { add_index } => {
                let removed_index = match add_index.cmp(&snapshot_index) {
                    Ordering::Less => Some(add_index),
                    Ordering::Equal if all_bases_identical => Some(add_index.saturating_sub(1)),
                    Ordering::Equal => None,
                    Ordering::Greater => Some(add_index - 1),
                };
                (
                    removed_index.map(TermSelection::Removed),
                    TermSelection::Added(add_index),
                )
            }
        }
    }

    fn prev(&mut self) {
        match self {
            Self::ShowTerms(term) => {
                term.prev();
            }
            Self::ShowDiffs { add_index, .. } => {
                *add_index = add_index.saturating_sub(1);
            }
        }
    }

    fn next(&mut self, num_sides: usize) {
        match self {
            Self::ShowTerms(term) => {
                term.next(num_sides);
            }
            Self::ShowDiffs { add_index, .. } => {
                *add_index = (*add_index + 1).min(num_sides - 1);
            }
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum HunkSelection {
    Term(TermSelection),
    Delete,
    CombineAdded,
}

impl HunkSelection {
    fn select_from<'a>(&self, contents: &'a Merge<BString>) -> Cow<'a, BStr> {
        match self {
            Self::Term(term) => Cow::Borrowed(term.select_from(contents).as_bstr()),
            Self::Delete => Cow::Borrowed(BStr::new("")),
            Self::CombineAdded => Cow::Owned(
                contents
                    .as_ref()
                    .simplify()
                    .adds()
                    .map(|&hunk| {
                        let mut hunk = Cow::from(hunk);
                        if hunk.last().is_some_and(|&ch| ch != b'\n') {
                            // TODO: use appropriate line endings
                            hunk.to_mut().push(b'\n');
                        }
                        hunk
                    })
                    .unique()
                    .join("")
                    .into(),
            ),
        }
    }

    fn to_term_selection(self) -> Option<TermSelection> {
        if let Self::Term(term) = self {
            Some(term)
        } else {
            None
        }
    }
}

fn context_hunk(hunk: &BStr, width: u16) -> ScrollableItem<'_> {
    if hunk.is_empty() {
        ScrollableItem::empty()
    } else {
        ScrollableItem::from_text(wrap_text(hunk.into(), width))
    }
}

fn content_to_text(content: Cow<'_, BStr>, width: u16) -> Text<'_> {
    content_to_text_styled(content, width, Style::new())
}

fn content_to_text_styled(content: Cow<'_, BStr>, width: u16, style: Style) -> Text<'_> {
    if content.is_empty() {
        Text::from("(empty)".dim()).centered()
    } else {
        wrap_text(content, width).style(style)
    }
}

fn diff_to_text_columns<'a>(
    removed: &'a BStr,
    added: &'a BStr,
    width: u16,
) -> (Text<'a>, Text<'a>) {
    // If either side is empty, we want to show "(empty)" as appropriate.
    if removed.is_empty() || added.is_empty() {
        return (
            content_to_text_styled(removed.into(), width, Style::new().red().bold()),
            content_to_text_styled(added.into(), width, Style::new().green().bold()),
        );
    }

    let diff = unified_diff_hunks(
        Diff::new(removed, added),
        usize::MAX,
        LineCompareMode::Exact,
    );

    // If there are no changes, return both sides but dim.
    if diff.is_empty() {
        return (
            wrap_text(removed.into(), width).fg(Color::Reset).dim(),
            wrap_text(added.into(), width).fg(Color::Reset).dim(),
        );
    }

    let mut removed_text = Text::default().style(Style::new().fg(Color::Reset));
    let mut added_text = Text::default().style(Style::new().fg(Color::Reset));
    let padding_line = Line::from("~".blue().dim());
    for (_, lines) in &diff
        .into_iter()
        .flat_map(|hunk| hunk.lines)
        .chunk_by(|(diff_type, _)| *diff_type == DiffLineType::Context)
    {
        for (diff_type, tokens) in lines {
            let push_line = |removed_text: &mut Text, added_text: &mut Text| match diff_type {
                DiffLineType::Context => {
                    removed_text.push_line(Line::default());
                    added_text.push_line(Line::default());
                }
                DiffLineType::Removed => removed_text.push_line(Line::default()),
                DiffLineType::Added => added_text.push_line(Line::default()),
            };

            let different_style = match diff_type {
                DiffLineType::Context => Style::new(),
                DiffLineType::Removed => Style::new().red().bold(),
                DiffLineType::Added => Style::new().green().bold(),
            };
            push_line(&mut removed_text, &mut added_text);
            let mut line_len = 0;
            for (token_type, token) in tokens {
                // TODO: we should use the same wrapping algorithm as `text_util::wrap_bytes`
                for byte_fragment in text_util::split_byte_line_to_words(token) {
                    if line_len + byte_fragment.word_width > width.into() {
                        push_line(&mut removed_text, &mut added_text);
                        line_len = 0;
                    }
                    line_len += byte_fragment.word_width + byte_fragment.whitespace_len;
                    let word_start = text_util::byte_offset_from(token, byte_fragment.word);
                    let word_len = byte_fragment.word.len() + byte_fragment.whitespace_len;
                    let word = &token[word_start..word_start + word_len];

                    let span = match token_type {
                        DiffTokenType::Matching => word.to_str_lossy().into(),
                        DiffTokenType::Different => {
                            Span::styled(word.to_str_lossy(), different_style)
                        }
                    };
                    match diff_type {
                        DiffLineType::Context => {
                            removed_text.push_span(span.clone());
                            added_text.push_span(span);
                        }
                        DiffLineType::Removed => removed_text.push_span(span),
                        DiffLineType::Added => added_text.push_span(span),
                    }
                }
            }
        }
        match removed_text.height().cmp(&added_text.height()) {
            Ordering::Less => removed_text.extend(iter::repeat_n(
                padding_line.clone(),
                added_text.height() - removed_text.height(),
            )),
            Ordering::Equal => {}
            Ordering::Greater => added_text.extend(iter::repeat_n(
                padding_line.clone(),
                removed_text.height() - added_text.height(),
            )),
        }
    }

    (removed_text, added_text)
}
