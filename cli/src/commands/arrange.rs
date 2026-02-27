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

    let result = run_tui(
        ui,
        &mut terminal,
        &workspace_command.commit_summary_template(),
        state,
    );

    // Restore the terminal
    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

    if let Some(new_state) = result? {
        let mut tx = workspace_command.start_transaction();
        new_state.apply_changes(tx.repo_mut()).await?;
        tx.finish(ui, "arrange revisions")?;
        Ok(())
    } else {
        Err(user_error("Canceled by user"))
    }
}

enum Action {
    Abandon,
    Keep,
}

struct State {
    commits: HashMap<CommitId, Commit>,
    /// Heads of the set in the order they should be added to the UI. This is
    /// used to make the graph rendering more stable. It must be kept up to date
    /// parents are changed.
    head_order: Vec<CommitId>,
    /// The current order of commits in the UI. This is recalculated when
    /// necessary from `head_order`.
    current_order: Vec<CommitId>,
    // The current selection as an index into `current_order`
    current_selection: usize,
    actions: HashMap<CommitId, Action>,
    parents: HashMap<CommitId, Vec<CommitId>>,
    external_children: HashMap<CommitId, Commit>,
}

impl State {
    fn new(commits: Vec<Commit>, external_children: Vec<Commit>) -> Self {
        let current_order = commits
            .iter()
            .map(|commit| commit.id().clone())
            .collect_vec();
        let actions = commits
            .iter()
            .chain(external_children.iter())
            .map(|commit| (commit.id().clone(), Action::Keep))
            .collect();
        let commits: HashMap<CommitId, Commit> = commits
            .into_iter()
            .map(|commit| {
                let id = commit.id().clone();
                (id, commit)
            })
            .collect();
        let mut parents: HashMap<CommitId, Vec<CommitId>> = HashMap::new();
        for (id, commit) in &commits {
            parents.insert(id.clone(), commit.parent_ids().to_vec());
        }
        for child in &external_children {
            parents.insert(child.id().clone(), child.parent_ids().to_vec());
        }
        let external_children = external_children
            .into_iter()
            .map(|commit| (commit.id().clone(), commit))
            .collect();
        // Initialize head_order to match the heads in the input's order.
        let heads: HashSet<&CommitId> = dag_walk::heads(
            current_order.iter(),
            |id| *id,
            |id| {
                parents
                    .get(id)
                    .unwrap()
                    .iter()
                    .filter(|id| commits.contains_key(id))
            },
        );
        let head_order = current_order
            .iter()
            .filter(|id| heads.contains(id))
            .cloned()
            .collect_vec();
        Self {
            commits,
            head_order,
            current_order,
            current_selection: 0,
            actions,
            parents,
            external_children,
        }
    }

    /// Update the current UI commit order after parents have changed.
    fn update_commit_order(&mut self) {
        // Use the original order to get a determinisic order.
        let commit_ids: Vec<&CommitId> = dag_walk::topo_order_reverse(
            self.head_order.iter(),
            |id| *id,
            |id| {
                self.parents
                    .get(id)
                    .unwrap()
                    .iter()
                    .filter(|id| self.commits.contains_key(id))
            },
            |_| panic!("cycle detected"),
        )
        .unwrap();
        self.current_order = commit_ids.into_iter().cloned().collect();
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
        for parents in self.parents.values_mut() {
            for id in parents {
                if id == a_id {
                    *id = b_id.clone();
                } else if id == b_id {
                    *id = a_id.clone();
                }
            }
        }

        // Temporarily remove the parents of the swapped commits
        let a_parents = self.parents.remove(a_id).unwrap();
        let b_parents = self.parents.remove(b_id).unwrap();
        self.parents.insert(a_id.clone(), b_parents);
        self.parents.insert(b_id.clone(), a_parents);
    }

