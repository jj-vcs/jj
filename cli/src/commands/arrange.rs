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

use crossterm::ExecutableCommand as _;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;
use crossterm::event::{self};
use crossterm::terminal::EnterAlternateScreen;
use crossterm::terminal::LeaveAlternateScreen;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use indexmap::IndexSet;
use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::dag_walk;
use jj_lib::repo::MutableRepo;
use jj_lib::repo::Repo as _;
use jj_lib::revset::RevsetIteratorExt as _;
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
use renderdag::Ancestor;
use renderdag::GraphRowRenderer;
use renderdag::Renderer as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
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
    /// The revisions to edit.
    ///
    /// If no revisions are specified, this defaults to the `revsets.arrange`
    /// setting, or `reachable(@, mutable())` if it is not set.
    #[arg(long, short, value_name = "REVSETS")]
    #[arg(add = clap_complete::ArgValueCompleter::new(complete::revset_expression_mutable))]
    revisions: Vec<RevisionArg>,
}

#[instrument(skip_all)]
pub(crate) async fn cmd_arrange(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &ArrangeArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo().clone();
    let target_expression = if args.revisions.is_empty() {
        let revs = workspace_command.settings().get_string("revsets.arrange")?;
        workspace_command.parse_revset(ui, &RevisionArg::from(revs))?
    } else {
        workspace_command.parse_union_revsets(ui, &args.revisions)?
    }
    .resolve()?;
    workspace_command.check_rewritable_expr(&target_expression)?;

    let gaps_revset = target_expression
        .connected()
        .minus(&target_expression)
        .evaluate(repo.as_ref())?;
    if let Some(commit_id) = gaps_revset.iter().next() {
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
    let external_children: Vec<_> = children_revset.iter().commits(repo.store()).try_collect()?;

    let revset = target_expression.evaluate(repo.as_ref())?;
    let commits: Vec<Commit> = revset.iter().commits(repo.store()).try_collect()?;
    if commits.is_empty() {
        writeln!(ui.status(), "No revisions to arrange.")?;
        return Ok(());
    }

    // Set up the terminal
    io::stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;

    let mut state = State::new(commits, external_children);
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
        tx.finish(ui, "arrange revisions")?;
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
struct CommitState {
    commit: Commit,
    action: UiAction,
    parents: Vec<CommitId>,
}

struct State {
    /// Commits in the target set, as well as any external children.
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
}

impl State {
    /// Creates a new `State` from a list of commits and a list of external
    /// children. The list of commits must not have gaps between commits.
    fn new(commits: Vec<Commit>, external_children: Vec<Commit>) -> Self {
        // Initialize head_order to match the heads in the input's order.
        let mut heads: HashSet<_> = commits.iter().map(|commit| commit.id()).collect();
        for commit in &commits {
            for parent in commit.parent_ids() {
                heads.remove(parent);
            }
        }
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
            .map(|commit: Commit| {
                let id = commit.id().clone();
                let parents = commit.parent_ids().to_vec();
                let commit_state = CommitState {
                    commit,
                    action: UiAction::Keep,
                    parents,
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
        };
        state.update_commit_order();
        state
    }

    /// Update the current UI commit order after parents have changed.
    fn update_commit_order(&mut self) {
        // Use the original order to get a determinisic order.
        // TODO: Use TopoGroupedGraphIterator so the order better matches `jj log`
        let commit_ids: Vec<&CommitId> = dag_walk::topo_order_reverse(
            self.head_order.iter(),
            |id| *id,
            |id| {
                self.commits
                    .get(id)
                    .unwrap()
                    .parents
                    .iter()
                    .filter(|id| self.commits.contains_key(id))
            },
            |_| panic!("cycle detected"),
        )
        .unwrap();
        self.current_order = commit_ids.into_iter().cloned().collect();
    }

    /// Check if one commit is a parent of the other or vice versa.
    fn are_graph_neighbors(&self, a_idx: usize, b_idx: usize) -> bool {
        let a_id = &self.current_order[a_idx];
        let b_id = &self.current_order[b_idx];
        self.commits.get(b_id).unwrap().parents.contains(a_id)
            || self.commits.get(a_id).unwrap().parents.contains(b_id)
    }

    fn swap_commits(&mut self, a_idx: usize, b_idx: usize) {
        if a_idx == b_idx {
            return;
        }

        if self.current_selection == a_idx {
            self.current_selection = b_idx;
        } else if self.current_selection == b_idx {
            self.current_selection = a_idx;
        }

        self.current_order.swap(a_idx, b_idx);
        // Backwards because we just swapped them. It doesn't matter which is which
        // anyway.
        let a_id = &self.current_order[b_idx];
        let b_id = &self.current_order[a_idx];

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
            match (event.code, event.modifiers) {
                (KeyCode::Char('q'), KeyModifiers::NONE)
                | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    return Ok(None);
                }
                (KeyCode::Char('c'), KeyModifiers::NONE) => {
                    return Ok(Some(state));
                }
                (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE) => {
                    if state.current_selection + 1 < state.current_order.len() {
                        state.current_selection += 1;
                    }
                }
                (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE) => {
                    if state.current_selection > 0 {
                        state.current_selection -= 1;
                    }
                }
                (KeyCode::Char('a'), KeyModifiers::NONE) => {
                    let id = state.current_order[state.current_selection].clone();
                    state.commits.get_mut(&id).unwrap().action = UiAction::Abandon;
                }
                (KeyCode::Char('p'), KeyModifiers::NONE) => {
                    let id = state.current_order[state.current_selection].clone();
                    state.commits.get_mut(&id).unwrap().action = UiAction::Keep;
                }
                (KeyCode::Down | KeyCode::Char('J'), KeyModifiers::SHIFT) => {
                    if state.current_selection + 1 < state.current_order.len()
                        && state.are_graph_neighbors(
                            state.current_selection,
                            state.current_selection + 1,
                        )
                    {
                        state.swap_commits(state.current_selection, state.current_selection + 1);
                    }
                }
                (KeyCode::Up | KeyCode::Char('K'), KeyModifiers::SHIFT) => {
                    if state.current_selection > 0
                        && state.are_graph_neighbors(
                            state.current_selection,
                            state.current_selection - 1,
                        )
                    {
                        state.swap_commits(state.current_selection, state.current_selection - 1);
                    }
                }
                _ => {
                    continue;
                }
            }
            state.update_commit_order();
        }
    }
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
    let current_seletion_id = &state.current_order[state.current_selection];
    let commits_to_render = state
        .external_children
        .iter()
        .chain(state.current_order.iter());
    // TODO: also render external parents dimmed
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
        let graph_lines = row_renderer.next_row(id, edges, glyph.to_string(), "".to_string());
        let graph_text = Text::from(graph_lines);
        row_area = row_area
            .offset(Offset {
                x: 0,
                y: graph_text.height() as i32,
            })
            .intersection(main_area);
        frame.render_widget(graph_text, graph_area);

        let is_context_node = state.external_children.contains(id);
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
    }
}

#[cfg(test)]
mod tests {
    use maplit::hashset;
    use pollster::FutureExt as _;
    use testutils::CommitBuilderExt as _;
    use testutils::TestRepo;

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
    fn test_update_commit_order_empty() {
        let mut state = State::new(vec![], vec![]);
        assert_eq!(state.head_order, vec![]);
        state.update_commit_order();
        assert_eq!(state.current_order, vec![]);
    }

    #[test]
    fn test_update_commit_order_reorder() {
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
            vec![
                commit_d.clone(),
                commit_c.clone(),
                commit_b.clone(),
                commit_a.clone(),
            ],
            vec![],
        );

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
    }

    #[test]
    fn test_swap_commits() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();
        let empty_tree = store.empty_merged_tree();

        // Swap C and D:
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
            vec![
                commit_d.clone(),
                commit_c.clone(),
                commit_b.clone(),
                commit_a.clone(),
            ],
            vec![commit_e.clone(), commit_f.clone()],
        );
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

        // Swap C and D and check result
        state.swap_commits(0, 2);
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
    }

    #[test]
    fn test_execute_plan_reorder() {
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
        tx.repo_mut().rebase_descendants().block_on().unwrap();
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
    }

    #[test]
    fn test_execute_plan_abandon() {
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
        tx.repo_mut().rebase_descendants().block_on().unwrap();
        assert_eq!(rewritten.keys().sorted().collect_vec(), vec![commit_d.id()]);
        let new_commit_d = rewritten.get(commit_d.id()).unwrap();
        assert_eq!(new_commit_d.parent_ids(), &[commit_a.id().clone()]);
        assert_eq!(
            *tx.repo_mut().view().heads(),
            hashset![commit_b.id().clone(), new_commit_d.id().clone()]
        );
    }
}
