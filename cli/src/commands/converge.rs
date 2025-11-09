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

use std::collections::HashSet;
use std::io;

use clap_complete::ArgValueCompleter;
use indoc::indoc;
use itertools::Itertools as _;
use jj_lib::backend::ChangeId;
use jj_lib::backend::CommitId;
use jj_lib::backend::Signature;
use jj_lib::commit::Commit;
use jj_lib::conflict_labels::ConflictLabels;
use jj_lib::conflicts::ConflictMarkerStyle;
use jj_lib::conflicts::ConflictMaterializeOptions;
use jj_lib::conflicts::materialize_merge_result_to_bytes;
use jj_lib::converge::CommitsByChangeId;
use jj_lib::converge::ConvergeError;
use jj_lib::converge::ConvergedAttribute;
use jj_lib::converge::TruncatedEvolutionGraph;
use jj_lib::converge::apply_solution;
use jj_lib::converge::converge_change;
use jj_lib::converge::find_divergent_changes;
use jj_lib::files::FileMergeHunkLevel;
use jj_lib::merge::MergeBuilder;
use jj_lib::merge::SameChange;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::tree_merge::MergeOptions;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::WorkspaceCommandTransaction;
use crate::cli_util::short_change_hash;
use crate::cli_util::short_commit_hash;
use crate::command_error::CommandError;
use crate::command_error::CommandErrorKind;
use crate::complete;
use crate::description_util::TextEditor;
use crate::templater::TemplateRenderer;
use crate::ui::Ui;

/// Converge divergent changes
///
/// Attempts to resolve divergence by replacing two or more visible commits for
/// a given change with a single commit. `jj converge` evaluates the revset(s)
/// given by the `--revisions` arg (or the `revsets.converge` setting if none
/// are specified) and groups the resulting commits by change-id. Change-ids
/// with more than one commit are divergent.
///
/// If there is no divergence it returns successfully. If there is more than one
/// divergent change it prompts the user to choose one. The command then applies
/// heuristics to try to automatically come up with a good solution (i.e. a new
/// commit) to rewrite the divergent commits. If the heuristics are inconclusive
/// `jj converge` falls back to prompting the user. Use `--no-interactive` to
/// print a warning instead of prompting the user.
///
/// The user may be prompted for any of the following: to merge commit
/// descriptions, to choose parents for the solution, and/or (very rarely) to
/// choose a commit author.
///
/// When a solution is found, the new commit rewrites the divergent commits of
/// the specific change-id (but only those matching the revsets; if there are
/// other visible commits for the same change-id outside the revset, those will
/// remain and you will still have divergence, though it will be reduced). The
/// new commit becomes the successor of those commits it rewrites. Descendants
/// of the divergent commits are rebased onto the solution, and local bookmarks
/// pointing to any divergent commit are updated to point to the solution.
///
/// Note that there may be file conflicts in the solution whether or not there
/// were conflicts to begin with.
///
/// The modifications made by `jj converge` can be reviewed by `jj op show -p`.
/// You can inspect the change evolution with `jj evolog`. If not satisfied with
/// the result you can run `jj undo`.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct ConvergeArgs {
    /// The search space to look for divergent commits
    ///
    /// If no revisions are specified, this defaults to the `revsets.converge`
    /// setting.
    #[arg(long = "revision", short, value_name = "REVSETS", alias = "revisions")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    revisions: Vec<RevisionArg>,

    /// Do not prompt the user for help resolving divergence
    #[arg(long, conflicts_with = "_interactive")]
    no_interactive: bool,

    /// No-op flag to pair with --no-interactive
    #[arg(long, hide = true)]
    _interactive: bool,
}

// TODO: consider adding logic to deal with more than one divergent change-id in
// one invocation. Pick one, solve it, pick another one, solve it, etc.
// NOTE: currently we walk the operation history as far back as necessary when
// building the TruncatedEvolutionGraph. If this ever becomes a problem (because
// of a very deep fork in the op log), we could add a config setting to limit
// the walk and pretend that a "root" operation happened at that point.
pub(crate) async fn cmd_converge(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &ConvergeArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;
    let settings = workspace_command.settings();

    let search_space = {
        if args.revisions.is_empty() {
            let revset_string = settings.get_string("revsets.converge")?;
            workspace_command.parse_revset(ui, &RevisionArg::from(revset_string))?
        } else {
            workspace_command.parse_union_revsets(ui, &args.revisions)?
        }
    }
    .resolve()?;

    workspace_command
        .check_rewritable_expr(&search_space)
        .await?;

    let interactive = !args.no_interactive;

    let tx = workspace_command.start_transaction();

    // Find all divergent changes and choose one to converge.
    let divergent_changes = find_divergent_changes(tx.base_repo(), search_space).await?;
    if divergent_changes.is_empty() {
        writeln!(
            ui.status(),
            "No divergent changes found in the specified revset."
        )?;
        return Ok(());
    }
    report_divergent_changes(ui, &divergent_changes, &tx.commit_summary_template())?;
    let Some(change_id) = choose_change(ui, &divergent_changes, interactive)? else {
        return Err(CommandError::new(
            CommandErrorKind::User,
            "No change selected",
        ));
    };

    Converge::new(ui, tx, &divergent_changes, change_id.clone(), interactive)
        .await?
        .run()
        .await
}

