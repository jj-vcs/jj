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

use std::collections::HashMap;
use std::collections::HashSet;
use std::io;

use bstr::ByteSlice as _;
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
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::future::try_join_all;
use indexmap::IndexSet;
use itertools::Itertools as _;
use jj_lib::backend::BackendResult;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::dag_walk;
use jj_lib::repo::MutableRepo;
use jj_lib::repo::Repo as _;
use jj_lib::revset::RevsetStreamExt as _;
use jj_lib::rewrite::CommitRewriter;
use ratatui::Terminal;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::layout::Offset;
use ratatui::layout::Rect;
use ratatui::prelude::CrosstermBackend;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use renderdag::Ancestor;
use renderdag::GraphRowRenderer;
use renderdag::Renderer as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::short_change_hash;
use crate::cli_util::short_commit_hash;
use crate::command_error::CommandError;
use crate::command_error::internal_error;
use crate::command_error::user_error;
use crate::complete;
use crate::formatter::FormatterExt as _;
use crate::templater::TemplateRenderer;
use crate::ui::Ui;

/// Interactively arrange the commit graph.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct ArrangeArgs {
    /// The revisions to arrange [aliases: -r]
    ///
    /// If no revisions are specified, this defaults to the `revsets.arrange`
    /// setting.
    #[arg(value_name = "REVSETS")]
    #[arg(add = clap_complete::ArgValueCompleter::new(complete::revset_expression_mutable))]
    revisions_pos: Vec<RevisionArg>,

    #[arg(short = 'r', hide = true, value_name = "REVSETS")]
    #[arg(add = clap_complete::ArgValueCompleter::new(complete::revset_expression_mutable))]
    revisions_opt: Vec<RevisionArg>,
}

