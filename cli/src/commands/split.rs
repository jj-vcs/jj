// Copyright 2020 The Jujutsu Authors
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
use std::io::Write as _;

use clap_complete::ArgValueCandidates;
use clap_complete::ArgValueCompleter;
use jj_lib::commit::Commit;
use jj_lib::matchers::Matcher;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo::Repo as _;
use jj_lib::rewrite::CommitWithSelection;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::DiffSelector;
use crate::cli_util::RevisionArg;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::WorkspaceCommandTransaction;
use crate::command_error::user_error_with_hint;
use crate::command_error::CommandError;
use crate::complete;
use crate::description_util::description_template;
use crate::description_util::edit_description;
use crate::ui::Ui;

/// Split a revision in two
///
/// Starts a [diff editor] on the changes in the revision. Edit the right side
/// of the diff until it has the content you want in the first revision. Once
/// you close the editor, your edited content will replace the previous
/// revision. The remaining changes will be put in a new revision on top.
///
/// [diff editor]:
///     https://jj-vcs.github.io/jj/latest/config/#editing-diffs
///
/// If the change you split had a description, you will be asked to enter a
/// change description for each commit. If the change did not have a
/// description, the second part will not get a description, and you will be
/// asked for a description only for the first part.
///
/// Splitting an empty commit is not supported because the same effect can be
/// achieved with `jj new`.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct SplitArgs {
    /// Interactively choose which parts to split
    ///
    /// This is the default if no filesets are provided.
    #[arg(long, short)]
    interactive: bool,
    /// Specify diff editor to be used (implies --interactive)
    #[arg(long, value_name = "NAME")]
    tool: Option<String>,
    /// The revision to split
    #[arg(
        long, short,
        default_value = "@",
        value_name = "REVSET",
        add = ArgValueCandidates::new(complete::mutable_revisions)
    )]
    revision: RevisionArg,
    /// Split the revision into two parallel revisions instead of a parent and
    /// child
    #[arg(long, short)]
    parallel: bool,
    /// Files matching any of these filesets are put in the first commit
    #[arg(
        value_name = "FILESETS",
        value_hint = clap::ValueHint::AnyPath,
        add = ArgValueCompleter::new(complete::modified_revision_files),
    )]
    paths: Vec<String>,
}

impl SplitArgs {
    /// Resolves the raw SplitArgs into the components necessary to run the
    /// command. Returns an error if the command cannot proceed.
    fn resolve(
        &self,
        ui: &Ui,
        workspace_command: &WorkspaceCommandHelper,
    ) -> Result<ResolvedSplitArgs, CommandError> {
        let target_commit = workspace_command.resolve_single_rev(ui, &self.revision)?;
        if target_commit.is_empty(workspace_command.repo().as_ref())? {
            return Err(user_error_with_hint(
                format!(
                    "Refusing to split empty commit {}.",
                    target_commit.id().hex()
                ),
                "Use `jj new` if you want to create another empty commit.",
            ));
        }
        workspace_command.check_rewritable([target_commit.id()])?;
        let matcher = workspace_command
            .parse_file_patterns(ui, &self.paths)?
            .to_matcher();
        let diff_selector = workspace_command.diff_selector(
            ui,
            self.tool.as_deref(),
            self.interactive || self.paths.is_empty(),
        )?;
        Ok(ResolvedSplitArgs {
            target_commit,
            matcher,
            diff_selector,
            parallel: self.parallel,
        })
    }
}

struct ResolvedSplitArgs {
    target_commit: Commit,
    matcher: Box<dyn Matcher>,
    diff_selector: DiffSelector,
    parallel: bool,
}

