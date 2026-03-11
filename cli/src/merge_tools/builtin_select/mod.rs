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
use std::io;

use bstr::BStr;
use bstr::BString;
use bstr::ByteSlice as _;
use crossterm::event;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use crossterm::event::MouseEvent;
use crossterm::event::MouseEventKind;
use itertools::Itertools as _;
use jj_lib::backend::BackendResult;
use jj_lib::conflict_labels::ConflictLabels;
use jj_lib::conflicts;
use jj_lib::conflicts::ConflictMarkerStyle;
use jj_lib::conflicts::ConflictMaterializeOptions;
use jj_lib::file_util;
use jj_lib::files::FromMergeHunks;
use jj_lib::files::MergeHunk;
use jj_lib::matchers::FilesMatcher;
use jj_lib::matchers::Matcher;
use jj_lib::merge::Merge;
use jj_lib::merge::MergedTreeValue;
use jj_lib::merged_tree::MergedTree;
use jj_lib::merged_tree_builder::MergedTreeBuilder;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use pollster::FutureExt as _;
use ratatui::Terminal;
use ratatui::layout::Margin;
use ratatui::layout::Rect;
use ratatui::prelude::CrosstermBackend;
use ratatui::style::Style;
use ratatui::style::Stylize as _;
use ratatui::text::Line;
use ratatui::text::Text;
use ratatui::widgets::Block;
use ratatui::widgets::List;
use ratatui::widgets::ListState;
use ratatui::widgets::ScrollDirection;
use thiserror::Error;

use crate::description_util::TextEditor;
use crate::merge_tools::builtin_select::hunk_viewer::HunkConflictResolution;
use crate::merge_tools::builtin_select::hunk_viewer::HunkConflictViewerState;
use crate::merge_tools::builtin_select::term_viewer::TermConflictViewerState;
use crate::text_util;
use crate::tui_util;
use crate::tui_util::enter_tui;
use crate::tui_util::exit_tui;
use crate::ui::Ui;

mod hunk_viewer;
mod term_viewer;

#[derive(Debug, Error)]
pub enum BuiltinSelectToolError {
    #[error("Canceled by user")]
    CanceledByUser,
    #[error(transparent)]
    Backend(#[from] jj_lib::backend::BackendError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub async fn edit_merge_builtin_select(
    ui: &Ui,
    tree: &MergedTree,
    repo_paths: &[&RepoPath],
    text_editor: &TextEditor,
    conflict_marker_style: ConflictMarkerStyle,
) -> Result<MergedTree, BuiltinSelectToolError> {
    // TODO: accept matcher directly?
    let matcher = FilesMatcher::new(repo_paths);
    let mut select_tool = SelectTool::new(tree, &matcher).await?;

    loop {
        enter_tui()?;
        let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        terminal.clear()?;

        let result = select_tool.run_tui(&mut terminal);
        drop(terminal);
        exit_tui()?;

        match result? {
            SelectToolResult::Quit => return Err(BuiltinSelectToolError::CanceledByUser),
            SelectToolResult::Confirm(new_tree) => return Ok(new_tree),
            SelectToolResult::EditConflict {
                file_name,
                contents,
                labels,
            } => {
                let conflict_marker_len =
                    conflicts::choose_materialized_conflict_marker_len(&contents);
                let options = ConflictMaterializeOptions {
                    marker_style: conflict_marker_style,
                    marker_len: Some(conflict_marker_len),
                    merge: tree.store().merge_options().clone(),
                };
                let materialized =
                    conflicts::materialize_merge_result_to_bytes(&contents, &labels, &options);
                match text_editor.edit_str(materialized, Some(&format!("-{file_name}"))) {
                    Ok(edited) => {
                        let new_contents = if let Some(parsed) = conflicts::parse_conflict(
                            edited.as_bytes(),
                            contents.num_sides(),
                            conflict_marker_len,
                        ) {
                            FromMergeHunks::from_hunks(parsed.into_iter().map(MergeHunk::Owned))
                        } else {
                            Merge::resolved(edited.into())
                        };
                        // Only update file if new contents is different
                        if new_contents != contents {
                            select_tool.on_edit_complete(new_contents)?;
                        }
                    }
                    Err(err) => {
                        // TODO: better error handling
                        writeln!(ui.error_with_heading("Error: "), "{err}")?;
                    }
                }
            }
        }
    }
}

pub enum SelectToolResult {
    Quit,
    Confirm(MergedTree),
    EditConflict {
        file_name: String,
        contents: Merge<BString>,
        labels: ConflictLabels,
    },
}

pub struct SelectTool<'a> {
    tree: &'a MergedTree,
    conflicts: Vec<Conflict>,
    conflict_list_state: ListState,
    conflict_viewer_state: Option<ConflictViewerState>,
}

