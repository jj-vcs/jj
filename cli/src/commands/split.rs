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
use std::collections::HashMap;
use std::io::Write as _;

use clap_complete::ArgValueCandidates;
use clap_complete::ArgValueCompleter;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::matchers::Matcher;
use jj_lib::merge::Diff;
use jj_lib::object_id::ObjectId as _;
use jj_lib::rewrite::CommitWithSelection;
use jj_lib::rewrite::EmptyBehavior;
use jj_lib::rewrite::MoveCommitsLocation;
use jj_lib::rewrite::MoveCommitsTarget;
use jj_lib::rewrite::RebaseOptions;
use jj_lib::rewrite::RebasedCommit;
use jj_lib::rewrite::RewriteRefsOptions;
use jj_lib::rewrite::move_commits;
use pollster::FutureExt as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::DiffSelector;
use crate::cli_util::RevisionArg;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::WorkspaceCommandTransaction;
use crate::cli_util::compute_commit_location;
use crate::cli_util::print_unmatched_explicit_paths;
use crate::command_error::CommandError;
use crate::command_error::user_error_with_hint;
use crate::complete;
use crate::description_util::add_trailers;
use crate::description_util::description_template;
use crate::description_util::edit_description;
use crate::description_util::join_message_paragraphs;
use crate::ui::Ui;

/// Split a revision in two
///
/// Starts a [diff editor] on the changes in the revision. Edit the right side
/// of the diff until it has the content you want in the first commit. Once you
/// close the editor, your revision will be split into two commits.
///
/// [diff editor]:
///     https://docs.jj-vcs.dev/latest/config/#editing-diffs
///
/// By default, the selected changes stay in the original commit, and the
/// remaining changes go into a new child commit:
///
/// ```text
/// L                 L'
/// |                 |
/// K (split)   =>    K" (remaining)
/// |                 |
/// J                 K' (selected)
///                   |
///                   J
/// ```
///
/// With `--parallel/-p`, the two parts become sibling commits instead of
/// parent and child:
///
/// ```text
///                   L'
/// L                / \
/// |               K'  |  (selected)
/// K (split)  =>   |   K" (remaining)
/// |                \ /
/// J                 J
/// ```
///
/// With `-o`, `-A`, or `-B`, the selected changes are extracted into a new
/// commit at the specified location, while the remaining changes stay in place:
///
/// ```text
/// M                 M'
/// |                 |
/// L                 L'
/// |                 |
/// K (split)   =>    K' (remaining, stays here)
/// |                 |
/// J                 J'
///                   |
///                   K" (selected, inserted before J with -B J)
/// ```
///
/// If the change you split had a description, you will be asked to enter a
/// change description for each commit. If the change did not have a
/// description, the second commit will not get a description, and you will be
/// asked for a description only for the first commit.
///
/// Splitting an empty commit is not supported because the same effect can be
/// achieved with `jj new`.
#[derive(clap::Args, Clone, Debug)]
#[command(verbatim_doc_comment)]
pub(crate) struct SplitArgs {
    /// Interactively choose which parts to split
    ///
    /// This is the default if no filesets are provided.
    #[arg(long, short)]
    interactive: bool,

    /// Specify diff editor to be used (implies --interactive)
    #[arg(long, value_name = "NAME")]
    #[arg(add = ArgValueCandidates::new(complete::diff_editors))]
    tool: Option<String>,

    /// The revision to split
    #[arg(long, short, default_value = "@", value_name = "REVSET")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_mutable))]
    revision: RevisionArg,