#[instrument(skip_all)]
pub(crate) fn cmd_split(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &SplitArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let ResolvedSplitArgs {
        target_commit,
        matcher,
        diff_selector,
        parallel,
    } = args.resolve(ui, &workspace_command)?;
    let text_editor = workspace_command.text_editor()?;
    let mut tx = workspace_command.start_transaction();

    // Prompt the user to select the changes they want for the first commit.
    let target = select_diff(ui, &tx, &target_commit, &matcher, &diff_selector)?;

    // Create the first commit, which includes the changes selected by the user.
    let first_commit = {
        let mut commit_builder = tx.repo_mut().rewrite_commit(&target.commit).detach();
        commit_builder.set_tree_id(target.selected_tree.id());
        if commit_builder.description().is_empty() {
            commit_builder.set_description(tx.settings().get_string("ui.default-description")?);
        }
        let temp_commit = commit_builder.write_hidden()?;
        let template = description_template(
            ui,
            &tx,
            "Enter a description for the first commit.",
            &temp_commit,
        )?;
        let description = edit_description(&text_editor, &template)?;
        commit_builder.set_description(description);
        commit_builder.write(tx.repo_mut())?
    };

    // Create the second commit, which includes everything the user didn't
    // select.
    let second_commit = {
        let target_tree = target.commit.tree()?;
        let new_tree = if args.parallel {
            // Merge the original commit tree with its parent using the tree
            // containing the user selected changes as the base for the merge.
            // This results in a tree with the changes the user didn't select.
            target_tree.merge(&target.selected_tree, &target.parent_tree)?
        } else {
            target_tree
        };
        let parents = if parallel {
            target.commit.parent_ids().to_vec()
        } else {
            vec![first_commit.id().clone()]
        };
        let mut commit_builder = tx.repo_mut().rewrite_commit(&target.commit).detach();
        commit_builder
            .set_parents(parents)
            .set_tree_id(new_tree.id())
            // Generate a new change id so that the commit being split doesn't
            // become divergent.
            .generate_new_change_id();
        let description = if target.commit.description().is_empty() {
            // If there was no description before, don't ask for one for the
            // second commit.
            "".to_string()
        } else {
            let temp_commit = commit_builder.write_hidden()?;
            let template = description_template(
                ui,
                &tx,
                "Enter a description for the second commit.",
                &temp_commit,
            )?;
            edit_description(&text_editor, &template)?
        };
        commit_builder.set_description(description);
        commit_builder.write(tx.repo_mut())?
    };

    let legacy_bookmark_behavior = tx.settings().get_bool("split.legacy-bookmark-behavior")?;
    if legacy_bookmark_behavior {
        // Mark the commit being split as rewritten to the second commit. This
        // moves any bookmarks pointing to the target commit to the second
        // commit.
        tx.repo_mut()
            .set_rewritten_commit(target.commit.id().clone(), second_commit.id().clone());
    }
    let mut num_rebased = 0;
    tx.repo_mut()
        .transform_descendants(vec![target.commit.id().clone()], |mut rewriter| {
            num_rebased += 1;
            if parallel && legacy_bookmark_behavior {
                // The old_parent is the second commit due to the rewrite above.
                rewriter
                    .replace_parent(second_commit.id(), [first_commit.id(), second_commit.id()]);
            } else if parallel {
                rewriter.replace_parent(first_commit.id(), [first_commit.id(), second_commit.id()]);
            } else {
                rewriter.replace_parent(first_commit.id(), [second_commit.id()]);
            }
            rewriter.rebase()?.write()?;
            Ok(())
        })?;
    // Move the working copy commit (@) to the second commit for any workspaces
    // where the target commit is the working copy commit.
    for (name, working_copy_commit) in tx.base_repo().clone().view().wc_commit_ids() {
        if working_copy_commit == target.commit.id() {
            tx.repo_mut().edit(name.clone(), &second_commit)?;
        }
    }

    if let Some(mut formatter) = ui.status_formatter() {
        if num_rebased > 0 {
            writeln!(formatter, "Rebased {num_rebased} descendant commits")?;
        }
        write!(formatter, "First part: ")?;
        tx.write_commit_summary(formatter.as_mut(), &first_commit)?;
        write!(formatter, "\nSecond part: ")?;
        tx.write_commit_summary(formatter.as_mut(), &second_commit)?;
        writeln!(formatter)?;
    }
    tx.finish(ui, format!("split commit {}", target.commit.id().hex()))?;
    Ok(())
}

/// Prompts the user to select the content they want in the first commit and
/// returns the target commit and the tree corresponding to the selection.
fn select_diff(
    ui: &Ui,
    tx: &WorkspaceCommandTransaction,
    target_commit: &Commit,
    matcher: &dyn Matcher,
    diff_selector: &DiffSelector,
) -> Result<CommitWithSelection, CommandError> {
    let format_instructions = || {
        format!(
            "\
You are splitting a commit into two: {}

The diff initially shows the changes in the commit you're splitting.

Adjust the right side until it shows the contents you want for the first commit.
The remainder will be in the second commit.
",
            tx.format_commit_summary(target_commit)
        )
    };
    let parent_tree = target_commit.parent_tree(tx.repo())?;
    let selected_tree_id = diff_selector.select(
        &parent_tree,
        &target_commit.tree()?,
        matcher,
        format_instructions,
    )?;
    let selection = CommitWithSelection {
        commit: target_commit.clone(),
        selected_tree: tx.repo().store().get_root_tree(&selected_tree_id)?,
        parent_tree,
    };
    if selection.is_full_selection() {
        writeln!(
            ui.warning_default(),
            "All changes have been selected, so the second commit will be empty"
        )?;
    } else if selection.is_empty_selection() {
        writeln!(
            ui.warning_default(),
            "No changes have been selected, so the first commit will be empty"
        )?;
    }

    Ok(selection)
}