impl<'a> SelectTool<'a> {
    pub async fn new(tree: &'a MergedTree, matcher: &dyn Matcher) -> BackendResult<Self> {
        let mut conflicts = Vec::new();
        for (path, unsimplified_conflict) in tree.conflicts_matching(matcher) {
            let unsimplified_conflict = unsimplified_conflict?;
            conflicts.push(Conflict {
                path,
                unsimplified_conflict,
                resolution: None,
            });
        }
        let mut state = Self {
            tree,
            conflicts,
            conflict_list_state: ListState::default().with_selected(Some(0)),
            conflict_viewer_state: None,
        };
        // If there's only one conflict, we can skip the file list
        if state.conflicts.len() == 1 {
            state.view_current_conflict()?;
        }
        Ok(state)
    }

    pub fn run_tui<B>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<SelectToolResult, BuiltinSelectToolError>
    where
        B: ratatui::backend::Backend<Error = io::Error>,
    {
        loop {
            terminal.draw(|frame| {
                self.render(frame, frame.area());
            })?;

            loop {
                match event::read()? {
                    Event::Key(event) => {
                        if let Some(result) = self.handle_input(event)? {
                            return Ok(result);
                        }
                    }
                    Event::Mouse(MouseEvent {
                        kind: MouseEventKind::ScrollUp,
                        ..
                    }) => self.scroll_by(ScrollDirection::Backward, 1),
                    Event::Mouse(MouseEvent {
                        kind: MouseEventKind::ScrollDown,
                        ..
                    }) => self.scroll_by(ScrollDirection::Forward, 1),
                    // We need to re-render when the window is resized.
                    Event::Resize(_, _) => {}
                    // If the event is unrecognized, continue waiting until a recognized event. This
                    // prevents us from having to re-render every time the mouse moves.
                    _ => continue,
                }
                break;
            }
        }
    }

    pub fn handle_input(&mut self, event: KeyEvent) -> BackendResult<Option<SelectToolResult>> {
        if !event.is_press() {
            return Ok(None);
        }

        match (event.code, event.modifiers) {
            // Allow scrolling with page up/down or control + arrow keys
            (KeyCode::PageUp, KeyModifiers::NONE)
            | (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                self.scroll_by(ScrollDirection::Backward, 10);
                return Ok(None);
            }
            (KeyCode::PageDown, KeyModifiers::NONE)
            | (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::CONTROL) => {
                self.scroll_by(ScrollDirection::Forward, 10);
                return Ok(None);
            }
            // Can move up and down one line at a time by holding shift
            (KeyCode::Up | KeyCode::Char('K'), KeyModifiers::SHIFT) => {
                self.scroll_by(ScrollDirection::Backward, 1);
                return Ok(None);
            }
            (KeyCode::Down | KeyCode::Char('J'), KeyModifiers::SHIFT) => {
                self.scroll_by(ScrollDirection::Forward, 1);
                return Ok(None);
            }
            // Quit on control + C, regardless of what view is open
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                return Ok(Some(SelectToolResult::Quit));
            }
            _ => {}
        }

        let conflict = &self.conflicts[self.current_conflict_index()];