    /// The revision(s) to rebase the selected changes onto (can be repeated to
    /// create a merge commit)
    ///
    /// Extracts the selected changes into a new commit based on the given
    /// revision(s). The remaining changes stay in the original commit's
    /// location.
    #[arg(
        long,
        visible_alias = "destination",
        short,
        visible_short_alias = 'd',
        conflicts_with = "parallel",
        value_name = "REVSETS"
    )]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    onto: Option<Vec<RevisionArg>>,

    /// The revision(s) to insert after (can be repeated to create a merge
    /// commit)
    ///
    /// Extracts the selected changes into a new commit inserted after the
    /// given revision(s). The remaining changes stay in the original commit's
    /// location.
    #[arg(
        long,
        short = 'A',
        visible_alias = "after",
        conflicts_with = "onto",
        conflicts_with = "parallel",
        value_name = "REVSETS"
    )]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    insert_after: Option<Vec<RevisionArg>>,

    /// The revision(s) to insert before (can be repeated to create a merge
    /// commit)
    ///
    /// Extracts the selected changes into a new commit inserted before the
    /// given revision(s). The remaining changes stay in the original commit's
    /// location.
    #[arg(
        long,
        short = 'B',
        visible_alias = "before",
        conflicts_with = "onto",
        conflicts_with = "parallel",
        value_name = "REVSETS"
    )]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_mutable))]
    insert_before: Option<Vec<RevisionArg>>,

    /// The change description to use (don't open editor)
    ///
    /// Sets the description for the first commit (the one containing the
    /// selected changes). The second commit keeps the original description.
    #[arg(long = "message", short, value_name = "MESSAGE")]
    message_paragraphs: Vec<String>,

    /// Open an editor to edit the change description
    ///
    /// Forces an editor to open when using `--message` to allow the
    /// message to be edited afterwards.
    #[arg(long)]
    editor: bool,

    /// Split the revision into two parallel revisions instead of a parent and
    /// child
    #[arg(long, short)]
    parallel: bool,

    /// Files matching any of these filesets are put in the selected changes
    #[arg(value_name = "FILESETS", value_hint = clap::ValueHint::AnyPath)]
    #[arg(add = ArgValueCompleter::new(complete::modified_revision_files))]
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
        let repo = workspace_command.repo();
        let fileset_expression = workspace_command.parse_file_patterns(ui, &self.paths)?;
        let matcher = fileset_expression.to_matcher();
        let diff_selector = workspace_command.diff_selector(
            ui,
            self.tool.as_deref(),
            self.interactive || self.paths.is_empty(),
        )?;
        let use_move_flags =
            self.onto.is_some() || self.insert_after.is_some() || self.insert_before.is_some();
        let (new_parent_ids, new_child_ids) = if use_move_flags {
            compute_commit_location(
                ui,
                workspace_command,
                self.onto.as_deref(),
                self.insert_after.as_deref(),
                self.insert_before.as_deref(),
                "split-out commit",
            )?
        } else {
            Default::default()
        };

        print_unmatched_explicit_paths(
            ui,
            workspace_command,
            &fileset_expression,
            [
                // We check the parent commit to account for deleted files.
                &target_commit.parent_tree(repo.as_ref())?,
                &target_commit.tree(),
            ],
        )?;

        Ok(ResolvedSplitArgs {
            target_commit,
            matcher,
            diff_selector,
            parallel: self.parallel,
            use_move_flags,
            new_parent_ids,
            new_child_ids,
        })
    }
}

