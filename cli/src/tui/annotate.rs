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

//! Interactive browser of file annotations.
//!
//! The view is a stack of annotated file revisions. The user can navigate the
//! lines of the current annotation and annotate the file at the parent of the
//! commit that introduced the selected line, which pushes a new entry onto the
//! stack.

use bstr::ByteSlice as _;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use jj_lib::annotate::FileAnnotator;
use jj_lib::annotate::get_file_contents;
use jj_lib::backend::MergedTreeValueExt as _;
use jj_lib::commit::Commit;
use jj_lib::diff::ContentDiff;
use jj_lib::diff::DiffHunkKind;
use jj_lib::repo::Repo;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::revset::RevsetExpression;
use jj_lib::ui_path::RepoPathUiConverter;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Widget as _;

use crate::cli_util::short_change_hash;
use crate::command_error::CommandError;
use crate::commit_templater::AnnotationLine;
use crate::tui;
use crate::tui::TuiTerminal;

/// The revision whose file content is annotated.
#[derive(Clone, Debug)]
pub(crate) enum AnnotatedRevision {
    Commit(Commit),
    /// The (auto-merged) parents of a merge commit.
    MergedParents(Commit),
}

/// Annotation of a file at a certain revision, and the selection within it.
#[derive(Clone, Debug)]
pub(crate) struct AnnotatedFile {
    revision: AnnotatedRevision,
    path: RepoPathBuf,
    lines: Vec<AnnotationLine>,
    /// Index of the selected line in `lines`
    selected: usize,
    /// Index of the first line shown in the viewport
    scroll_top: usize,
}

impl AnnotatedFile {
    pub fn new(revision: AnnotatedRevision, path: RepoPathBuf, lines: Vec<AnnotationLine>) -> Self {
        Self {
            revision,
            path,
            lines,
            selected: 0,
            scroll_top: 0,
        }
    }

    fn selected_line(&self) -> Option<&AnnotationLine> {
        self.lines.get(self.selected)
    }

    fn select(&mut self, index: usize) {
        self.selected = index.min(self.lines.len().saturating_sub(1));
    }

    fn move_selection(&mut self, delta: isize) {
        self.select(self.selected.saturating_add_signed(delta));
    }

    fn select_next_hunk(&mut self) {
        if let Some(index) = (self.selected + 1..self.lines.len())
            .find(|&index| self.lines[index].first_line_in_hunk)
        {
            self.selected = index;
        }
    }