struct Converge<'a> {
    ui: &'a Ui,
    tx: WorkspaceCommandTransaction<'a>,
    divergent_changes: &'a CommitsByChangeId,
    change_id: ChangeId,
    truncated_evolution_graph: TruncatedEvolutionGraph,
    interactive: bool,
}

impl<'a> Converge<'a> {
    async fn new(
        ui: &'a Ui,
        tx: WorkspaceCommandTransaction<'a>,
        divergent_changes: &'a CommitsByChangeId,
        change_id: ChangeId,
        interactive: bool,
    ) -> Result<Self, CommandError> {
        let divergent_commits = divergent_changes
            .get(&change_id)
            .expect("change_id is in divergent_changes")
            .values()
            .cloned()
            .collect_vec();
        let truncated_evolution_graph =
            TruncatedEvolutionGraph::new(tx.base_repo().clone(), divergent_commits).await?;
        Ok(Self {
            ui,
            tx,
            divergent_changes,
            change_id,
            truncated_evolution_graph,
            interactive,
        })
    }

    fn repo(&self) -> &ReadonlyRepo {
        self.truncated_evolution_graph.repo()
    }

    fn text_editor(&self) -> Result<TextEditor, CommandError> {
        Ok(self.tx.base_workspace_helper().text_editor()?)
    }

    async fn run(mut self) -> Result<(), CommandError> {
        // Initially we start with zero knowledge about what the solution should look
        // like.
        let author = None;
        let description = None;
        let parents = None;
        let tree = None;

        // Call the library function to attempt to converge the change automatically.
        let automatic_converge_result = converge_change(
            &self.truncated_evolution_graph,
            author,
            description,
            parents,
            tree,
        )
        .await?;

        // Now solve the author, description and parents, prompting the user for input
        // if necessary.
        let author = self.solve_author(automatic_converge_result.author)?;
        let description = self.solve_description(automatic_converge_result.description)?;
        let parents = self.solve_parents(automatic_converge_result.parents)?;

        if author.is_none() || description.is_none() || parents.is_none() {
            if author.is_none() {
                writeln!(self.ui.status(), "Could not determine which author to use.")?;
            }
            if description.is_none() {
                writeln!(
                    self.ui.status(),
                    "Could not determine which description to use."
                )?;
            }
            if parents.is_none() {
                writeln!(
                    self.ui.status(),
                    "Could not determine which parents to use."
                )?;
            }
            return Err(CommandError::new(
                CommandErrorKind::Internal,
                "Could not converge change",
            ));
        }

        // If we do not have a tree yet, call the converge_change library function
        // again, now that we have the author, description and parents.
        let tree = match automatic_converge_result.tree {
            Some(tree) => Ok(Some(tree)),
            None => {
                let converge_result = converge_change(
                    &self.truncated_evolution_graph,
                    author.clone(),
                    description.clone(),
                    parents.clone(),
                    None,
                )
                .await?;
                match converge_result.tree {
                    Some(tree) => Ok(Some(tree)),
                    None => Err(CommandError::new(
                        CommandErrorKind::Internal,
                        "Failed to converge tree",
                    )),
                }
            }
        }?;

        let author = author.unwrap();
        let description = description.unwrap();
        let parents = parents.unwrap();
        let tree = tree.unwrap();

        let (solution_commit, num_rebased) = apply_solution(
            author,
            description,
            parents,
            tree,
            self.change_id.clone(),
            self.truncated_evolution_graph.divergent_commit_ids(),
            self.tx.repo_mut(),
        )
        .await?;
        let transaction_description =
            self.make_transaction_description(solution_commit, num_rebased)?;
        self.tx.finish(self.ui, transaction_description).await?;
        Ok(())
    }

    fn solve_author(
        &self,
        automatic_convergence: ConvergedAttribute<Signature>,
    ) -> Result<Option<Signature>, CommandError> {
        self.generic_solver(automatic_convergence, Self::choose_author)
    }