        if let Some(state) = &mut self.conflict_viewer_state {
            return match (&*state, event.code, event.modifiers) {
                // In the term-level viewer, escape just closes the viewer. Both viewers support 'q'
                // to cancel.
                (ConflictViewerState::ByTerm(_), KeyCode::Esc, KeyModifiers::NONE)
                | (_, KeyCode::Char('q'), KeyModifiers::NONE) => {
                    // If there's only one conflict, there's no need to return to the file list
                    if self.conflicts.len() == 1 {
                        return Ok(Some(SelectToolResult::Quit));
                    }
                    self.conflict_viewer_state = None;
                    Ok(None)
                }
                // In the term-level viewer, enter saves changes immediately. In the hunk-level
                // viewer, escape saves any changes that were previously confirmed with enter. Both
                // viewers support 'c' to confirm.
                (ConflictViewerState::ByTerm(_), KeyCode::Enter, KeyModifiers::NONE)
                | (ConflictViewerState::ByHunk(_), KeyCode::Esc, KeyModifiers::NONE)
                | (_, KeyCode::Char('c'), KeyModifiers::NONE) => {
                    let resolution = state.confirm(conflict)?;
                    self.current_conflict_mut().resolution = resolution;
                    // If there's only one conflict, there's no need to return to the file list
                    if self.conflicts.len() == 1 {
                        let new_tree = self.write_tree()?;
                        return Ok(Some(SelectToolResult::Confirm(new_tree)));
                    }
                    self.conflict_viewer_state = None;
                    Ok(None)
                }
                _ => Ok(state.handle_press(event, conflict)),
            };
        }

        match (event.code, event.modifiers) {
            (KeyCode::Char('q'), KeyModifiers::NONE) => {
                // TODO: confirm if discarding changes
                return Ok(Some(SelectToolResult::Quit));
            }
            (KeyCode::Char('c'), KeyModifiers::NONE) => {
                let new_tree = self.write_tree()?;
                return Ok(Some(SelectToolResult::Confirm(new_tree)));
            }
            (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE) => {
                self.conflict_list_state.select_previous();
            }
            (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE) => {
                self.conflict_list_state.select_next();
            }
            (KeyCode::Enter | KeyCode::Char(' ' | 'l') | KeyCode::Right, KeyModifiers::NONE) => {
                self.view_current_conflict()?;
            }
            (KeyCode::Backspace | KeyCode::Delete, KeyModifiers::NONE) => {
                self.current_conflict_mut().resolution = None;
            }
            _ => {}
        }
        Ok(None)
    }

    pub fn scroll_by(&mut self, direction: ScrollDirection, amount: u16) {
        if let Some(state) = &mut self.conflict_viewer_state {
            state.scroll_by(direction, amount);
            return;
        }

        match direction {
            ScrollDirection::Backward => self.conflict_list_state.scroll_up_by(amount),
            ScrollDirection::Forward => self.conflict_list_state.scroll_down_by(amount),
        }
    }

    pub fn on_edit_complete(&mut self, new_contents: Merge<BString>) -> BackendResult<()> {
        match self.conflict_viewer_state.take() {
            Some(ConflictViewerState::ByHunk(state)) => {
                self.conflict_viewer_state = Some(ConflictViewerState::ByHunk(
                    state.set_contents(new_contents),
                ));
            }
            state => {
                self.conflict_viewer_state = state;
            }
        }
        Ok(())
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let conflict = &self.conflicts[self.current_conflict_index()];

        if let Some(state) = &mut self.conflict_viewer_state {
            state.render(frame, area, conflict);
            return;
        }

        let list_items = self.conflicts.iter().map(|conflict| {
            Line::from(vec![
                format!("[{}]", conflict.symbol()).bold(),
                " ".into(),
                conflict.path.as_internal_file_string().into(),
            ])
        });

        let list = List::new(list_items)
            .block(Block::bordered().title(" Conflicts ".bold()))
            .highlight_symbol("> ")
            .highlight_style(Style::new().reversed());
        let num_items = list.len();

        frame.render_stateful_widget(list, area, &mut self.conflict_list_state);

        tui_util::render_scrollbar(
            num_items,
            area.inner(Margin::new(0, 1)),
            frame.buffer_mut(),
            &mut self.conflict_list_state.offset(),
        );
    }

    fn current_conflict_index(&self) -> usize {
        self.conflict_list_state
            .selected()
            .expect("should always have selected file")
            .min(self.conflicts.len() - 1)
    }

    fn current_conflict(&self) -> &Conflict {
        let index = self.current_conflict_index();
        &self.conflicts[index]
    }

    fn current_conflict_mut(&mut self) -> &mut Conflict {
        let index = self.current_conflict_index();
        &mut self.conflicts[index]
    }

    fn view_current_conflict(&mut self) -> BackendResult<()> {
        let conflict = self.current_conflict();

        let file_conflict = conflicts::try_materialize_file_conflict_value(
            self.tree.store(),
            &conflict.path,
            &conflict.unsimplified_conflict,
            self.tree.labels(),
        )
        .block_on()?;

        // If there are any deleted files left after simplification, we treat it like a
        // non-file merge because splitting into hunks is useless if a deletion is
        // involved, since the hunk would consist of the entire file anyway. Similarly,
        // we don't use the hunk-based conflict viewer for binary files.
        let new_state = if let Some(file_conflict) = file_conflict
            && file_conflict.ids.iter().all(Option::is_some)
            && file_conflict
                .contents
                .iter()
                .all(|contents| !file_util::is_binary(contents))
        {
            ConflictViewerState::ByHunk(HunkConflictViewerState::new(
                self.tree.store().clone(),
                file_conflict,
                conflict
                    .resolution
                    .as_ref()
                    .and_then(|(_, resolution)| resolution.to_hunk_resolution()),
            ))
        } else {
            ConflictViewerState::ByTerm(TermConflictViewerState::new(
                self.tree,
                conflict,
                conflict
                    .resolution
                    .as_ref()
                    .and_then(|(_, resolution)| resolution.to_term_resolution()),
            )?)
        };
        self.conflict_viewer_state = Some(new_state);
        Ok(())
    }

    fn write_tree(&self) -> BackendResult<MergedTree> {
        if self.conflicts.iter().all(|file| file.resolution.is_none()) {
            return Ok(self.tree.clone());
        }

        let mut tree_builder = MergedTreeBuilder::new(self.tree.clone());
        for file in &self.conflicts {
            if let Some((resolved_value, _)) = &file.resolution {
                tree_builder.set_or_remove(file.path.clone(), resolved_value.clone());
            }
        }
        tree_builder.write_tree().block_on()
    }
}

