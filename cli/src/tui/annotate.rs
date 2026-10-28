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

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use jj_lib::annotate::FileAnnotator;
use jj_lib::annotate::get_file_contents;
use jj_lib::backend::MergedTreeValueExt as _;
use jj_lib::commit::Commit;
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
    Back,
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
        (KeyCode::Backspace | KeyCode::Char('b'), KeyModifiers::NONE) => Action::Back,
        (KeyCode::Esc | KeyCode::Char('q'), KeyModifiers::NONE)
        | (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::Quit,
        _ => return None,
    };
    Some(action)
}

const HELP_ITEMS: &[(&str, &str)] = &[
    ("↓/j", "down"),
    ("↑/k", "up"),
    ("]/[", "next/prev hunk"),
    ("p", "annotate parent"),
    ("b", "back"),
    ("q", "quit"),
];

#[derive(Clone, Debug)]
pub(crate) struct State {
    /// Annotations the user has navigated through. The last one is shown.
    /// Never empty.
    history: Vec<AnnotatedFile>,
    /// Message to show instead of the selected line's commit until the next
    /// key press
    message: Option<String>,
    path_converter: RepoPathUiConverter,
    /// Number of lines that fit in the viewport the last time it was rendered
    page_size: usize,
}

impl State {
    pub fn new(file: AnnotatedFile, path_converter: RepoPathUiConverter) -> Self {
        Self {
            history: vec![file],
            message: None,
            path_converter,
            page_size: 1,
        }
    }

    fn current(&self) -> &AnnotatedFile {
        self.history.last().unwrap()
    }

    fn current_mut(&mut self) -> &mut AnnotatedFile {
        self.history.last_mut().unwrap()
    }

    fn clamp_scroll(&mut self, viewport_rows: u16) {
        self.page_size = usize::from(viewport_rows).max(1);
        let page_size = self.page_size;
        let file = self.current_mut();
        file.scroll_top =
            tui::scroll_to_selection(file.scroll_top, file.selected, page_size, file.lines.len());
    }

    async fn apply(&mut self, repo: &dyn Repo, action: Action) -> Result<(), CommandError> {
        self.message = None;
        let page_size = isize::try_from(self.page_size).unwrap_or(isize::MAX);
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
            Action::Back => {
                if self.history.len() > 1 {
                    self.history.pop();
                } else {
                    self.message = Some("Already at the first annotation".to_owned());
                }
            }
            Action::Quit => {}
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

        // Annotating the parent content as if it were in `commit` attributes
        // every line to the parents. That also works when the commit has
        // multiple parents.
        let mut annotator = FileAnnotator::with_file_content(commit.id(), &path, parent_text);
        annotator.compute(repo, &RevsetExpression::all()).await?;
        let lines = AnnotationLine::from_annotation(repo, &annotator.to_annotation()).await?;
        let revision = if let [parent_id] = commit.parent_ids() {
            AnnotatedRevision::Commit(repo.store().get_commit_async(parent_id).await?)
        } else {
            AnnotatedRevision::MergedParents(commit)
        };
        let mut file = AnnotatedFile::new(revision, path, lines);
        file.select(line.original_line_number - 1);
        self.history.push(file);
        Ok(())
    }
}

/// Runs the event loop until the user quits.
pub(crate) async fn run(
    terminal: &mut TuiTerminal,
    repo: &dyn Repo,
    mut state: State,
    render_line: impl Fn(&AnnotationLine) -> Vec<u8>,
    render_commit: impl Fn(&Commit) -> Vec<u8>,
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
            Some(action) => state.apply(repo, action).await?,
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
    state.clamp_scroll(main_area.height);
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
        for &action in actions {
            state.apply(repo, action).block_on().unwrap();
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