struct ResolvedSplitArgs {
    target_commit: Commit,
    matcher: Box<dyn Matcher>,
    diff_selector: DiffSelector,
    parallel: bool,
    use_move_flags: bool,
    new_parent_ids: Vec<CommitId>,
    new_child_ids: Vec<CommitId>,
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
        use_move_flags,
        new_parent_ids,
        new_child_ids,
    } = args.resolve(ui, &workspace_command)?;
    let text_editor = workspace_command.text_editor()?;
    let mut tx = workspace_command.start_transaction();

    // Prompt the user to select the changes they want for the first commit.
    let target = select_diff(ui, &tx, &target_commit, &matcher, &diff_selector)?;

    // Create the first commit, which includes the changes selected by the user.
    let first_commit = {
        let mut commit_builder = tx.repo_mut().rewrite_commit(&target.commit).detach();
        commit_builder.set_tree(target.selected_tree.clone());
        if use_move_flags {
            commit_builder.clear_rewrite_source();
            // Generate a new change id so that the commit being split doesn't
            // become divergent.
            commit_builder.generate_new_change_id();
        }
        let description = if args.message_paragraphs.is_empty() {
            commit_builder.description().to_owned()
        } else {
            join_message_paragraphs(&args.message_paragraphs)
        };
        let description = if !description.is_empty() || args.editor {
            commit_builder.set_description(description);
            add_trailers(ui, &tx, &commit_builder)?
        } else {
            description
        };
        let description = if args.editor || args.message_paragraphs.is_empty() {
            commit_builder.set_description(description);
            let temp_commit = commit_builder.write_hidden()?;
            let intro = "Enter a description for the selected changes.";
            let template = description_template(ui, &tx, intro, &temp_commit)?;
            edit_description(&text_editor, &template)?
        } else {
            description
        };
        commit_builder.set_description(description);
        commit_builder.write(tx.repo_mut())?
    };

    // Create the second commit, which includes everything the user didn't
    // select.
    let second_commit = {
        let target_tree = target.commit.tree();
        let new_tree = if parallel {
            // Merge the original commit tree with its parent using the tree
            // containing the user selected changes as the base for the merge.
            // This results in a tree with the changes the user didn't select.
            target_tree
                .merge_unlabeled(target.selected_tree.clone(), target.parent_tree.clone())
                .block_on()?
        } else {
            target_tree
        };
        let parents = if parallel {
            target.commit.parent_ids().to_vec()
        } else {
            vec![first_commit.id().clone()]
        };
        let mut commit_builder = tx.repo_mut().rewrite_commit(&target.commit).detach();
        commit_builder.set_parents(parents).set_tree(new_tree);
        let mut show_editor = args.editor;
        if !use_move_flags {
            commit_builder.clear_rewrite_source();
            // Generate a new change id so that the commit being split doesn't
            // become divergent.
            commit_builder.generate_new_change_id();
        }
        let description = if target.commit.description().is_empty() {
            // If there was no description before, don't ask for one for the
            // second commit.
            "".to_string()
        } else {
            show_editor = show_editor || args.message_paragraphs.is_empty();
            // Just keep the original message unchanged
            commit_builder.description().to_owned()
        };
        let description = if show_editor {
            let new_description = add_trailers(ui, &tx, &commit_builder)?;
            commit_builder.set_description(new_description);
            let temp_commit = commit_builder.write_hidden()?;
            let intro = "Enter a description for the remaining changes.";
            let template = description_template(ui, &tx, intro, &temp_commit)?;
            edit_description(&text_editor, &template)?
        } else {
            description
        };
        commit_builder.set_description(description);
        commit_builder.write(tx.repo_mut())?
    };

    let (first_commit, second_commit, num_rebased) = if use_move_flags {
        move_first_commit(
            &mut tx,
            &target,
            first_commit,
            second_commit,
            new_parent_ids,
            new_child_ids,
        )?
    } else {
        rewrite_descendants(&mut tx, &target, first_commit, second_commit, parallel)?
    };
    if let Some(mut formatter) = ui.status_formatter() {
        if num_rebased > 0 {
            writeln!(formatter, "Rebased {num_rebased} descendant commits")?;
        }
        write!(formatter, "Selected changes : ")?;
        tx.write_commit_summary(formatter.as_mut(), &first_commit)?;
        write!(formatter, "\nRemaining changes: ")?;
        tx.write_commit_summary(formatter.as_mut(), &second_commit)?;
        writeln!(formatter)?;
    }
    tx.finish(ui, format!("split commit {}", target.commit.id().hex()))?;
    Ok(())
}