    fn solve_parents(
        &self,
        automatic_convergence: ConvergedAttribute<Vec<CommitId>>,
    ) -> Result<Option<Vec<CommitId>>, CommandError> {
        self.generic_solver(automatic_convergence, Self::choose_parents)
    }

    fn solve_description(
        &self,
        automatic_convergence: ConvergedAttribute<String>,
    ) -> Result<Option<String>, CommandError> {
        self.generic_solver(automatic_convergence, Self::merge_description)
    }

    fn generic_solver<T, VF>(
        &self,
        automatic_convergence: ConvergedAttribute<T>,
        interactive_converge: VF,
    ) -> Result<Option<T>, CommandError>
    where
        T: PartialEq + Clone,
        VF: Fn(&Self, CommitId, HashSet<CommitId>) -> Result<T, CommandError>,
    {
        match automatic_convergence {
            ConvergedAttribute::Solved(value) => Ok(Some(value)),
            ConvergedAttribute::Unsolved {
                base_commit,
                excluded_divergent_commits,
            } => {
                if !self.interactive {
                    Ok(None)
                } else {
                    Ok(Some(interactive_converge(
                        self,
                        base_commit,
                        excluded_divergent_commits,
                    )?))
                }
            }
        }
    }

    fn choose_author(
        &self,
        _base_commit: CommitId,
        _excluded_divergent_commits: HashSet<CommitId>,
    ) -> Result<Signature, CommandError> {
        choose_helper(
            self.ui,
            self.truncated_evolution_graph.divergent_commits(),
            |commit| commit.author().clone(),
            indoc! {"
            Enter the index of the author:"},
        )
    }

    fn choose_parents(
        &self,
        _base_commit: CommitId,
        excluded_divergent_commits: HashSet<CommitId>,
    ) -> Result<Vec<CommitId>, CommandError> {
        let viable_commits = self
            .truncated_evolution_graph
            .divergent_commits()
            .iter()
            .filter(|commit| !excluded_divergent_commits.contains(commit.id()))
            .cloned()
            .collect_vec();
        // TODO: need to think about the best way to present the parent choices to the
        // user. There may be 10 divergent commits, 9 of them with parents {A, B} and 1
        // with parents {C, D}. Showing a list of 10 commit ids may not be the best way
        // to do this.
        choose_helper(
            self.ui,
            &viable_commits,
            |commit| commit.parent_ids().to_vec(),
            indoc! {"
            Enter the index of one of the divergent commits, its parent(s) will be the parents of the solution:"},
        )
    }

    // TODO: Run the user's configured merge tool.
    fn merge_description(
        &self,
        base_commit: CommitId,
        _excluded_divergent_commits: HashSet<CommitId>,
    ) -> Result<String, CommandError> {
        let base_commit = self.repo().store().get_commit(&base_commit)?;
        let conflicted_description = materialize_conflicted_description(
            self.truncated_evolution_graph.divergent_commits(),
            &base_commit,
        );
        let merge_in_text_editor = self.ui.prompt_yes_no(
            indoc! {"
            There are divergent descriptions. You can choose to merge them now in a
            text editor, or skip merging and use the conflicted description (with
            conflict markers). Do you want to merge them now?
            "},
            Some(true),
        )?;
        let description = if merge_in_text_editor {
            self.text_editor()?
                .edit_str(conflicted_description, Some(".jj-converge-description"))
                .map_err(|err| err.with_name("description"))?
        } else {
            conflicted_description
        };
        Ok(description)
    }

    fn make_transaction_description(
        &self,
        solution_commit: Commit,
        num_rebased: usize,
    ) -> Result<String, io::Error> {
        let change_id = solution_commit.change_id();
        let short_solution_id = short_commit_hash(solution_commit.id());
        let short_change_id = short_change_hash(change_id);
        let num_divergent_commits = self
            .divergent_changes
            .get(change_id)
            .map(|m| m.len())
            .unwrap_or(0);
        write!(
            self.ui.status(),
            "Successfully converged change: created commit {short_solution_id}."
        )?;
        if num_rebased > 0 {
            write!(self.ui.status(), "Rebased {num_rebased} descendants")?;
        }
        writeln!(self.ui.status())?;
        if self.divergent_changes.len() > 1 {
            writeln!(
                self.ui.hint_default(),
                "There are still {} divergent changes remaining in the specified revset, you can \
                 run this command again to converge another one.",
                self.divergent_changes.len() - 1
            )?;
        }
        let transaction_description =
            format!("converge {short_change_id} with {num_divergent_commits} predecessors");
        Ok(transaction_description)
    }
}

/// Prompts the user to choose a change-id to converge, if there are multiple
/// divergent change-ids.
fn choose_change<'a>(
    ui: &Ui,
    divergent_changes: &'a CommitsByChangeId,
    interactive: bool,
) -> Result<Option<&'a ChangeId>, ConvergeError> {
    match divergent_changes.len() {
        0 => return Ok(None),
        1 => return Ok(Some(divergent_changes.keys().next().unwrap())),
        _ => (),
    }
    // TODO: consider using heuristics to automatically choose a "good" change-id to
    // converge, falling back to prompting the user only if the heuristics are
    // inconclusive. This is specially important in non-interactive mode.
    if !interactive {
        return Ok(None);
    }

    let mut formatter = ui.stderr_formatter();
    let mut choices: Vec<String> = Default::default();
    let change_ids: Vec<&ChangeId> = divergent_changes.keys().collect();
    for (i, change_id) in change_ids.iter().enumerate() {
        // TODO: is there a better way to display the change-id? perhaps with
        // format_short_change_id?
        writeln!(formatter, "{}: {}", i + 1, short_change_hash(change_id))?;
        choices.push(format!("{}", i + 1));
    }
    writeln!(formatter, "q: abort")?;
    choices.push("q".to_string());
    drop(formatter);
    let index = ui.prompt_choice("Enter the index of the change to converge", &choices, None)?;
    if index >= change_ids.len() {
        Ok(None)
    } else {
        Ok(Some(change_ids[index]))
    }
}

fn choose_helper<T, VF>(
    ui: &Ui,
    divergent_commits: &[Commit],
    value_fn: VF,
    prompt: &str,
) -> Result<T, CommandError>
where
    T: PartialEq + Clone,
    VF: Fn(&Commit) -> T,
{
    let mut formatter = ui.stderr_formatter();
    let mut choices: Vec<String> = Default::default();

    {
        let first_commit_value = value_fn(&divergent_commits[0]);
        let mut all_same_value = true;
        for (i, commit) in divergent_commits.iter().enumerate() {
            // TODO: is there a better way to display the commit-id?
            writeln!(formatter, "{}: {}", i + 1, short_commit_hash(commit.id()))?;
            choices.push(format!("{}", i + 1));
            if value_fn(commit) != first_commit_value {
                all_same_value = false;
            }
        }
        if all_same_value {
            return Ok(first_commit_value);
        }
    }

    writeln!(formatter, "q: abort")?;
    choices.push("q".to_string());
    drop(formatter);
    let index = ui.prompt_choice(prompt, &choices, None)?;
    if index >= divergent_commits.len() {
        Err(CommandError::new(CommandErrorKind::User, "User aborted"))
    } else {
        Ok(value_fn(&divergent_commits[index]))
    }
}

fn materialize_conflicted_description(
    divergent_commits: &Vec<Commit>,
    base_commit: &Commit,
) -> String {
    // TODO: this probably needs more work. We should only show distinct
    // descriptions (i.e. we should dedup).
    let (description_merge, conflict_labels) = {
        let base = base_commit.description();
        let base_label = base_commit.conflict_label();
        let mut merge_builder = MergeBuilder::default();
        let mut labels = vec![];
        merge_builder.extend([base.to_string()]);
        labels.push(base_label.clone());
        for commit in divergent_commits {
            merge_builder.extend([commit.description().to_string(), base.to_string()]);
            labels.extend([commit.conflict_label(), base_label.clone()]);
        }
        (merge_builder.build(), ConflictLabels::from_vec(labels))
    };
    let options = ConflictMaterializeOptions {
        marker_style: ConflictMarkerStyle::Diff,
        marker_len: None,
        merge: MergeOptions {
            hunk_level: FileMergeHunkLevel::Line,
            same_change: SameChange::Accept,
        },
    };
    materialize_merge_result_to_bytes(&description_merge, &conflict_labels, &options).to_string()
}

fn report_divergent_changes(
    ui: &Ui,
    divergent_changes: &CommitsByChangeId,
    commit_summary_template: &TemplateRenderer<Commit>,
) -> io::Result<()> {
    let mut formatter = ui.stdout_formatter();
    writeln!(
        ui.status(),
        "Found {} divergent changes in the specified revset:",
        divergent_changes.len()
    )?;
    for (change_id, commits) in divergent_changes {
        writeln!(
            ui.status(),
            "- Change: {} with {} commits:",
            short_change_hash(change_id),
            commits.len(),
        )?;
        let it = commits.iter();
        for (_, commit) in it.take(10) {
            write!(formatter, "    ")?;
            commit_summary_template.format(commit, formatter.as_mut())?;
            writeln!(formatter)?;
        }
        if commits.len() > 10 {
            write!(formatter, "    ... and {} more", commits.len() - 10)?;
        }
        writeln!(formatter)?;
    }
    Ok(())
}