struct Conflict {
    path: RepoPathBuf,
    unsimplified_conflict: MergedTreeValue,
    resolution: Option<(MergedTreeValue, ConflictResolution)>,
}

impl Conflict {
    pub fn symbol(&self) -> char {
        match &self.resolution {
            None => ' ',
            Some((resolved_value, _)) if resolved_value.is_resolved() => '✓',
            Some(_) => '-',
        }
    }

    pub fn file_name(&self) -> &str {
        self.path
            .components()
            .next_back()
            .unwrap()
            .as_internal_str()
    }
}

enum ConflictResolution {
    ByTerm(TermSelection),
    ByHunk(HunkConflictResolution),
}

impl ConflictResolution {
    pub fn to_term_resolution(&self) -> Option<&TermSelection> {
        match self {
            Self::ByTerm(resolution) => Some(resolution),
            Self::ByHunk(_) => None,
        }
    }

    pub fn to_hunk_resolution(&self) -> Option<&HunkConflictResolution> {
        match self {
            Self::ByTerm(_) => None,
            Self::ByHunk(resolution) => Some(resolution),
        }
    }
}

enum ConflictViewerState {
    ByTerm(TermConflictViewerState),
    ByHunk(HunkConflictViewerState),
}

impl ConflictViewerState {
    pub fn handle_press(
        &mut self,
        event: KeyEvent,
        conflict: &Conflict,
    ) -> Option<SelectToolResult> {
        match self {
            Self::ByTerm(state) => {
                state.handle_press(event);
                None
            }
            Self::ByHunk(state) => state.handle_press(event, conflict),
        }
    }