fn move_first_commit(
    tx: &mut WorkspaceCommandTransaction,
    target: &CommitWithSelection,
    mut first_commit: Commit,
    mut second_commit: Commit,
    new_parent_ids: Vec<CommitId>,
    new_child_ids: Vec<CommitId>,
) -> Result<(Commit, Commit, usize), CommandError> {
    let mut rewritten_commits: HashMap<CommitId, CommitId> = HashMap::new();
    rewritten_commits.insert(target.commit.id().clone(), second_commit.id().clone());
    tx.repo_mut()
        .transform_descendants(vec![target.commit.id().clone()], async |rewriter| {
            let old_commit_id = rewriter.old_commit().id().clone();
            let new_commit = rewriter.rebase().await?.write()?;
            rewritten_commits.insert(old_commit_id, new_commit.id().clone());
            Ok(())
        })?;

    let new_parent_ids: Vec<_> = new_parent_ids
        .iter()
        .map(|commit_id| rewritten_commits.get(commit_id).unwrap_or(commit_id))
        .cloned()
        .collect();
    let new_child_ids: Vec<_> = new_child_ids
        .iter()
        .map(|commit_id| rewritten_commits.get(commit_id).unwrap_or(commit_id))
        .cloned()
        .collect();
    let stats = move_commits(
        tx.repo_mut(),
        &MoveCommitsLocation {
            new_parent_ids,
            new_child_ids,
            target: MoveCommitsTarget::Commits(vec![first_commit.id().clone()]),
        },
        &RebaseOptions {
            empty: EmptyBehavior::Keep,
            rewrite_refs: RewriteRefsOptions {
                delete_abandoned_bookmarks: false,
            },
            simplify_ancestor_merge: false,
        },
    )?;

    // 1 for the transformation of the original commit to the second commit
    // that was inserted in rewritten_commits
    let mut num_new_rebased = 1;
    if let Some(RebasedCommit::Rewritten(commit)) = stats.rebased_commits.get(first_commit.id()) {
        first_commit = commit.clone();
        num_new_rebased += 1;
    }
    if let Some(RebasedCommit::Rewritten(commit)) = stats.rebased_commits.get(second_commit.id()) {
        second_commit = commit.clone();
    }

    let num_rebased = rewritten_commits.len() + stats.rebased_commits.len()
        // don't count the commit generated by the split in the rebased commits
        - num_new_rebased
        // only count once a commit that may have been rewritten twice in the process
        - rewritten_commits
            .iter()
            .filter(|(_, rewritten)| stats.rebased_commits.contains_key(rewritten))
            .count();

    Ok((first_commit, second_commit, num_rebased))
}

fn rewrite_descendants(
    tx: &mut WorkspaceCommandTransaction,
    target: &CommitWithSelection,
    first_commit: Commit,
    second_commit: Commit,
    parallel: bool,
) -> Result<(Commit, Commit, usize), CommandError> {
    let legacy_bookmark_behavior = tx.settings().get_bool("split.legacy-bookmark-behavior")?;
    if legacy_bookmark_behavior {
        // Mark the commit being split as rewritten to the second commit. This
        // moves any bookmarks pointing to the target commit to the second
        // commit.
        tx.repo_mut()
            .set_rewritten_commit(target.commit.id().clone(), second_commit.id().clone());
    }
    let mut num_rebased = 0;
    tx.repo_mut().transform_descendants(
        vec![target.commit.id().clone()],
        async |mut rewriter| {
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
            rewriter.rebase().await?.write()?;
            Ok(())
        },
    )?;
    // Move the working copy commit (@) to the second commit for any workspaces
    // where the target commit is the working copy commit.
    for (name, working_copy_commit) in tx.base_repo().clone().view().wc_commit_ids() {
        if working_copy_commit == target.commit.id() {
            tx.repo_mut().edit(name.clone(), &second_commit)?;
        }
    }

    Ok((first_commit, second_commit, num_rebased))
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

Adjust the right side until it shows the contents you want to split into the
new commit.
The changes that are not selected will replace the original commit.
",
            tx.format_commit_summary(target_commit)
        )
    };
    let parent_tree = target_commit.parent_tree(tx.repo())?;
    let selected_tree = diff_selector.select(
        Diff::new(&parent_tree, &target_commit.tree()),
        matcher,
        format_instructions,
    )?;
    let selection = CommitWithSelection {
        commit: target_commit.clone(),
        selected_tree,
        parent_tree,
    };
    if selection.is_full_selection() {
        writeln!(
            ui.warning_default(),
            "All changes have been selected, so the original revision will become empty"
        )?;
    } else if selection.is_empty_selection() {
        writeln!(
            ui.warning_default(),
            "No changes have been selected, so the new revision will be empty"
        )?;
    }

    Ok(selection)
}