    async fn apply_changes(
        mut self,
        mut_repo: &mut MutableRepo,
    ) -> Result<HashMap<CommitId, Commit>, CommandError> {
        // Find order to rebase the commits. The order is determined by the new
        // parents.
        let ordered_commit_ids = dag_walk::topo_order_forward(
            self.parents.keys().cloned(),
            |id| id.clone(),
            |id| {
                self.parents
                    .get(id)
                    .unwrap()
                    .iter()
                    .filter(|id| self.commits.contains_key(id))
                    .cloned()
            },
            |_| panic!("cycle detected"),
        )
        .unwrap();
        // Rewrite the commits in the order determined above
        let mut rewritten_commits: HashMap<CommitId, Commit> = HashMap::new();
        for id in ordered_commit_ids {
            let old_commit = self
                .commits
                .remove(&id)
                .or_else(|| self.external_children.remove(&id))
                .unwrap();
            let new_parents = mut_repo.new_parents(self.parents.get(&id).unwrap());
            let rewriter = CommitRewriter::new(mut_repo, old_commit, new_parents);
            let action = self.actions.remove(rewriter.old_commit().id()).unwrap();
            match action {
                Action::Abandon => rewriter.abandon(),
                Action::Keep => {
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
    template: &TemplateRenderer<Commit>,
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
                render(&state, ui, template, frame, main_area);
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
                (KeyCode::Char('q'), KeyModifiers::NONE) => {
                    return Ok(None);
                }
                (KeyCode::Char('c'), KeyModifiers::NONE) => {
                    return Ok(Some(state));
                }
                (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE) => {
                    if state.current_selection + 1 < state.commits.len() {
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
                    state.actions.insert(id, Action::Abandon);
                }
                (KeyCode::Char('p'), KeyModifiers::NONE) => {
                    let id = state.current_order[state.current_selection].clone();
                    state.actions.insert(id, Action::Keep);
                }
                // TODO: Allow swapping up/down only within linear parts of the graph.
                (KeyCode::Down | KeyCode::Char('J'), KeyModifiers::SHIFT) => {
                    if state.current_selection + 1 < state.commits.len() {
                        state.swap_commits(state.current_selection, state.current_selection + 1);
                    }
                }
                (KeyCode::Up | KeyCode::Char('K'), KeyModifiers::SHIFT) => {
                    if state.current_selection > 0 {
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
    // TODO: It might be nice to render external parents and children grayed out
    for (index, id) in state.current_order.iter().enumerate() {
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

        if index == state.current_selection {
            frame.render_widget(Text::from("▶"), selection_area);
        }

        let commit = state.commits.get(id).unwrap();
        let action = state.actions.get(id).unwrap();

        // TODO: The graph can be misaligned with the text because sometimes `renderdag`
        // inserts a line of edges before the line with the node and we assume the node
        // is the first line emitted.
        let parents = state.parents.get(id).unwrap();
        let edges = parents
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
            Action::Abandon => "×",
            Action::Keep => "○",
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

        let action_text = match action {
            Action::Abandon => "abandon",
            Action::Keep => "keep",
        };
        frame.render_widget(Text::from(action_text), action_area);

        let mut text_lines = vec![];
        let mut formatter = ui.new_formatter(&mut text_lines);
        template.format(commit, formatter.as_mut()).unwrap();
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
        state.update_commit_order();
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
        state
            .parents
            .insert(commit_a.id().clone(), vec![commit_c.id().clone()]);
        state
            .parents
            .insert(commit_b.id().clone(), vec![store.root_commit_id().clone()]);
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

        // Swap C and D and check result
        state.swap_commits(0, 1);
        assert_eq!(state.head_order, vec![commit_c.id().clone()]);
        assert_eq!(state.current_selection, 1);
        assert_eq!(
            *state.parents.get(commit_c.id()).unwrap(),
            vec![commit_b.id().clone(), commit_d.id().clone()],
        );
        assert_eq!(
            *state.parents.get(commit_d.id()).unwrap(),
            vec![commit_a.id().clone()],
        );
        assert_eq!(
            *state.parents.get(commit_e.id()).unwrap(),
            vec![commit_d.id().clone()],
        );
        assert_eq!(
            *state.parents.get(commit_f.id()).unwrap(),
            vec![commit_c.id().clone()],
        );
    }

    #[test]
    fn test_apply_changes_reorder() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();
        let empty_tree = store.empty_merged_tree();

        // Move A between C and D, let e follow:
        //   f           f e
        //   |           |/
        // D C           A
        // |/            |
        // B e    =>   D C
        // |/          |/
        // A           B
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
        let commit_c = create_commit(vec![commit_b.id().clone()]);
        let commit_d = create_commit(vec![commit_b.id().clone()]);
        let commit_e = create_commit(vec![commit_a.id().clone()]);
        let commit_f = create_commit(vec![commit_c.id().clone()]);

        let mut state = State::new(
            vec![
                commit_d.clone(),
                commit_c.clone(),
                commit_b.clone(),
                commit_a.clone(),
            ],
            vec![commit_f.clone(), commit_e.clone()],
        );

        // Update parents and apply the changes.
        state
            .parents
            .insert(commit_a.id().clone(), vec![commit_c.id().clone()]);
        state
            .parents
            .insert(commit_b.id().clone(), vec![store.root_commit_id().clone()]);
        state
            .parents
            .insert(commit_f.id().clone(), vec![commit_a.id().clone()]);
        let rewritten = state.apply_changes(tx.repo_mut()).block_on().unwrap();
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
    fn test_apply_changes_abandon() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();
        let empty_tree = store.empty_merged_tree();

        // Move C onto A and abandon it:
        // d           d
        // |           |
        // C           C (abandoned)
        // |           |
        // B    =>   B |
        // |         |/
        // A         A
        // |         |
        // root      root
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
        let commit_c = create_commit(vec![commit_b.id().clone()]);
        let commit_d = create_commit(vec![commit_c.id().clone()]);

        let mut state = State::new(
            vec![commit_c.clone(), commit_b.clone(), commit_a.clone()],
            vec![commit_d.clone()],
        );

        // Update parents and action, then apply the changes.
        state
            .parents
            .insert(commit_c.id().clone(), vec![commit_a.id().clone()]);
        state.actions.insert(commit_c.id().clone(), Action::Abandon);
        let rewritten = state.apply_changes(tx.repo_mut()).block_on().unwrap();
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