    pub fn scroll_by(&mut self, direction: ScrollDirection, amount: u16) {
        match self {
            Self::ByTerm(state) => state.scroll_by(direction, amount),
            Self::ByHunk(state) => state.scroll_by(direction, amount),
        }
    }

    pub fn render(&mut self, frame: &mut ratatui::Frame, area: Rect, conflict: &Conflict) {
        match self {
            Self::ByTerm(state) => state.render(frame, area, conflict),
            Self::ByHunk(state) => state.render(frame, area, conflict),
        }
    }

    pub fn confirm(
        &self,
        conflict: &Conflict,
    ) -> BackendResult<Option<(MergedTreeValue, ConflictResolution)>> {
        match self {
            Self::ByTerm(state) => {
                let (resolved_value, resolution) = state.confirm();
                Ok(Some((
                    resolved_value,
                    ConflictResolution::ByTerm(resolution),
                )))
            }
            Self::ByHunk(state) => match state.confirm(conflict)? {
                Some((resolved_value, resolution)) => Ok(Some((
                    resolved_value,
                    ConflictResolution::ByHunk(resolution),
                ))),
                None => Ok(None),
            },
        }
    }
}

#[derive(Copy, Clone, Eq, PartialEq)]
enum TermSelection {
    Added(usize),
    Removed(usize),
}

impl TermSelection {
    fn next(&mut self, num_sides: usize) {
        match self {
            Self::Added(add_index) => {
                *add_index = (*add_index + 1).min(num_sides - 1);
            }
            Self::Removed(remove_index) => {
                *remove_index = (*remove_index + 1).min(num_sides - 2);
            }
        }
    }

    fn prev(&mut self) {
        let (Self::Added(index) | Self::Removed(index)) = self;
        *index = index.saturating_sub(1);
    }

    fn toggle_base(&mut self) {
        *self = match *self {
            Self::Added(add_index) => Self::Removed(add_index.saturating_sub(1)),
            Self::Removed(remove_index) => Self::Added(remove_index + 1),
        }
    }

    fn select_from<'a, T>(&self, merge: &'a Merge<T>) -> &'a T {
        match *self {
            Self::Added(add_index) => merge.get_add(add_index).unwrap(),
            Self::Removed(remove_index) => merge.get_remove(remove_index).unwrap(),
        }
    }

    fn select_from_labels<'a>(&self, labels: &'a ConflictLabels) -> Cow<'a, str> {
        match *self {
            Self::Added(add_index) => labels.get_add_or_default(add_index),
            Self::Removed(remove_index) => labels.get_remove_or_default(remove_index),
        }
    }

    fn options_for_selection(&self, num_sides: usize) -> Vec<Self> {
        match *self {
            Self::Added(_) => (0..num_sides).map(Self::Added).collect_vec(),
            Self::Removed(_) => (0..num_sides - 1).map(Self::Removed).collect_vec(),
        }
    }

    fn option_label(&self, num_sides: usize) -> String {
        match *self {
            Self::Added(add_index) => (add_index + 1).to_string(),
            Self::Removed(_) if num_sides == 2 => "base".to_owned(),
            Self::Removed(remove_index) => format!("b{}", remove_index + 1),
        }
    }
}

fn wrap_text(s: Cow<'_, BStr>, width: u16) -> Text<'_> {
    match s {
        Cow::Borrowed(b) => {
            let without_newline = match b.split_last() {
                Some((b'\n', head)) => head,
                _ => b,
            };
            text_util::wrap_bytes(without_newline, width.into())
                .into_iter()
                .map(|hunk| hunk.to_str_lossy())
                .collect()
        }
        Cow::Owned(o) => {
            let without_newline = match o.split_last() {
                Some((b'\n', head)) => head,
                _ => &o,
            };
            text_util::wrap_bytes(without_newline, width.into())
                .into_iter()
                .map(|hunk| hunk.to_str_lossy().into_owned())
                .collect()
        }
    }
}