    /// Selects the first line of the current hunk, or the previous hunk if the
    /// first line of the current hunk is already selected.
    fn select_previous_hunk(&mut self) {
        if let Some(index) = (0..self.selected).rfind(|&index| self.lines[index].first_line_in_hunk)
        {
            self.selected = index;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Down,
    Up,
    PageDown,
    PageUp,
    First,
    Last,
    NextHunk,
    PreviousHunk,
    AnnotateParent,
    ShowCommit,
    Back,
    /// Closes the commit view, or quits if it's not open
    Close,
    Quit,
}

fn action_for_key(event: KeyEvent) -> Option<Action> {
    // Shift is part of the character (e.g. 'G'), so it's ignored
    let modifiers = event.modifiers - KeyModifiers::SHIFT;
    let action = match (event.code, modifiers) {
        (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE) => Action::Down,
        (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE) => Action::Up,
        (KeyCode::PageDown | KeyCode::Char(' '), KeyModifiers::NONE)
        | (KeyCode::Char('f'), KeyModifiers::CONTROL) => Action::PageDown,
        (KeyCode::PageUp, KeyModifiers::NONE) | (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
            Action::PageUp
        }
        (KeyCode::Home | KeyCode::Char('g'), KeyModifiers::NONE) => Action::First,
        (KeyCode::End | KeyCode::Char('G'), KeyModifiers::NONE) => Action::Last,
        (KeyCode::Char(']'), KeyModifiers::NONE) => Action::NextHunk,
        (KeyCode::Char('['), KeyModifiers::NONE) => Action::PreviousHunk,
        (KeyCode::Char('p'), KeyModifiers::NONE) => Action::AnnotateParent,
        (KeyCode::Enter, KeyModifiers::NONE) => Action::ShowCommit,
        (KeyCode::Backspace | KeyCode::Char('b'), KeyModifiers::NONE) => Action::Back,
        (KeyCode::Esc | KeyCode::Char('q'), KeyModifiers::NONE) => Action::Close,
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::Quit,
        _ => return None,
    };
    Some(action)
}

const HELP_ITEMS: &[(&str, &str)] = &[
    ("↓/j", "down"),
    ("↑/k", "up"),
    ("]/[", "next/prev hunk"),
    ("enter", "show commit"),
    ("p", "annotate parent"),
    ("b", "back"),
    ("q", "quit"),
];

const COMMIT_VIEW_HELP_ITEMS: &[(&str, &str)] = &[
    ("↓/j", "down"),
    ("↑/k", "up"),
    ("space", "page down"),
    ("p", "annotate parent"),
    ("q", "close"),
];

/// The commit that introduced the selected line, shown on top of the
/// annotation.
#[derive(Clone, Debug)]
struct CommitView {
    lines: Vec<Line<'static>>,
    /// Index of the first line shown in the viewport
    scroll_top: usize,
}

impl CommitView {
    fn new(output: &[u8]) -> Self {
        let lines = tui::ansi_to_text(output)
            .lines
            .into_iter()
            .map(tui::expand_tabs)
            .collect();
        Self {
            lines,
            scroll_top: 0,
        }
    }

    fn max_scroll(&self, page_size: usize) -> usize {
        self.lines.len().saturating_sub(page_size)
    }

    fn scroll_by(&mut self, delta: isize, page_size: usize) {
        self.scroll_top = self
            .scroll_top
            .saturating_add_signed(delta)
            .min(self.max_scroll(page_size));
    }
}

#[derive(Clone, Debug)]
pub(crate) struct State {
    /// Annotations the user has navigated through. The last one is shown.
    /// Never empty.
    history: Vec<AnnotatedFile>,
    /// Message to show instead of the selected line's commit until the next
    /// key press
    message: Option<String>,
    commit_view: Option<CommitView>,
    path_converter: RepoPathUiConverter,
    /// Number of lines that fit in the viewport the last time it was rendered
    page_size: usize,
    /// Width of the viewport the last time it was rendered
    page_width: usize,
}

impl State {
    pub fn new(file: AnnotatedFile, path_converter: RepoPathUiConverter) -> Self {
        Self {
            history: vec![file],
            message: None,
            commit_view: None,
            path_converter,
            page_size: 1,
            page_width: 80,
        }
    }

    fn current(&self) -> &AnnotatedFile {
        self.history.last().unwrap()
    }

    fn current_mut(&mut self) -> &mut AnnotatedFile {
        self.history.last_mut().unwrap()
    }

    fn clamp_scroll(&mut self, viewport: Rect) {
        self.page_size = usize::from(viewport.height).max(1);
        self.page_width = usize::from(viewport.width);
        let page_size = self.page_size;
        if let Some(view) = &mut self.commit_view {
            view.scroll_top = view.scroll_top.min(view.max_scroll(page_size));
        }
        let file = self.current_mut();
        file.scroll_top =
            tui::scroll_to_selection(file.scroll_top, file.selected, page_size, file.lines.len());
    }

    /// Applies the `action`. The `show_commit` function renders a commit and
    /// its diff for the given width.
    async fn apply(
        &mut self,
        repo: &dyn Repo,
        action: Action,
        show_commit: &impl AsyncFn(&Commit, usize) -> Result<Vec<u8>, CommandError>,
    ) -> Result<(), CommandError> {
        self.message = None;
        let page_size = isize::try_from(self.page_size).unwrap_or(isize::MAX);
        if let Some(view) = &mut self.commit_view {
            match action {
                Action::Down => view.scroll_by(1, self.page_size),
                Action::Up => view.scroll_by(-1, self.page_size),
                Action::PageDown => view.scroll_by(page_size, self.page_size),
                Action::PageUp => view.scroll_by(-page_size, self.page_size),
                Action::First => view.scroll_top = 0,
                Action::Last => view.scroll_top = view.max_scroll(self.page_size),
                Action::NextHunk | Action::PreviousHunk => {}
                Action::AnnotateParent => {
                    self.commit_view = None;
                    self.annotate_parent(repo).await?;
                }
                Action::ShowCommit | Action::Back | Action::Close | Action::Quit => {
                    self.commit_view = None;
                }
            }
            return Ok(());
        }
        match action {
            Action::Down => self.current_mut().move_selection(1),
            Action::Up => self.current_mut().move_selection(-1),
            Action::PageDown => self.current_mut().move_selection(page_size),
            Action::PageUp => self.current_mut().move_selection(-page_size),
            Action::First => self.current_mut().select(0),
            Action::Last => self.current_mut().select(usize::MAX),
            Action::NextHunk => self.current_mut().select_next_hunk(),
            Action::PreviousHunk => self.current_mut().select_previous_hunk(),
            Action::AnnotateParent => self.annotate_parent(repo).await?,
            Action::ShowCommit => {
                if let Some(line) = self.current().selected_line() {
                    let output = show_commit(&line.commit, self.page_width).await?;
                    self.commit_view = Some(CommitView::new(&output));
                }
            }
            Action::Back => {
                if self.history.len() > 1 {
                    self.history.pop();
                } else {
                    self.message = Some("Already at the first annotation".to_owned());
                }
            }
            Action::Close | Action::Quit => {}
        }
        Ok(())
    }

    /// Annotates the current file at the parents of the commit that introduced
    /// the selected line.
    async fn annotate_parent(&mut self, repo: &dyn Repo) -> Result<(), CommandError> {
        let current = self.current();
        let Some(line) = current.selected_line() else {
            return Ok(());
        };
        let commit = line.commit.clone();
        let path = current.path.clone();
        let ui_path = self.path_converter.format_file_path(&path);
        let short_id = short_change_hash(commit.change_id());
        if commit.parent_ids().is_empty() {
            self.message = Some(format!("Commit {short_id} has no parents"));
            return Ok(());
        }
        let parent_tree = commit.parent_tree(repo.index()).await?;
        let parent_value = parent_tree.path_value(&path).await?;
        if parent_value.is_absent() {
            self.message = Some(format!("{ui_path} does not exist in parent of {short_id}"));
            return Ok(());
        }
        if parent_value.to_file_merge().is_none() {
            self.message = Some(format!(
                "{ui_path} is not a regular file in parent of {short_id}"
            ));
            return Ok(());
        }
        let parent_text = get_file_contents(repo.store(), &path, &parent_tree).await?;
        let text = get_file_contents(repo.store(), &path, &commit.tree()).await?;
        let parent_line = map_line_to_left(&parent_text, &text, line.original_line_number - 1);

        // Annotating the parent content as if it were in `commit` attributes
        // every line to the parents. That also works when the commit has
        // multiple parents.
        let mut annotator =
            FileAnnotator::with_file_content(commit.id(), &path, parent_text.clone());
        annotator.compute(repo, &RevsetExpression::all()).await?;
        let lines = AnnotationLine::from_annotation(repo, &annotator.to_annotation()).await?;
        let revision = if let [parent_id] = commit.parent_ids() {
            AnnotatedRevision::Commit(repo.store().get_commit_async(parent_id).await?)
        } else {
            AnnotatedRevision::MergedParents(commit)
        };
        let mut file = AnnotatedFile::new(revision, path, lines);
        file.select(parent_line);
        self.history.push(file);
        Ok(())
    }
}

/// Returns the 0-based line number in `left` that best corresponds to
/// `right_line` in `right`.
///
/// For a line in a changed hunk, that's the line at the same offset in the
/// corresponding hunk in `left`, or the last line of that hunk if it's shorter.
/// If the hunk in `left` is empty, it's the next line after the hunk.
fn map_line_to_left(left: &[u8], right: &[u8], right_line: usize) -> usize {
    let diff = ContentDiff::by_line([left, right]);
    let mut left_start = 0;
    let mut right_start = 0;
    for hunk in diff.hunk_ranges() {
        let left_count = left[hunk.ranges[0].clone()].lines_with_terminator().count();
        let right_count = right[hunk.ranges[1].clone()]
            .lines_with_terminator()
            .count();
        if right_line < right_start + right_count {
            let offset = right_line - right_start;
            return match hunk.kind {
                DiffHunkKind::Matching => left_start + offset,
                DiffHunkKind::Different => left_start + offset.min(left_count.saturating_sub(1)),
            };
        }
        left_start += left_count;
        right_start += right_count;
    }
    left_start
}

/// Runs the event loop until the user quits.
pub(crate) async fn run(
    terminal: &mut TuiTerminal,
    repo: &dyn Repo,
    mut state: State,
    render_line: impl Fn(&AnnotationLine) -> Vec<u8>,
    render_commit: impl Fn(&Commit) -> Vec<u8>,
    show_commit: impl AsyncFn(&Commit, usize) -> Result<Vec<u8>, CommandError>,
) -> Result<(), CommandError> {
    loop {
        terminal.draw(|frame| {
            let area = frame.area();
            render(
                &mut state,
                &render_line,
                &render_commit,
                frame.buffer_mut(),
                area,
            );
        })?;
        let Some(event) = tui::read_key_press()? else {
            continue;
        };
        match action_for_key(event) {
            Some(Action::Quit) => return Ok(()),
            Some(Action::Close) if state.commit_view.is_none() => return Ok(()),
            Some(action) => state.apply(repo, action, &show_commit).await?,
            None => {}
        }
    }
}

fn render(
    state: &mut State,
    render_line: impl Fn(&AnnotationLine) -> Vec<u8>,
    render_commit: impl Fn(&Commit) -> Vec<u8>,
    buf: &mut Buffer,
    area: Rect,
) {
    let [header_area, main_area, status_area, help_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    state.clamp_scroll(main_area);
    let file = state.current();

    // Header with the path and revision
    let (revision_prefix, revision_commit) = match &file.revision {
        AnnotatedRevision::Commit(commit) => ("", commit),
        AnnotatedRevision::MergedParents(commit) => ("parents of ", commit),
    };
    let mut header = vec![
        Span::styled(
            state.path_converter.format_file_path(&file.path),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" at {revision_prefix}")),
    ];
    header.extend(first_line(&render_commit(revision_commit)).spans);
    if state.history.len() > 1 {
        header.push(Span::raw(format!(" [depth {}]", state.history.len() - 1)));
    }
    Line::from(header).render(header_area, buf);

    if let Some(view) = &state.commit_view {
        let visible_lines = view
            .lines
            .iter()
            .skip(view.scroll_top)
            .take(usize::from(main_area.height));
        for (row, line) in visible_lines.enumerate() {
            let line_area = Rect {
                y: main_area.y + row as u16,
                height: 1,
                ..main_area
            };
            line.render(line_area, buf);
        }
        let last_visible = (view.scroll_top + state.page_size).min(view.lines.len());
        Span::raw(format!(
            "Lines {}-{last_visible} of {}",
            view.scroll_top + 1,
            view.lines.len()
        ))
        .render(status_area, buf);
        tui::help_line(COMMIT_VIEW_HELP_ITEMS).render(help_area, buf);
        return;
    }

    // Annotated lines
    let [marker_area, lines_area] =
        Layout::horizontal([Constraint::Length(2), Constraint::Fill(1)]).areas(main_area);
    let visible_lines = file
        .lines
        .iter()
        .enumerate()
        .skip(file.scroll_top)
        .take(usize::from(main_area.height));
    for (row, (index, line)) in visible_lines.enumerate() {
        let y = main_area.y + row as u16;
        let line_area = Rect {
            y,
            height: 1,
            ..lines_area
        };
        first_line(&render_line(line)).render(line_area, buf);
        if index == file.selected {
            let marker_area = Rect {
                y,
                height: 1,
                ..marker_area
            };
            Span::raw("▶").render(marker_area, buf);
            buf.set_style(line_area, Style::default().add_modifier(Modifier::REVERSED));
        }
    }

    // Status line with a message or the selected line's commit
    if let Some(message) = &state.message {
        Span::raw(message.as_str()).render(status_area, buf);
    } else if let Some(line) = file.selected_line() {
        first_line(&render_commit(&line.commit)).render(status_area, buf);
    }

    tui::help_line(HELP_ITEMS).render(help_area, buf);
}

/// Returns the first line of the formatted `bytes` with tabs expanded.
fn first_line(bytes: &[u8]) -> Line<'static> {
    let text = tui::ansi_to_text(bytes);
    let line = text.lines.into_iter().next().unwrap_or_default();
    tui::expand_tabs(line)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use itertools::Itertools as _;
    use jj_lib::backend::CommitId;
    use jj_lib::repo::MutableRepo;
    use pollster::FutureExt as _;
    use testutils::CommitBuilderExt as _;
    use testutils::TestRepo;
    use testutils::TestResult;
    use testutils::create_tree;
    use testutils::repo_path;

    use super::*;

    fn create_commit(
        test_repo: &TestRepo,
        mut_repo: &mut MutableRepo,
        parents: &[&Commit],
        description: &str,
        content: Option<&str>,
    ) -> Commit {
        let path_contents = content
            .map(|content| (repo_path("file"), content))
            .into_iter()
            .collect::<Vec<_>>();
        let tree = create_tree(&test_repo.repo, &path_contents);
        let parent_ids = if parents.is_empty() {
            vec![test_repo.repo.store().root_commit_id().clone()]
        } else {
            parents.iter().map(|commit| commit.id().clone()).collect()
        };
        mut_repo
            .new_commit(parent_ids, tree)
            .set_description(description)
            .write_unwrap()
    }

    fn new_state(repo: &dyn Repo, commit: &Commit) -> TestResult<State> {
        let path = repo_path("file");
        let mut annotator = FileAnnotator::from_commit(commit, path).block_on()?;
        annotator
            .compute(repo, &RevsetExpression::all())
            .block_on()?;
        let lines = AnnotationLine::from_annotation(repo, &annotator.to_annotation()).block_on()?;
        let file = AnnotatedFile::new(
            AnnotatedRevision::Commit(commit.clone()),
            path.to_owned(),
            lines,
        );
        let path_converter = RepoPathUiConverter::Fs {
            cwd: PathBuf::from("/repo"),
            base: PathBuf::from("/repo"),
        };
        Ok(State::new(file, path_converter))
    }

    fn apply(state: &mut State, repo: &dyn Repo, actions: &[Action]) {
        // Renders the commit as its description followed by numbered lines,
        // as many as the width
        let show_commit = async |commit: &Commit, width: usize| {
            let diff_lines = (1..=width).map(|i| format!("diff line {i}\n"));
            let output = itertools::chain([format!("{}\n", commit.description())], diff_lines)
                .collect::<String>();
            Ok(output.into_bytes())
        };
        for &action in actions {
            state.apply(repo, action, &show_commit).block_on().unwrap();
        }
    }

    fn selected_commit_id(state: &State) -> &CommitId {
        state.current().selected_line().unwrap().commit.id()
    }

    fn render_to_string(state: &mut State, width: u16, height: u16) -> String {
        let render_line = |line: &AnnotationLine| {
            let description = line.commit.description().trim_end();
            format!("{description}: {}", line.content).into_bytes()
        };
        let render_commit = |commit: &Commit| commit.description().as_bytes().to_vec();
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        render(state, render_line, render_commit, &mut buf, area);
        tui::buffer_to_string(&buf)
    }

    #[test]
    fn test_navigation() -> TestResult {
        let test_repo = TestRepo::init();
        let mut tx = test_repo.repo.start_transaction();
        let mut_repo = tx.repo_mut();
        let commit1 = create_commit(&test_repo, mut_repo, &[], "commit1", Some("a\nb\nc\n"));
        let commit2 = create_commit(
            &test_repo,
            mut_repo,
            &[&commit1],
            "commit2",
            Some("a\nB\nC\nd\n"),
        );
        let commit3 = create_commit(
            &test_repo,
            mut_repo,
            &[&commit2],
            "commit3",
            Some("a\nB\nC\nd\ne\n"),
        );
        let repo: &dyn Repo = mut_repo;

        let mut state = new_state(repo, &commit3)?;
        insta::assert_snapshot!(render_to_string(&mut state, 40, 9), @"
        file at commit3
        ▶ commit1: a
          commit2: B
          commit2: C
          commit2: d
          commit3: e

        commit1
        ↓/j down • ↑/k up • ]/[ next/prev hunk •
        ");

        apply(&mut state, repo, &[Action::Down, Action::Down]);
        assert_eq!(state.current().selected, 2);
        // Doesn't move past the end
        apply(&mut state, repo, &[Action::Last, Action::Down]);
        assert_eq!(state.current().selected, 4);
        apply(&mut state, repo, &[Action::First, Action::Up]);
        assert_eq!(state.current().selected, 0);

        // Hunks are [a], [B, C, d], [e]
        apply(&mut state, repo, &[Action::NextHunk]);
        assert_eq!(state.current().selected, 1);
        apply(&mut state, repo, &[Action::NextHunk]);
        assert_eq!(state.current().selected, 4);
        apply(&mut state, repo, &[Action::NextHunk]);
        assert_eq!(state.current().selected, 4);
        apply(&mut state, repo, &[Action::Up, Action::PreviousHunk]);
        assert_eq!(state.current().selected, 1);
        apply(&mut state, repo, &[Action::PreviousHunk]);
        assert_eq!(state.current().selected, 0);
        apply(&mut state, repo, &[Action::PreviousHunk]);
        assert_eq!(state.current().selected, 0);

        // The page size is the height of the viewport from the last render
        render_to_string(&mut state, 40, 5);
        apply(&mut state, repo, &[Action::PageDown]);
        assert_eq!(state.current().selected, 2);
        apply(&mut state, repo, &[Action::PageDown]);
        assert_eq!(state.current().selected, 4);
        insta::assert_snapshot!(render_to_string(&mut state, 40, 5), @"
        file at commit3
          commit2: d
        ▶ commit3: e
        commit3
        ↓/j down • ↑/k up • ]/[ next/prev hunk •
        ");
        apply(&mut state, repo, &[Action::PageUp]);
        assert_eq!(state.current().selected, 2);
        Ok(())
    }

    #[test]
    fn test_annotate_parent() -> TestResult {
        let test_repo = TestRepo::init();
        let mut tx = test_repo.repo.start_transaction();
        let mut_repo = tx.repo_mut();
        let commit1 = create_commit(&test_repo, mut_repo, &[], "commit1", Some("a\nb\nc\n"));
        let commit2 = create_commit(
            &test_repo,
            mut_repo,
            &[&commit1],
            "commit2",
            Some("a\nB\nc\n"),
        );
        let commit3 = create_commit(
            &test_repo,
            mut_repo,
            &[&commit2],
            "commit3",
            Some("a\nB\nc\nd\n"),
        );
        let repo: &dyn Repo = mut_repo;

        let mut state = new_state(repo, &commit3)?;
        apply(&mut state, repo, &[Action::Down]);
        assert_eq!(selected_commit_id(&state), commit2.id());

        // Annotate the parent of commit2, i.e. commit1
        apply(&mut state, repo, &[Action::AnnotateParent]);
        insta::assert_snapshot!(render_to_string(&mut state, 40, 7), @"
        file at commit1 [depth 1]
          commit1: a
        ▶ commit1: b
          commit1: c

        commit1
        ↓/j down • ↑/k up • ]/[ next/prev hunk •
        ");
        assert_eq!(state.current().selected, 1);
        assert_eq!(selected_commit_id(&state), commit1.id());

        // The file doesn't exist in the parent of commit1
        apply(&mut state, repo, &[Action::AnnotateParent]);
        insta::assert_snapshot!(render_to_string(&mut state, 40, 7), @"
        file at commit1 [depth 1]
          commit1: a
        ▶ commit1: b
          commit1: c

        file does not exist in parent of sryyqqk
        ↓/j down • ↑/k up • ]/[ next/prev hunk •
        ");
        assert_eq!(state.history.len(), 2);

        // Go back to the original annotation with the selection preserved
        apply(&mut state, repo, &[Action::Back]);
        assert_eq!(state.history.len(), 1);
        assert_eq!(state.current().selected, 1);
        apply(&mut state, repo, &[Action::Back]);
        insta::assert_snapshot!(render_to_string(&mut state, 40, 7), @"
        file at commit3
          commit1: a
        ▶ commit2: B
          commit1: c
          commit3: d
        Already at the first annotation
        ↓/j down • ↑/k up • ]/[ next/prev hunk •
        ");
        Ok(())
    }

    #[test]
    fn test_annotate_parent_maps_selected_line() -> TestResult {
        let test_repo = TestRepo::init();
        let mut tx = test_repo.repo.start_transaction();
        let mut_repo = tx.repo_mut();
        let commit1 = create_commit(&test_repo, mut_repo, &[], "commit1", Some("a\nb\nc\n"));
        let commit2 = create_commit(
            &test_repo,
            mut_repo,
            &[&commit1],
            "commit2",
            Some("x\ny\na\nB\nc\n"),
        );
        let repo: &dyn Repo = mut_repo;

        // The parent's line corresponding to "B" is selected, not the line with
        // the same number
        let mut state = new_state(repo, &commit2)?;
        apply(
            &mut state,
            repo,
            &[Action::Down, Action::Down, Action::Down],
        );
        apply(&mut state, repo, &[Action::AnnotateParent]);
        insta::assert_snapshot!(render_to_string(&mut state, 40, 6), @"
        file at commit1 [depth 1]
          commit1: a
        ▶ commit1: b
          commit1: c
        commit1
        ↓/j down • ↑/k up • ]/[ next/prev hunk •
        ");
        Ok(())
    }

    #[test]
    fn test_show_commit() -> TestResult {
        let test_repo = TestRepo::init();
        let mut tx = test_repo.repo.start_transaction();
        let mut_repo = tx.repo_mut();
        let commit1 = create_commit(&test_repo, mut_repo, &[], "commit1", Some("a\nb\n"));
        let commit2 = create_commit(&test_repo, mut_repo, &[&commit1], "commit2", Some("a\nB\n"));
        let repo: &dyn Repo = mut_repo;

        // The commit is rendered for the width of the viewport
        let mut state = new_state(repo, &commit2)?;
        render_to_string(&mut state, 6, 7);
        apply(&mut state, repo, &[Action::Down, Action::ShowCommit]);
        insta::assert_snapshot!(render_to_string(&mut state, 40, 7), @"
        file at commit2
        commit2
        diff line 1
        diff line 2
        diff line 3
        Lines 1-4 of 7
        ↓/j down • ↑/k up • space page down • p
        ");

        // Scrolling stops at the last page
        apply(&mut state, repo, &[Action::PageDown]);
        insta::assert_snapshot!(render_to_string(&mut state, 40, 7), @"
        file at commit2
        diff line 3
        diff line 4
        diff line 5
        diff line 6
        Lines 4-7 of 7
        ↓/j down • ↑/k up • space page down • p
        ");
        apply(&mut state, repo, &[Action::PageDown]);
        assert_eq!(state.commit_view.as_ref().unwrap().scroll_top, 3);
        apply(&mut state, repo, &[Action::Up, Action::First]);
        assert_eq!(state.commit_view.as_ref().unwrap().scroll_top, 0);
        // Growing the viewport doesn't leave empty lines at the end
        apply(&mut state, repo, &[Action::Last]);
        render_to_string(&mut state, 40, 12);
        assert_eq!(state.commit_view.as_ref().unwrap().scroll_top, 0);

        // Closing returns to the annotation with the selection preserved
        apply(&mut state, repo, &[Action::Close]);
        assert!(state.commit_view.is_none());
        assert_eq!(state.current().selected, 1);

        // The parent can be annotated directly from the commit view
        apply(
            &mut state,
            repo,
            &[Action::ShowCommit, Action::AnnotateParent],
        );
        assert!(state.commit_view.is_none());
        assert_eq!(state.history.len(), 2);
        assert_eq!(selected_commit_id(&state), commit1.id());
        Ok(())
    }

    #[test]
    fn test_annotate_parent_of_root() -> TestResult {
        let test_repo = TestRepo::init();
        let mut tx = test_repo.repo.start_transaction();
        let mut_repo = tx.repo_mut();
        let commit1 = create_commit(&test_repo, mut_repo, &[], "commit1", Some("a\n"));
        let repo: &dyn Repo = mut_repo;

        // Lines outside the annotation's domain are attributed to the root
        // commit
        let mut state = new_state(repo, &commit1)?;
        state.current_mut().lines[0].commit = repo.store().root_commit();
        apply(&mut state, repo, &[Action::AnnotateParent]);
        assert_eq!(state.history.len(), 1);
        assert_eq!(
            state.message.as_deref(),
            Some("Commit zzzzzzzzzzzz has no parents")
        );
        Ok(())
    }

    #[test]
    fn test_annotate_parent_of_merge() -> TestResult {
        let test_repo = TestRepo::init();
        let mut tx = test_repo.repo.start_transaction();
        let mut_repo = tx.repo_mut();
        let base = create_commit(&test_repo, mut_repo, &[], "base", Some("a\nx\nb\n"));
        let left = create_commit(&test_repo, mut_repo, &[&base], "left", Some("A\nx\nb\n"));
        let right = create_commit(&test_repo, mut_repo, &[&base], "right", Some("a\nx\nB\n"));
        let merge = create_commit(
            &test_repo,
            mut_repo,
            &[&left, &right],
            "merge",
            Some("A\nx\nB\nm\n"),
        );
        let repo: &dyn Repo = mut_repo;

        let mut state = new_state(repo, &merge)?;
        apply(&mut state, repo, &[Action::Last]);
        assert_eq!(selected_commit_id(&state), merge.id());
        apply(&mut state, repo, &[Action::AnnotateParent]);
        insta::assert_snapshot!(render_to_string(&mut state, 40, 7), @"
        file at parents of merge [depth 1]
          left: A
          base: x
        ▶ right: B

        right
        ↓/j down • ↑/k up • ]/[ next/prev hunk •
        ");
        Ok(())
    }

    #[test]
    fn test_map_line_to_left() {
        let left = "a\nb\nc\nd\ne\n";
        let right = "a\nB1\nB2\nB3\nc\nD\ne\nf";
        let mapped = (0..9)
            .map(|line| map_line_to_left(left.as_bytes(), right.as_bytes(), line))
            .collect_vec();
        // B1-B3 replace b, D replaces d, f is added
        assert_eq!(mapped, [0, 1, 1, 1, 2, 3, 4, 5, 5]);

        // Lines in a hunk of the same size map to the same offset
        assert_eq!(map_line_to_left(b"a\nb\nc\n", b"A\nB\nC\n", 1), 1);
        // Added lines map to the next line
        assert_eq!(map_line_to_left(b"a\nc\n", b"a\nb\nc\n", 1), 1);
        assert_eq!(map_line_to_left(b"", b"a\n", 0), 0);
    }

    #[test]
    fn test_render_tabs_and_long_lines() -> TestResult {
        let test_repo = TestRepo::init();
        let mut tx = test_repo.repo.start_transaction();
        let mut_repo = tx.repo_mut();
        let commit1 = create_commit(
            &test_repo,
            mut_repo,
            &[],
            "commit1",
            Some("\tindented\nthis line is too long to fit in the viewport\n"),
        );
        let repo: &dyn Repo = mut_repo;

        let mut state = new_state(repo, &commit1)?;
        insta::assert_snapshot!(render_to_string(&mut state, 40, 5), @"
        file at commit1
        ▶ commit1:        indented
          commit1: this line is too long to fit
        commit1
        ↓/j down • ↑/k up • ]/[ next/prev hunk •
        ");
        Ok(())
    }
}