#[instrument(skip_all)]
pub(crate) async fn cmd_arrange(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &ArrangeArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo().clone();
    let target_expression = if args.revisions_pos.is_empty() && args.revisions_opt.is_empty() {
        let revs = workspace_command.settings().get_string("revsets.arrange")?;
        workspace_command.parse_revset(ui, &RevisionArg::from(revs))?
    } else {
        workspace_command
            .parse_union_revsets(ui, &[&*args.revisions_pos, &*args.revisions_opt].concat())?
    }
    .resolve()?;
    workspace_command
        .check_rewritable_expr(&target_expression)
        .await?;

    let gaps_revset = target_expression
        .connected()
        .minus(&target_expression)
        .evaluate(repo.as_ref())?;
    if let Some(commit_id) = gaps_revset.stream().next().await {
        return Err(
            user_error("Cannot arrange revset with gaps in.").hinted(format!(
                "Revision {} would need to be in the set.",
                short_commit_hash(&commit_id?)
            )),
        );
    }

    let children_revset = target_expression
        .children()
        .minus(&target_expression)
        .evaluate(repo.as_ref())?;
    let external_children: Vec<_> = children_revset
        .stream()
        .commits(repo.store())
        .try_collect()
        .await?;

    let revset = target_expression.evaluate(repo.as_ref())?;
    let commits: Vec<Commit> = revset.stream().commits(repo.store()).try_collect().await?;
    if commits.is_empty() {
        writeln!(ui.status(), "No revisions to arrange.")?;
        return Ok(());
    }

    // Set up the terminal
    io::stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;

    let mut state = State::new(repo.as_ref(), commits, external_children).await?;
    state.update_commit_order();

    let template_string = workspace_command
        .settings()
        .get_string("templates.arrange")?;
    let template = workspace_command
        .parse_commit_template(ui, &template_string)?
        .labeled(["commit"]);

    let result = run_tui(ui, &mut terminal, template, state);

    // Restore the terminal
    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

    if let Some(new_state) = result? {
        let mut tx = workspace_command.start_transaction();
        let rewrites = new_state.to_rewrite_plan();
        rewrites.execute(tx.repo_mut()).await?;
        tx.finish(ui, "arrange revisions").await?;
        Ok(())
    } else {
        Err(user_error("Canceled by user"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum UiAction {
    Abandon,
    Keep,
}

/// The state of a single commit in the UI
#[derive(Clone, Debug, PartialEq, Eq)]
struct CommitState {
    commit: Commit,
    action: UiAction,
    parents: Vec<CommitId>,
    bookmarks: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RebasePopupState {
    filter_text: String,
    selected_index: usize,
    candidates: Vec<CommitId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct State {
    /// Commits in the target set, as well as any external children and parents.
    commits: HashMap<CommitId, CommitState>,
    /// Heads of the target set in the order they should be added to the UI.
    /// This is used to make the graph rendering more stable. It must be
    /// kept up to date when parents are changed.
    head_order: Vec<CommitId>,
    /// The current order of commits target commits in the UI. This is
    /// recalculated when necessary from `head_order`.
    current_order: Vec<CommitId>,
    /// The current selection as an index into `current_order`
    current_selection: usize,
    external_children: IndexSet<CommitId>,
    external_parents: IndexSet<CommitId>,
    rebase_popup: Option<RebasePopupState>,
}

impl State {
    /// Creates a new `State` from a list of commits and a list of external
    /// children. The list of commits must not have gaps between commits.
    async fn new(
        repo: &dyn jj_lib::repo::Repo,
        commits: Vec<Commit>,
        external_children: Vec<Commit>,
    ) -> BackendResult<Self> {
        // Initialize head_order to match the heads in the input's order.
        let commit_set: HashSet<_> = commits.iter().map(|commit| commit.id()).collect();
        let mut heads: HashSet<_> = commit_set.clone();
        for commit in &commits {
            for parent in commit.parent_ids() {
                heads.remove(parent);
            }
        }
        let mut external_parents = IndexSet::new();
        for commit in &commits {
            for parent_id in commit.parent_ids() {
                if !commit_set.contains(parent_id) {
                    external_parents.insert(parent_id.clone());
                }
            }
        }
        let external_parent_commits: Vec<_> = try_join_all(
            external_parents
                .iter()
                .map(|id| commits[0].store().get_commit_async(id)),
        )
        .await?;
        let head_order = commits
            .iter()
            .filter(|&commit| heads.contains(commit.id()))
            .map(|commit| commit.id().clone())
            .collect();
        let external_children_ids = external_children
            .iter()
            .map(|commit| commit.id().clone())
            .collect();
        let commits: HashMap<CommitId, CommitState> = commits
            .into_iter()
            .chain(external_children)
            .chain(external_parent_commits)
            .map(|commit: Commit| {
                let id = commit.id().clone();
                let parents = commit.parent_ids().to_vec();
                let bookmarks = repo
                    .view()
                    .local_bookmarks_for_commit(&id)
                    .map(|(name, _)| name.as_str().to_string())
                    .collect();
                let commit_state = CommitState {
                    commit,
                    action: UiAction::Keep,
                    parents,
                    bookmarks,
                };
                (id, commit_state)
            })
            .collect();
        let mut state = Self {
            commits,
            head_order,
            current_order: vec![], // Will be set by update_commit_order()
            current_selection: 0,
            external_children: external_children_ids,
            external_parents,
            rebase_popup: None,
        };
        state.update_commit_order();
        Ok(state)
    }

    fn is_valid(&self) -> bool {
        true
    }

    fn current_id(&self) -> &CommitId {
        &self.current_order[self.current_selection]
    }

    /// Update `head_order` to reflect changes in parents, ensuring all components
    /// of the graph remain reachable during traversal.
    fn update_heads(&mut self) {
        let target_commits: HashSet<_> = self.current_order.iter().cloned().collect();
        let mut children = HashSet::new();
        for id in &target_commits {
            let commit_state = self.commits.get(id).unwrap();
            for parent in &commit_state.parents {
                if target_commits.contains(parent) {
                    children.insert(parent.clone());
                }
            }
        }
        
        let mut heads = Vec::new();
        // Preserve order from head_order if they are still heads
        for id in &self.head_order {
            if target_commits.contains(id) && !children.contains(id) {
                heads.push(id.clone());
            }
        }
        // Add any new heads in current_order
        for id in &self.current_order {
            if !children.contains(id) && !heads.contains(id) {
                heads.push(id.clone());
            }
        }
        self.head_order = heads;
    }

    fn get_descendants(&self, commit_id: &CommitId) -> HashSet<CommitId> {
        let mut descendants = HashSet::new();
        let mut children_map: HashMap<CommitId, Vec<CommitId>> = HashMap::new();
        for (id, state) in &self.commits {
            for parent in &state.parents {
                children_map.entry(parent.clone()).or_default().push(id.clone());
            }
        }
        
        let mut visited = HashSet::new();
        let mut stack = vec![commit_id.clone()];
        
        while let Some(id) = stack.pop() {
            if visited.contains(&id) {
                continue;
            }
            visited.insert(id.clone());
            if &id != commit_id {
                descendants.insert(id.clone());
            }
            if let Some(children) = children_map.get(&id) {
                for child in children {
                    stack.push(child.clone());
                }
            }
        }
        
        descendants
    }

    fn get_filtered_candidates(&self, current_id: &CommitId, filter_text: &str) -> Vec<CommitId> {
        let descendants = self.get_descendants(current_id);
        let all_candidates: Vec<CommitId> = self.current_order.iter()
            .chain(self.external_parents.iter())
            .filter(|id| **id != *current_id && !descendants.contains(*id))
            .cloned()
            .collect();
        
        if filter_text.is_empty() {
            return all_candidates;
        }
        
        let filter_lower = filter_text.to_lowercase();
        all_candidates.into_iter().filter(|id| {
            let commit_state = self.commits.get(id).unwrap();
            let commit = &commit_state.commit;
            let change_hash = short_change_hash(commit.change_id()).to_lowercase();
            let desc = commit.description().to_lowercase();
            let matches_bookmark = commit_state.bookmarks.iter().any(|b| b.to_lowercase().contains(&filter_lower));
            change_hash.contains(&filter_lower) || desc.contains(&filter_lower) || matches_bookmark
        }).collect()
    }

    /// Update the current UI commit order after parents have changed.
    fn update_commit_order(&mut self) {
        // Use the original order to get a determinisic order.
        // TODO: Use TopoGroupedGraph so the order better matches `jj log`
        let commit_ids: Vec<&CommitId> = dag_walk::topo_order_reverse(
            self.head_order.iter(),
            |id| *id,
            |id| {
                self.commits.get(id).unwrap().parents.iter().filter(|id| {
                    self.commits.contains_key(id) && !self.external_parents.contains(*id)
                })
            },
            |_| panic!("cycle detected"),
        )
        .unwrap();
        self.current_order = commit_ids.into_iter().cloned().collect();
    }

    fn swap_commits(&mut self, a_id: &CommitId, b_id: &CommitId) {
        if a_id == b_id {
            return;
        }

        let a_idx = self.current_order.iter().position(|x| x == a_id).unwrap();
        let b_idx = self.current_order.iter().position(|x| x == b_id).unwrap();

        if self.current_selection == a_idx {
            self.current_selection = b_idx;
        } else if self.current_selection == b_idx {
            self.current_selection = a_idx;
        }

        self.current_order.swap(a_idx, b_idx);

        for id in &mut self.head_order {
            if id == a_id {
                *id = b_id.clone();
            } else if id == b_id {
                *id = a_id.clone();
            }
        }

        // Update references to the swapped commits from their children
        for commit_state in self.commits.values_mut() {
            for id in &mut commit_state.parents {
                if id == a_id {
                    *id = b_id.clone();
                } else if id == b_id {
                    *id = a_id.clone();
                }
            }
        }

        // Swap the parents of the swapped commits
        let [a_state, b_state] = self
            .commits
            .get_disjoint_mut([a_id, b_id])
            .map(Option::unwrap);
        std::mem::swap(&mut a_state.parents, &mut b_state.parents);
    }

    fn to_rewrite_plan(&self) -> RewritePlan {
        let mut rewrites = HashMap::new();
        for (id, commit_state) in &self.commits {
            if self.external_parents.contains(id) {
                continue;
            }
            rewrites.insert(
                id.clone(),
                Rewrite {
                    old_commit: commit_state.commit.clone(),
                    new_parents: commit_state.parents.clone(),
                    action: match commit_state.action {
                        UiAction::Abandon => RewriteAction::Abandon,
                        UiAction::Keep => RewriteAction::Keep,
                    },
                },
            );
        }
        RewritePlan { rewrites }
    }

    /// Swap the selected commit with its parent. Does nothing if there is not
    /// exactly one parent or if the parent is an external parent.
    fn swap_selection_down(&mut self) {
        let current_id = self.current_id().clone();
        let [parent] = self.commits.get(&current_id).unwrap().parents.as_slice() else {
            return;
        };
        if self.external_parents.contains(parent) {
            return;
        }
        let parent = parent.clone();
        self.swap_commits(&current_id, &parent);
    }

    /// Swap the selected commit with one of its children. Does nothing if there
    /// is not exactly one child or if the child is an external child.
    fn swap_selection_up(&mut self) {
        let current_id = self.current_id().clone();
        let children: Vec<_> = self
            .commits
            .iter()
            .filter(|(_, state)| state.parents.contains(&current_id))
            .map(|(id, _)| id.clone())
            .collect();
        let [child] = children.as_slice() else {
            return;
        };
        if self.external_children.contains(child) {
            return;
        }
        self.swap_commits(&current_id, child);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum RewriteAction {
    Abandon,
    Keep,
}

struct Rewrite {
    old_commit: Commit,
    new_parents: Vec<CommitId>,
    action: RewriteAction,
}

struct RewritePlan {
    rewrites: HashMap<CommitId, Rewrite>,
}

impl RewritePlan {
    async fn execute(
        mut self,
        mut_repo: &mut MutableRepo,
    ) -> Result<HashMap<CommitId, Commit>, CommandError> {
        // Find order to rebase the commits. The order is determined by the new
        // parents.
        let ordered_commit_ids = dag_walk::topo_order_forward(
            self.rewrites.keys().cloned(),
            |id| id.clone(),
            |id| {
                self.rewrites
                    .get(id)
                    .unwrap()
                    .new_parents
                    .iter()
                    .filter(|id| self.rewrites.contains_key(id))
                    .cloned()
            },
            |_| panic!("cycle detected"),
        )
        .unwrap();
        // Rewrite the commits in the order determined above
        let mut rewritten_commits: HashMap<CommitId, Commit> = HashMap::new();
        for id in ordered_commit_ids {
            let rewrite = self.rewrites.remove(&id).unwrap();
            let new_parents = mut_repo.new_parents(&rewrite.new_parents);
            let rewriter = CommitRewriter::new(mut_repo, rewrite.old_commit, new_parents);
            match rewrite.action {
                RewriteAction::Abandon => rewriter.abandon(),
                RewriteAction::Keep => {
                    if rewriter.parents_changed() {
                        let new_commit = rewriter.rebase().await?.write().await?;
                        rewritten_commits.insert(id, new_commit);
                    }
                }
            }
        }
        Ok(rewritten_commits)
    }
}

fn run_tui<B: ratatui::backend::Backend>(
    ui: &mut Ui,
    terminal: &mut Terminal<B>,
    template: TemplateRenderer<Commit>,
    mut state: State,
) -> Result<Option<State>, CommandError> {
    let help_items = [
        ("↓/j", "down"),
        ("↑/k", "up"),
        ("⇧+↓/J", "swap down"),
        ("⇧+↑/K", "swap up"),
        ("a", "abandon"),
        ("p", "keep"),
        ("r", "rebase"),
        ("c", "confirm"),
        ("q", "quit"),
    ];
    let mut help_spans = Vec::new();
    for (i, (key, desc)) in help_items.iter().enumerate() {
        if i > 0 {
            help_spans.push(Span::raw(" • "));
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
                let main_area = layout[0];
                let help_area = layout[1];
                render(&state, ui, &template, frame, main_area);
                frame.render_widget(&help_line, help_area);
            })
            .map_err(|e| internal_error(format!("Failed to draw TUI: {e}")))?;

        if let Event::Key(event) =
            event::read().map_err(|e| internal_error(format!("Failed to read TUI events: {e}")))?
        {
            // On Windows, we get Press and Release (and maybe Repeat) events, but on Linux
            // we only get Press.
            if event.is_release() {
                continue;
            }
            if state.rebase_popup.is_none() {
                match (event.code, event.modifiers) {
                    (KeyCode::Char('q'), KeyModifiers::NONE)
                    | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        return Ok(None);
                    }
                    (KeyCode::Char('c'), KeyModifiers::NONE) => {
                        return Ok(Some(state));
                    }
                    _ => {}
                }
            }
            let new_state = handle_key_event(event, state.clone());
            if new_state != state && new_state.is_valid() {
                state = new_state;
                state.update_commit_order();
            }
        }
    }
}

fn handle_key_event(event: KeyEvent, mut state: State) -> State {
    if let Some(mut popup_state) = state.rebase_popup.take() {
        match (event.code, event.modifiers) {
            (KeyCode::Esc, KeyModifiers::NONE) => {
                state.rebase_popup = None;
            }
            (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE) => {
                if popup_state.selected_index + 1 < popup_state.candidates.len() {
                    popup_state.selected_index += 1;
                }
                state.rebase_popup = Some(popup_state);
            }
            (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE) => {
                if popup_state.selected_index > 0 {
                    popup_state.selected_index -= 1;
                }
                state.rebase_popup = Some(popup_state);
            }
            (KeyCode::Enter, KeyModifiers::NONE) => {
                if let Some(target_id) = popup_state.candidates.get(popup_state.selected_index) {
                    let current_id = state.current_id().clone();
                    state.commits.get_mut(&current_id).unwrap().parents = vec![target_id.clone()];
                    state.update_heads();
                    state.update_commit_order();
                }
                state.rebase_popup = None;
            }
            (KeyCode::Backspace, KeyModifiers::NONE) => {
                popup_state.filter_text.pop();
                popup_state.selected_index = 0;
                let current_id = state.current_id().clone();
                popup_state.candidates = state.get_filtered_candidates(&current_id, &popup_state.filter_text);
                state.rebase_popup = Some(popup_state);
            }
            (KeyCode::Char(c), KeyModifiers::NONE) => {
                popup_state.filter_text.push(c);
                popup_state.selected_index = 0;
                let current_id = state.current_id().clone();
                popup_state.candidates = state.get_filtered_candidates(&current_id, &popup_state.filter_text);
                state.rebase_popup = Some(popup_state);
            }
            _ => {
                state.rebase_popup = Some(popup_state);
            }
        }
    } else {
        match (event.code, event.modifiers) {
            (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE)
                if state.current_selection + 1 < state.current_order.len() =>
            {
                state.current_selection += 1;
            }
            (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE) if state.current_selection > 0 => {
                state.current_selection -= 1;
            }
            (KeyCode::Char('a'), KeyModifiers::NONE) => {
                let id = state.current_id().clone();
                state.commits.get_mut(&id).unwrap().action = UiAction::Abandon;
            }
            (KeyCode::Char('p'), KeyModifiers::NONE) => {
                let id = state.current_id().clone();
                state.commits.get_mut(&id).unwrap().action = UiAction::Keep;
            }
            (KeyCode::Down | KeyCode::Char('J'), KeyModifiers::SHIFT) => {
                state.swap_selection_down();
            }
            (KeyCode::Up | KeyCode::Char('K'), KeyModifiers::SHIFT) => {
                state.swap_selection_up();
            }
            (KeyCode::Char('r'), KeyModifiers::NONE) => {
                let current_id = state.current_id().clone();
                let commit_state = state.commits.get(&current_id).unwrap();
                if commit_state.parents.len() == 1 {
                    let candidates = state.get_filtered_candidates(&current_id, "");
                    state.rebase_popup = Some(RebasePopupState {
                        filter_text: String::new(),
                        selected_index: 0,
                        candidates,
                    });
                }
            }
            _ => {}
        }
    }
    state
}

fn render(
    state: &State,
    ui: &mut Ui,
    template: &crate::templater::TemplateRenderer<Commit>,
    frame: &mut ratatui::Frame,
    main_area: Rect,
) {
    let mut row_renderer = GraphRowRenderer::new()
        .output()
        .with_min_row_height(2)
        .build_box_drawing();
    let mut row_area = main_area;
    let current_seletion_id = state.current_id();
    let commits_to_render = state
        .external_children
        .iter()
        .chain(state.current_order.iter())
        .chain(state.external_parents.iter());
    for id in commits_to_render {
        // TODO: Make the graph column width depend on what's needed to render the
        // graph.
        let row_layout = Layout::horizontal([
            Constraint::Min(2),
            Constraint::Min(10),
            Constraint::Min(10),
            Constraint::Fill(100),
        ])
        .split(row_area);
        let selection_area = row_layout[0];
        let graph_area = row_layout[1];
        let action_area = row_layout[2];
        let text_area = row_layout[3];

        if id == current_seletion_id {
            frame.render_widget(Text::from("▶"), selection_area);
        }

        let commit_state = state.commits.get(id).unwrap();
        let action = &commit_state.action;

        // TODO: The graph can be misaligned with the text because sometimes `renderdag`
        // inserts a line of edges before the line with the node and we assume the node
        // is the first line emitted.
        let edges = commit_state
            .parents
            .iter()
            .map(|parent| {
                if state.commits.contains_key(parent) {
                    Ancestor::Parent(parent)
                } else {
                    Ancestor::Anonymous
                }
            })
            .collect_vec();
        let glyph = match action {
            UiAction::Abandon => "×",
            UiAction::Keep => "○",
        };

        let is_context_node =
            state.external_children.contains(id) || state.external_parents.contains(id);
        if !is_context_node {
            let action_text = match action {
                UiAction::Abandon => "abandon",
                UiAction::Keep => "keep",
            };
            frame.render_widget(Text::from(action_text), action_area);
        }

        let mut text_lines = vec![];
        let mut formatter = ui.new_formatter(&mut text_lines).into_labeled("arrange");
        if is_context_node {
            template
                .format(&commit_state.commit, formatter.labeled("context").as_mut())
                .unwrap();
        } else {
            template
                .format(&commit_state.commit, formatter.as_mut())
                .unwrap();
        }
        drop(formatter);
        let text = ansi_to_tui::IntoText::into_text(&text_lines).unwrap();
        frame.render_widget(text, text_area);

        // Make graph as tall as the text
        let graph_message = "\n".repeat(text_lines.lines().count());
        let graph_lines = row_renderer.next_row(id, edges, glyph.to_string(), graph_message);
        let graph_text = Text::from(graph_lines);
        row_area = row_area
            .offset(Offset {
                x: 0,
                y: graph_text.height() as i32,
            })
            .intersection(main_area);
        frame.render_widget(graph_text, graph_area);
    }

    if let Some(ref popup) = state.rebase_popup {
        let area = frame.area();
        let popup_area = Rect::new(area.width / 4, area.height / 4, area.width / 2, area.height / 2);
        
        frame.render_widget(Clear, popup_area);
        
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(" Rebase onto ", Style::default().fg(Color::Green)));
        
        let inner_area = block.inner(popup_area);
        frame.render_widget(block, popup_area);
        
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(1)])
            .split(inner_area);
        
        let input = Paragraph::new(popup.filter_text.as_str())
            .block(Block::default().borders(Borders::ALL).title("Filter"));
        
        frame.render_widget(input, layout[0]);
        
        let current_id = state.current_id();
        let items: Vec<ListItem> = state.get_filtered_candidates(current_id, &popup.filter_text).iter().enumerate().map(|(i, id)| {
            let commit_state = state.commits.get(id).unwrap();
            let hash = short_change_hash(commit_state.commit.change_id());
            let desc = commit_state.commit.description().lines().next().unwrap_or("");
            let bookmarks_str = if commit_state.bookmarks.is_empty() {
                String::new()
            } else {
                format!(" ({})", commit_state.bookmarks.join(", "))
            };
            let content = format!("{} {}{}", hash, desc, bookmarks_str);
            let style = if i == popup.selected_index {
                Style::default().bg(Color::Blue).fg(Color::White)
            } else {
                Style::default()
            };
            ListItem::new(content).style(style)
        }).collect();
        
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("Candidates"));
        
        let mut list_state = ratatui::widgets::ListState::default();
        list_state.select(Some(popup.selected_index));
        
        frame.render_stateful_widget(list, layout[1], &mut list_state);
    }
}

#[cfg(test)]
mod tests {
    use maplit::hashset;
    use pollster::FutureExt as _;
    use testutils::CommitBuilderExt as _;
    use testutils::TestRepo;
    use testutils::TestResult;

    use super::*;

    fn no_op_plan(commits: &[&Commit]) -> RewritePlan {
        let rewrites = commits
            .iter()
            .map(|commit| {
                (
                    commit.id().clone(),
                    Rewrite {
                        old_commit: (*commit).clone(),
                        new_parents: commit.parent_ids().to_vec(),
                        action: RewriteAction::Keep,
                    },
                )
            })
            .collect();
        RewritePlan { rewrites }
    }

    #[test]
    fn test_update_commit_order_empty() -> TestResult {
        let test_repo = TestRepo::init();
        let mut state = State::new(test_repo.repo.as_ref(), vec![], vec![]).block_on()?;
        assert_eq!(state.head_order, vec![]);
        state.update_commit_order();
        assert_eq!(state.current_order, vec![]);
        Ok(())
    }

    #[test]
    fn test_update_commit_order_reorder() -> TestResult {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();
        let empty_tree = store.empty_merged_tree();

        // Move A on top of C:
        // D C          A
        // |/           |
        // B     =>     C D
        // |            |/
        // A            B
        let mut tx = test_repo.repo.start_transaction();
        let mut create_commit = |parents| {
            tx.repo_mut()
                .new_commit(parents, empty_tree.clone())
                .write_unwrap()
        };
        let commit_a = create_commit(vec![store.root_commit_id().clone()]);
        let commit_b = create_commit(vec![commit_a.id().clone()]);
        let commit_c = create_commit(vec![commit_b.id().clone()]);
        let commit_d = create_commit(vec![commit_b.id().clone()]);

        let mut state = State::new(
            test_repo.repo.as_ref(),
            vec![
                commit_d.clone(),
                commit_c.clone(),
                commit_b.clone(),
                commit_a.clone(),
            ],
            vec![],
        )
        .block_on()?;

        // The initial head order is determined by the input order
        assert_eq!(
            state.head_order,
            vec![commit_d.id().clone(), commit_c.id().clone()]
        );

        // We get the original order before we make any changes
        assert_eq!(
            state.current_order,
            vec![
                commit_d.id().clone(),
                commit_c.id().clone(),
                commit_b.id().clone(),
                commit_a.id().clone(),
            ]
        );

        // Update parents and head order and check that the commit order changes.
        state.commits.get_mut(commit_a.id()).unwrap().parents = vec![commit_c.id().clone()];
        state.commits.get_mut(commit_b.id()).unwrap().parents =
            vec![store.root_commit_id().clone()];
        state.head_order = vec![commit_d.id().clone(), commit_a.id().clone()];
        state.update_commit_order();
        assert_eq!(
            state.current_order,
            vec![
                commit_d.id().clone(),
                commit_a.id().clone(),
                commit_c.id().clone(),
                commit_b.id().clone(),
            ]
        );

        Ok(())
    }

    #[test]
    fn test_swap_commits() -> TestResult {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();
        let empty_tree = store.empty_merged_tree();

        // Construct the graph:
        // f
        // |
        // D e
        // |\|
        // B C
        // |/
        // A
        // |
        // root
        //
        // Lowercase nodes are external to the set
        let mut tx = test_repo.repo.start_transaction();
        let mut create_commit = |parents| {
            tx.repo_mut()
                .new_commit(parents, empty_tree.clone())
                .write_unwrap()
        };
        let commit_a = create_commit(vec![store.root_commit_id().clone()]);
        let commit_b = create_commit(vec![commit_a.id().clone()]);
        let commit_c = create_commit(vec![commit_a.id().clone()]);
        let commit_d = create_commit(vec![commit_b.id().clone(), commit_c.id().clone()]);
        let commit_e = create_commit(vec![commit_c.id().clone()]);
        let commit_f = create_commit(vec![commit_d.id().clone()]);

        let mut state = State::new(
            test_repo.repo.as_ref(),
            vec![
                commit_d.clone(),
                commit_c.clone(),
                commit_b.clone(),
                commit_a.clone(),
            ],
            vec![commit_e.clone(), commit_f.clone()],
        )
        .block_on()?;
        assert_eq!(state.head_order, vec![commit_d.id().clone()]);
        assert_eq!(
            state.current_order,
            vec![
                commit_d.id().clone(),
                commit_b.id().clone(),
                commit_c.id().clone(),
                commit_a.id().clone()
            ]
        );

        // Swap D and C:
        // f           f
        // |           |
        // D e         C e
        // |\|         |\|
        // B C    =>   B D
        // |/          |/
        // A           A
        // |           |
        // root        root
        //
        state.swap_commits(commit_d.id(), commit_c.id());
        assert_eq!(state.head_order, vec![commit_c.id().clone()]);
        assert_eq!(
            state.current_order,
            vec![
                commit_c.id().clone(),
                commit_b.id().clone(),
                commit_d.id().clone(),
                commit_a.id().clone()
            ]
        );
        assert_eq!(state.current_selection, 2);
        assert_eq!(
            *state.commits.get(commit_c.id()).unwrap().parents,
            vec![commit_b.id().clone(), commit_d.id().clone()],
        );
        assert_eq!(
            *state.commits.get(commit_d.id()).unwrap().parents,
            vec![commit_a.id().clone()],
        );
        assert_eq!(
            *state.commits.get(commit_e.id()).unwrap().parents,
            vec![commit_d.id().clone()],
        );
        assert_eq!(
            *state.commits.get(commit_f.id()).unwrap().parents,
            vec![commit_c.id().clone()],
        );

        // Swap B and D:
        // f           f
        // |           |
        // C e         C e
        // |\|         |\|
        // B D    =>   D B
        // |/          |/
        // A           A
        // |           |
        // root        root
        //
        state.swap_commits(commit_b.id(), commit_d.id());
        assert_eq!(state.head_order, vec![commit_c.id().clone()]);
        assert_eq!(
            state.current_order,
            vec![
                commit_c.id().clone(),
                commit_d.id().clone(),
                commit_b.id().clone(),
                commit_a.id().clone()
            ]
        );
        assert_eq!(state.current_selection, 1);
        assert_eq!(
            *state.commits.get(commit_c.id()).unwrap().parents,
            vec![commit_d.id().clone(), commit_b.id().clone()],
        );
        assert_eq!(
            *state.commits.get(commit_d.id()).unwrap().parents,
            vec![commit_a.id().clone()],
        );
        assert_eq!(
            *state.commits.get(commit_b.id()).unwrap().parents,
            vec![commit_a.id().clone()],
        );
        assert_eq!(
            *state.commits.get(commit_e.id()).unwrap().parents,
            vec![commit_b.id().clone()],
        );

        // Swap A and C:
        // f           f
        // |           |
        // C e         A e
        // |\|         |\|
        // D B    =>   D B
        // |/          |/
        // A           C
        // |           |
        // root        root
        //
        state.swap_commits(commit_a.id(), commit_c.id());
        assert_eq!(state.head_order, vec![commit_a.id().clone()]);
        assert_eq!(
            state.current_order,
            vec![
                commit_a.id().clone(),
                commit_d.id().clone(),
                commit_b.id().clone(),
                commit_c.id().clone()
            ]
        );
        assert_eq!(state.current_selection, 1);
        assert_eq!(
            *state.commits.get(commit_a.id()).unwrap().parents,
            vec![commit_d.id().clone(), commit_b.id().clone()],
        );
        assert_eq!(
            *state.commits.get(commit_d.id()).unwrap().parents,
            vec![commit_c.id().clone()],
        );
        assert_eq!(
            *state.commits.get(commit_b.id()).unwrap().parents,
            vec![commit_c.id().clone()],
        );
        assert_eq!(
            *state.commits.get(commit_f.id()).unwrap().parents,
            vec![commit_a.id().clone()],
        );

        // No-op swap
        state.swap_commits(commit_a.id(), commit_a.id());
        assert_eq!(state.current_selection, 1);
        assert_eq!(
            state.current_order,
            vec![
                commit_a.id().clone(),
                commit_d.id().clone(),
                commit_b.id().clone(),
                commit_c.id().clone()
            ]
        );
        Ok(())
    }

    #[test]
    fn test_swap_selection_down() -> TestResult {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();
        let empty_tree = store.empty_merged_tree();

        // Construct the graph:
        // f
        // |
        // D e
        // |\|
        // B C
        // |/
        // A
        // |
        // root
        //
        // Lowercase nodes are external to the set
        let mut tx = test_repo.repo.start_transaction();
        let mut create_commit = |parents| {
            tx.repo_mut()
                .new_commit(parents, empty_tree.clone())
                .write_unwrap()
        };
        let commit_a = create_commit(vec![store.root_commit_id().clone()]);
        let commit_b = create_commit(vec![commit_a.id().clone()]);
        let commit_c = create_commit(vec![commit_a.id().clone()]);
        let commit_d = create_commit(vec![commit_b.id().clone(), commit_c.id().clone()]);
        let commit_e = create_commit(vec![commit_c.id().clone()]);
        let commit_f = create_commit(vec![commit_d.id().clone()]);

        let mut state = State::new(
            test_repo.repo.as_ref(),
            vec![
                commit_d.clone(),
                commit_c.clone(),
                commit_b.clone(),
                commit_a.clone(),
            ],
            vec![commit_e.clone(), commit_f.clone()],
        )
        .block_on()?;
        assert_eq!(
            state.current_order,
            vec![
                commit_d.id().clone(),
                commit_b.id().clone(),
                commit_c.id().clone(),
                commit_a.id().clone(),
            ]
        );

        // Attempting to swap D down should have no effect because it has two parents
        state.current_selection = 0;
        assert_eq!(state.current_id(), commit_d.id());
        let state_before = state.clone();
        state.swap_selection_down();
        assert_eq!(state, state_before);

        // Swap B down:
        // f           f
        // |           |
        // D e         D e
        // |\|         |\|
        // B C    =>   A C
        // |/          |/
        // A           B
        // |           |
        // root        root
        //
        state.current_selection = 1;
        assert_eq!(state.current_id(), commit_b.id());
        state.swap_selection_down();
        assert_eq!(
            state.current_order,
            vec![
                commit_d.id().clone(),
                commit_a.id().clone(),
                commit_c.id().clone(),
                commit_b.id().clone(),
            ]
        );
        assert_eq!(state.current_selection, 3);

        // Swap C down:
        // f           f
        // |           |
        // D e         D e
        // |\|         |\|
        // A C    =>   A B
        // |/          |/
        // B           C
        // |           |
        // root        root
        //
        state.current_selection = 2;
        assert_eq!(state.current_id(), commit_c.id());
        state.swap_selection_down();
        assert_eq!(
            state.current_order,
            vec![
                commit_d.id().clone(),
                commit_a.id().clone(),
                commit_b.id().clone(),
                commit_c.id().clone(),
            ]
        );
        assert_eq!(state.current_selection, 3);

        // Attempting to swap C down should have no effect because it would move outside
        // of range
        state.current_selection = 3;
        assert_eq!(state.current_id(), commit_c.id());
        let state_before = state.clone();
        state.swap_selection_down();
        assert_eq!(state, state_before);

        Ok(())
    }

    #[test]
    fn test_swap_selection_up() -> TestResult {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();
        let empty_tree = store.empty_merged_tree();

        // Construct the graph:
        // f
        // |
        // D e
        // |\|
        // B C
        // |/
        // A
        // |
        // root
        //
        // Lowercase nodes are external to the set
        let mut tx = test_repo.repo.start_transaction();
        let mut create_commit = |parents| {
            tx.repo_mut()
                .new_commit(parents, empty_tree.clone())
                .write_unwrap()
        };
        let commit_a = create_commit(vec![store.root_commit_id().clone()]);
        let commit_b = create_commit(vec![commit_a.id().clone()]);
        let commit_c = create_commit(vec![commit_a.id().clone()]);
        let commit_d = create_commit(vec![commit_b.id().clone(), commit_c.id().clone()]);
        let commit_e = create_commit(vec![commit_c.id().clone()]);
        let commit_f = create_commit(vec![commit_d.id().clone()]);

        let mut state = State::new(
            test_repo.repo.as_ref(),
            vec![
                commit_d.clone(),
                commit_c.clone(),
                commit_b.clone(),
                commit_a.clone(),
            ],
            vec![commit_e.clone(), commit_f.clone()],
        )
        .block_on()?;
        assert_eq!(
            state.current_order,
            vec![
                commit_d.id().clone(),
                commit_b.id().clone(),
                commit_c.id().clone(),
                commit_a.id().clone(),
            ]
        );

        // Attempting to swap A up should have no effect because it has two children
        state.current_selection = 3;
        assert_eq!(state.current_id(), commit_a.id());
        let state_before = state.clone();
        state.swap_selection_up();
        assert_eq!(state, state_before);

        // Attempting to swap C up should have no effect because it has two children
        // even though one is external. We could change this to ignore the external
        // child.
        state.current_selection = 2;
        assert_eq!(state.current_id(), commit_c.id());
        let state_before = state.clone();
        state.swap_selection_up();
        assert_eq!(state, state_before);

        // Swap B up:
        // f           f
        // |           |
        // D e         B e
        // |\|         |\|
        // B C    =>   D C
        // |/          |/
        // A           A
        // |           |
        // root        root
        //
        state.current_selection = 1;
        assert_eq!(state.current_id(), commit_b.id());
        state.swap_selection_up();
        assert_eq!(
            state.current_order,
            vec![
                commit_b.id().clone(),
                commit_d.id().clone(),
                commit_c.id().clone(),
                commit_a.id().clone(),
            ]
        );
        assert_eq!(state.current_selection, 0);

        // Attempting to swap B up should have no effect because it would move outside
        // of range
        state.current_selection = 0;
        assert_eq!(state.current_id(), commit_b.id());
        let state_before = state.clone();
        state.swap_selection_up();
        assert_eq!(state, state_before);

        Ok(())
    }

    #[test]
    fn test_execute_plan_reorder() -> TestResult {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();
        let empty_tree = store.empty_merged_tree();

        // Move A between C and D, let E follow:
        //   F           F E
        //   |           |/
        // D C           A
        // |/            |
        // B E    =>   D C
        // |/          |/
        // A           B
        // |           |
        // root        root
        let mut tx = test_repo.repo.start_transaction();
        let mut create_commit = |parents| {
            tx.repo_mut()
                .new_commit(parents, empty_tree.clone())
                .write_unwrap()
        };
        let commit_a = create_commit(vec![store.root_commit_id().clone()]);
        let commit_b = create_commit(vec![commit_a.id().clone()]);
        let commit_c = create_commit(vec![commit_b.id().clone()]);
        let commit_d = create_commit(vec![commit_b.id().clone()]);
        let commit_e = create_commit(vec![commit_a.id().clone()]);
        let commit_f = create_commit(vec![commit_c.id().clone()]);
        let mut plan = no_op_plan(&[
            &commit_a, &commit_b, &commit_c, &commit_d, &commit_e, &commit_f,
        ]);

        // Update the plan with the new parents
        plan.rewrites.get_mut(commit_a.id()).unwrap().new_parents = vec![commit_c.id().clone()];
        plan.rewrites.get_mut(commit_b.id()).unwrap().new_parents =
            vec![store.root_commit_id().clone()];
        plan.rewrites.get_mut(commit_f.id()).unwrap().new_parents = vec![commit_a.id().clone()];

        let rewritten = plan.execute(tx.repo_mut()).block_on().unwrap();
        tx.repo_mut().rebase_descendants().block_on()?;
        assert_eq!(
            rewritten.keys().collect::<HashSet<_>>(),
            hashset![
                commit_a.id(),
                commit_b.id(),
                commit_c.id(),
                commit_d.id(),
                commit_e.id(),
                commit_f.id(),
            ]
        );
        let new_commit_a = rewritten.get(commit_a.id()).unwrap();
        let new_commit_b = rewritten.get(commit_b.id()).unwrap();
        let new_commit_c = rewritten.get(commit_c.id()).unwrap();
        let new_commit_d = rewritten.get(commit_d.id()).unwrap();
        let new_commit_e = rewritten.get(commit_e.id()).unwrap();
        let new_commit_f = rewritten.get(commit_f.id()).unwrap();
        assert_eq!(new_commit_b.parent_ids(), &[store.root_commit_id().clone()]);
        assert_eq!(new_commit_c.parent_ids(), &[new_commit_b.id().clone()]);
        assert_eq!(new_commit_a.parent_ids(), &[new_commit_c.id().clone()]);
        assert_eq!(new_commit_d.parent_ids(), &[new_commit_b.id().clone()]);
        assert_eq!(new_commit_e.parent_ids(), &[new_commit_a.id().clone()]);
        assert_eq!(new_commit_f.parent_ids(), &[new_commit_a.id().clone()]);
        Ok(())
    }

    #[test]
    fn test_execute_plan_abandon() -> TestResult {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();
        let empty_tree = store.empty_merged_tree();

        // Move C onto A and abandon it:
        // D           D
        // |           |
        // C           C (abandoned)
        // |           |
        // B    =>   B |
        // |         |/
        // A         A
        // |         |
        // root      root
        let mut tx = test_repo.repo.start_transaction();
        let mut create_commit = |parents| {
            tx.repo_mut()
                .new_commit(parents, empty_tree.clone())
                .write_unwrap()
        };
        let commit_a = create_commit(vec![store.root_commit_id().clone()]);
        let commit_b = create_commit(vec![commit_a.id().clone()]);
        let commit_c = create_commit(vec![commit_b.id().clone()]);
        let commit_d = create_commit(vec![commit_c.id().clone()]);
        let mut plan = no_op_plan(&[&commit_a, &commit_b, &commit_c, &commit_d]);

        // Update parents and action, then apply the changes.
        *plan.rewrites.get_mut(commit_c.id()).unwrap() = Rewrite {
            old_commit: commit_c.clone(),
            new_parents: vec![commit_a.id().clone()],
            action: RewriteAction::Abandon,
        };

        let rewritten = plan.execute(tx.repo_mut()).block_on().unwrap();
        tx.repo_mut().rebase_descendants().block_on()?;
        assert_eq!(rewritten.keys().sorted().collect_vec(), vec![commit_d.id()]);
        let new_commit_d = rewritten.get(commit_d.id()).unwrap();
        assert_eq!(new_commit_d.parent_ids(), &[commit_a.id().clone()]);
        assert_eq!(
            *tx.repo_mut().view().heads(),
            hashset![commit_b.id().clone(), new_commit_d.id().clone()]
        );
        Ok(())
    }

    #[test]
    fn test_rebase_popup_transitions() -> TestResult {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();
        let empty_tree = store.empty_merged_tree();

        let mut tx = test_repo.repo.start_transaction();
        let mut create_commit = |parents| {
            tx.repo_mut()
                .new_commit(parents, empty_tree.clone())
                .write_unwrap()
        };
        let commit_a = create_commit(vec![store.root_commit_id().clone()]);
        let commit_b = create_commit(vec![commit_a.id().clone()]);

        let mut state = State::new(
            test_repo.repo.as_ref(),
            vec![commit_b.clone(), commit_a.clone()],
            vec![],
        )
        .block_on()?;

        // Initially no popup
        assert!(state.rebase_popup.is_none());

        // Press 'r' on commit_b (has 1 parent)
        state.current_selection = 0; // commit_b
        state = handle_key_event(KeyEvent {
            code: KeyCode::Char('r'),
            modifiers: KeyModifiers::NONE,
            kind: event::KeyEventKind::Press,
            state: event::KeyEventState::empty(),
        }, state);
        assert!(state.rebase_popup.is_some());
        
        let popup = state.rebase_popup.as_ref().unwrap();
        assert_eq!(popup.filter_text, "");
        assert_eq!(popup.candidates, vec![commit_a.id().clone()]);

        // Press 'Esc' to close
        state = handle_key_event(KeyEvent {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
            kind: event::KeyEventKind::Press,
            state: event::KeyEventState::empty(),
        }, state);
        assert!(state.rebase_popup.is_none());

        Ok(())
    }

    #[test]
    fn test_rebase_popup_filter() -> TestResult {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();
        let empty_tree = store.empty_merged_tree();

        let mut tx = test_repo.repo.start_transaction();
        let mut create_commit = |parents, desc: &str| {
            tx.repo_mut()
                .new_commit(parents, empty_tree.clone())
                .set_description(desc)
                .write_unwrap()
        };
        let commit_a = create_commit(vec![store.root_commit_id().clone()], "feat: a");
        let commit_b = create_commit(vec![commit_a.id().clone()], "fix: b");
        let commit_c = create_commit(vec![commit_a.id().clone()], "feat: c");

        let mut state = State::new(
            test_repo.repo.as_ref(),
            vec![commit_c.clone(), commit_b.clone(), commit_a.clone()],
            vec![],
        )
        .block_on()?;

        // Find index of commit_b in current_order
        let b_idx = state.current_order.iter().position(|id| *id == *commit_b.id()).unwrap();
        state.current_selection = b_idx;
        state = handle_key_event(KeyEvent {
            code: KeyCode::Char('r'),
            modifiers: KeyModifiers::NONE,
            kind: event::KeyEventKind::Press,
            state: event::KeyEventState::empty(),
        }, state);
        
        // Type 'a'
        state = handle_key_event(KeyEvent {
            code: KeyCode::Char('a'),
            modifiers: KeyModifiers::NONE,
            kind: event::KeyEventKind::Press,
            state: event::KeyEventState::empty(),
        }, state);

        let popup = state.rebase_popup.as_ref().unwrap();
        assert_eq!(popup.filter_text, "a");
        let current_id = state.current_id().clone();
        let candidates = state.get_filtered_candidates(&current_id, &popup.filter_text);
        assert_eq!(candidates, vec![commit_a.id().clone()]); // Only A matches "a"

        Ok(())
    }

    #[test]
    fn test_descendant_exclusion() -> TestResult {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();
        let empty_tree = store.empty_merged_tree();

        let mut tx = test_repo.repo.start_transaction();
        let mut create_commit = |parents| {
            tx.repo_mut()
                .new_commit(parents, empty_tree.clone())
                .write_unwrap()
        };
        let commit_a = create_commit(vec![store.root_commit_id().clone()]);
        let commit_b = create_commit(vec![commit_a.id().clone()]);
        let commit_c = create_commit(vec![commit_b.id().clone()]);

        let state = State::new(
            test_repo.repo.as_ref(),
            vec![commit_c.clone(), commit_b.clone(), commit_a.clone()],
            vec![],
        )
        .block_on()?;

        // Descendants of A should be B and C
        let descendants = state.get_descendants(commit_a.id());
        assert_eq!(descendants.len(), 2);
        assert!(descendants.contains(commit_b.id()));
        assert!(descendants.contains(commit_c.id()));

        Ok(())
    }

    #[test]
    fn test_update_heads_after_rebase() -> TestResult {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();
        let empty_tree = store.empty_merged_tree();

        let mut tx = test_repo.repo.start_transaction();
        let mut create_commit = |parents| {
            tx.repo_mut()
                .new_commit(parents, empty_tree.clone())
                .write_unwrap()
        };
        let commit_a = create_commit(vec![store.root_commit_id().clone()]);
        let commit_b = create_commit(vec![commit_a.id().clone()]);
        let commit_c = create_commit(vec![commit_a.id().clone()]);

        let mut state = State::new(
            test_repo.repo.as_ref(),
            vec![commit_c.clone(), commit_b.clone(), commit_a.clone()],
            vec![],
        )
        .block_on()?;

        // Initially heads are B and C
        assert_eq!(state.head_order.len(), 2);
        assert!(state.head_order.contains(commit_b.id()));
        assert!(state.head_order.contains(commit_c.id()));

        // Rebase C onto B
        state.commits.get_mut(commit_c.id()).unwrap().parents = vec![commit_b.id().clone()];
        state.update_heads();

        // Now only C is a head
        assert_eq!(state.head_order, vec![commit_c.id().clone()]);

        Ok(())
    }

    #[test]
    fn test_rebase_popup_filter_bookmarks() -> TestResult {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();
        let empty_tree = store.empty_merged_tree();

        let mut tx = test_repo.repo.start_transaction();
        let mut create_commit = |parents, desc: &str| {
            tx.repo_mut()
                .new_commit(parents, empty_tree.clone())
                .set_description(desc)
                .write_unwrap()
        };
        let commit_a = create_commit(vec![store.root_commit_id().clone()], "feat: a");
        let commit_b = create_commit(vec![commit_a.id().clone()], "fix: b");
        let commit_c = create_commit(vec![commit_a.id().clone()], "feat: c");

        // Add a bookmark to A
        tx.repo_mut().set_local_bookmark_target("my-bookmark".as_ref(), jj_lib::op_store::RefTarget::normal(commit_a.id().clone()));

        let mut state = State::new(
            test_repo.repo.as_ref(),
            vec![commit_c.clone(), commit_b.clone(), commit_a.clone()],
            vec![],
        )
        .block_on()?;

        // Find index of commit_b in current_order
        let b_idx = state.current_order.iter().position(|id| *id == *commit_b.id()).unwrap();
        state.current_selection = b_idx;
        state = handle_key_event(KeyEvent {
            code: KeyCode::Char('r'),
            modifiers: KeyModifiers::NONE,
            kind: event::KeyEventKind::Press,
            state: event::KeyEventState::empty(),
        }, state);
        
        // Type 'm' (for my-bookmark)
        state = handle_key_event(KeyEvent {
            code: KeyCode::Char('m'),
            modifiers: KeyModifiers::NONE,
            kind: event::KeyEventKind::Press,
            state: event::KeyEventState::empty(),
        }, state);

        let popup = state.rebase_popup.as_ref().unwrap();
        assert_eq!(popup.filter_text, "m");
        let current_id = state.current_id().clone();
        let candidates = state.get_filtered_candidates(&current_id, &popup.filter_text);
        assert_eq!(candidates, vec![commit_a.id().clone()]); // Only A matches "my-bookmark"

        Ok(())
    }
}
