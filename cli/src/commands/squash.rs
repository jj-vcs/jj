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
use std::iter::once;

use clap_complete::ArgValueCandidates;
use clap_complete::ArgValueCompleter;
use indoc::formatdoc;
use itertools::Itertools as _;
use jj_lib::commit::Commit;
use jj_lib::commit::CommitIteratorExt as _;
use jj_lib::matchers::Matcher;
use jj_lib::merge::Diff;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo::Repo as _;
use jj_lib::rewrite;
use jj_lib::rewrite::CommitWithSelection;
use jj_lib::rewrite::merge_commit_trees;
use pollster::FutureExt as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::DiffSelector;
use crate::cli_util::RevisionArg;
use crate::cli_util::WorkspaceCommandTransaction;
use crate::cli_util::compute_commit_location;
use crate::cli_util::print_unmatched_explicit_paths;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::command_error::user_error_with_hint;
use crate::complete;
use crate::description_util::add_trailers;
use crate::description_util::combine_messages_for_editing;
use crate::description_util::description_template;
use crate::description_util::edit_description;
use crate::description_util::join_message_paragraphs;
use crate::description_util::try_combine_messages;
use crate::ui::Ui;

/// Move changes from a revision into another revision
///
/// Without any options, moves the changes from the working-copy revision to the
/// parent revision.
///
/// With the `-r` option, moves the changes from the specified revision to the
/// parent revision. Fails if there are several parent revisions (i.e., the
/// given revision is a merge).
///
/// With the `--from` and/or `--into` options, moves changes from/to the given
/// revisions. If either is left out, it defaults to the working-copy commit.
/// For example, `jj squash --into @--` moves changes from the working-copy
/// commit to the grandparent.
///
/// If, after moving changes out, the source revision is empty compared to its
/// parent(s), and `--keep-emptied` is not set, it will be abandoned. Without
/// `--interactive` or paths, the source revision will always be empty.
///
/// If the source was abandoned and both the source and destination had a
/// non-empty description, you will be asked for the combined description. If
/// either was empty, then the other one will be used.
///
/// If a working-copy commit gets abandoned, it will be given a new, empty
/// commit. This is true in general; it is not specific to this command.
///
/// EXPERIMENTAL FEATURES
///
/// An alternative squashing UI is available via the `-o`, `-A`, and `-B`
/// options. Using any of these options creates a new commit. They can be used
/// together with one or more `--from` options (if no `--from` is specified,
/// `--from @` is assumed).
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct SquashArgs {
    /// Revision to squash into its parent (default: @). Incompatible with the
    /// experimental `-o`/`-A`/`-B` options.
    #[arg(
        long,
        short,
        value_name = "REVSET",
        add = ArgValueCompleter::new(complete::revset_expression_mutable),
    )]
    revision: Option<RevisionArg>,

    /// Revision(s) to squash from (default: @)
    #[arg(
        long, short,
        conflicts_with = "revision",
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_mutable),
    )]
    from: Vec<RevisionArg>,

    /// Revision to squash into (default: @)
    #[arg(
        long, short = 't',
        conflicts_with = "revision",
        visible_alias = "to",
        value_name = "REVSET",
        add = ArgValueCompleter::new(complete::revset_expression_mutable),
    )]
    into: Option<RevisionArg>,

    /// (Experimental) The revision(s) to use as parent for the new commit (can
    /// be repeated to create a merge commit)
    #[arg(
        long,
        alias = "destination",
        short,
        short_alias = 'd',
        conflicts_with = "into",
        conflicts_with = "revision",
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_all),
    )]
    onto: Option<Vec<RevisionArg>>,

    /// (Experimental) The revision(s) to insert the new commit after (can be
    /// repeated to create a merge commit)
    #[arg(
        long,
        short = 'A',
        visible_alias = "after",
        conflicts_with = "onto",
        conflicts_with = "into",
        conflicts_with = "revision",
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_all),
    )]
    insert_after: Option<Vec<RevisionArg>>,

    /// (Experimental) The revision(s) to insert the new commit before (can be
    /// repeated to create a merge commit)
    #[arg(
        long,
        short = 'B',
        visible_alias = "before",
        conflicts_with = "onto",
        conflicts_with = "into",
        conflicts_with = "revision",
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_mutable),
    )]
    insert_before: Option<Vec<RevisionArg>>,

    /// The description to use for squashed revision (don't open editor)
    #[arg(long = "message", short, value_name = "MESSAGE")]
    message_paragraphs: Vec<String>,

    /// Use the description of the destination revision and discard the
    /// description(s) of the source revision(s)
    #[arg(long, short, conflicts_with = "message_paragraphs")]
    use_destination_message: bool,

    /// Open an editor to edit the change description
    ///
    /// Forces an editor to open when using `--message` to allow the
    /// message to be edited afterwards.
    #[arg(long)]
    editor: bool,

    /// Interactively choose which parts to squash
    #[arg(long, short)]
    interactive: bool,

    /// Specify diff editor to be used (implies --interactive)
    #[arg(
        long,
        value_name = "NAME",
        add = ArgValueCandidates::new(complete::diff_editors),
    )]
    tool: Option<String>,

    /// Move only changes to these paths (instead of all paths)
    #[arg(
        value_name = "FILESETS",
        value_hint = clap::ValueHint::AnyPath,
        add = ArgValueCompleter::new(complete::squash_revision_files),
    )]
    paths: Vec<String>,

    /// The source revision will not be abandoned
    #[arg(long, short)]
    keep_emptied: bool,
}

#[instrument(skip_all)]
pub(crate) fn cmd_squash(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &SquashArgs,
) -> Result<(), CommandError> {
    let insert_destination_commit =
        args.onto.is_some() || args.insert_after.is_some() || args.insert_before.is_some();

    let mut workspace_command = command.workspace_helper(ui)?;

    let mut sources: Vec<Commit>;
    let pre_existing_destination;

    if !args.from.is_empty() || args.into.is_some() || insert_destination_commit {
        sources = if args.from.is_empty() {
            workspace_command.parse_revset(ui, &RevisionArg::AT)?
        } else {
            workspace_command.parse_union_revsets(ui, &args.from)?
        }
        .evaluate_to_commits()?
        .try_collect()?;
        if insert_destination_commit {
            pre_existing_destination = None;
        } else {
            let destination = workspace_command
                .resolve_single_rev(ui, args.into.as_ref().unwrap_or(&RevisionArg::AT))?;
            // remove the destination from the sources
            sources.retain(|source| source.id() != destination.id());
            pre_existing_destination = Some(destination);
        }
        // Reverse the set so we apply the oldest commits first. It shouldn't affect the
        // result, but it avoids creating transient conflicts and is therefore probably
        // a little faster.
        sources.reverse();
    } else {
        let source = workspace_command
            .resolve_single_rev(ui, args.revision.as_ref().unwrap_or(&RevisionArg::AT))?;
        let mut parents: Vec<_> = source.parents().try_collect()?;
        if parents.len() != 1 {
            return Err(user_error_with_hint(
                "Cannot squash merge commits without a specified destination",
                "Use `--into` to specify which parent to squash into",
            ));
        }
        sources = vec![source];
        pre_existing_destination = Some(parents.pop().unwrap());
    };

    workspace_command.check_rewritable(sources.iter().chain(&pre_existing_destination).ids())?;

    // prepare the tx description before possibly rebasing the source commits
    let source_ids: Vec<_> = sources.iter().ids().collect();
    let tx_description = if let Some(destination) = &pre_existing_destination {
        format!("squash commits into {}", destination.id().hex())
    } else {
        match &source_ids[..] {
            [] => format!("squash {} commits", source_ids.len()),
            [id] => format!("squash commit {}", id.hex()),
            [first, others @ ..] => {
                format!("squash commit {} and {} more", first.hex(), others.len())
            }
        }
    };

    let mut tx = workspace_command.start_transaction();
    let mut num_rebased = 0;
    let destination = if let Some(commit) = pre_existing_destination {
        commit
    } else {
        // create the new destination commit
        let (parent_ids, child_ids) = compute_commit_location(
            ui,
            tx.base_workspace_helper(),
            args.onto.as_deref(),
            args.insert_after.as_deref(),
            args.insert_before.as_deref(),
            "squashed commit",
        )?;
        let parent_commits: Vec<_> = parent_ids
            .iter()
            .map(|commit_id| {
                tx.base_workspace_helper()
                    .repo()
                    .store()
                    .get_commit(commit_id)
            })
            .try_collect()?;
        let merged_tree = merge_commit_trees(tx.repo(), &parent_commits).block_on()?;
        let commit = tx
            .repo_mut()
            .new_commit(parent_ids.clone(), merged_tree)
            .write()?;
        let mut rewritten = HashMap::new();
        tx.repo_mut()
            .transform_descendants(child_ids.clone(), async |mut rewriter| {
                let old_commit_id = rewriter.old_commit().id().clone();
                for parent_id in &parent_ids {
                    rewriter.replace_parent(parent_id, [commit.id()]);
                }
                let new_parents = rewriter.new_parents();
                if child_ids.contains(&old_commit_id) && !new_parents.contains(commit.id()) {
                    rewriter.set_new_parents(
                        new_parents
                            .iter()
                            .cloned()
                            .chain(once(commit.id().clone()))
                            .collect(),
                    );
                }
                let new_commit = rewriter.rebase().await?.write()?;
                rewritten.insert(old_commit_id, new_commit);
                num_rebased += 1;
                Ok(())
            })?;
        for source in &mut *sources {
            if let Some(rewritten_source) = rewritten.remove(source.id()) {
                *source = rewritten_source;
            }
        }
        commit
    };

    let fileset_expression = tx
        .base_workspace_helper()
        .parse_file_patterns(ui, &args.paths)?;
    let matcher = fileset_expression.to_matcher();
    let diff_selector =
        tx.base_workspace_helper()
            .diff_selector(ui, args.tool.as_deref(), args.interactive)?;
    let text_editor = tx.base_workspace_helper().text_editor()?;
    let squashed_description = SquashedDescription::from_args(args);

    let source_commits = select_diff(&tx, &sources, &destination, &matcher, &diff_selector)?;

    print_unmatched_explicit_paths(
        ui,
        tx.base_workspace_helper(),
        &fileset_expression,
        source_commits.iter().map(|commit| &commit.selected_tree),
    )?;

    if let Some(squashed) = rewrite::squash_commits(
        tx.repo_mut(),
        &source_commits,
        &destination,
        args.keep_emptied,
    )? {
        let mut commit_builder = squashed.commit_builder.detach();
        let single_description = match squashed_description {
            SquashedDescription::Exact(description) => Some(description),
            SquashedDescription::UseDestination => Some(destination.description().to_owned()),
            SquashedDescription::Combine => {
                let abandoned_commits = &squashed.abandoned_commits;
                try_combine_messages(abandoned_commits, &destination)
            }
        };
        let description = if let Some(description) = single_description {
            if description.is_empty() && !args.editor {
                description
            } else {
                commit_builder.set_description(&description);
                let description_with_trailers = add_trailers(ui, &tx, &commit_builder)?;
                if args.editor {
                    commit_builder.set_description(&description_with_trailers);
                    let temp_commit = commit_builder.write_hidden()?;
                    let intro = "";
                    let template = description_template(ui, &tx, intro, &temp_commit)?;
                    edit_description(&text_editor, &template)?
                } else {
                    description_with_trailers
                }
            }
        } else {
            // edit combined
            let abandoned_commits = &squashed.abandoned_commits;
            let combined = combine_messages_for_editing(
                ui,
                &tx,
                abandoned_commits,
                (!insert_destination_commit).then_some(&destination),
                &commit_builder,
            )?;
            // It's weird that commit.description() contains "JJ: " lines, but works.
            commit_builder.set_description(combined);
            let temp_commit = commit_builder.write_hidden()?;
            let intro = "Enter a description for the combined commit.";
            let template = description_template(ui, &tx, intro, &temp_commit)?;
            edit_description(&text_editor, &template)?
        };
        commit_builder.set_description(description);
        if insert_destination_commit {
            // forget about the intermediate commit
            commit_builder.set_predecessors(
                commit_builder
                    .predecessors()
                    .iter()
                    .filter(|p| p != &destination.id())
                    .cloned()
                    .collect(),
            );
        }
        let commit = commit_builder.write(tx.repo_mut())?;
        let num_rebased = tx.repo_mut().rebase_descendants()?;
        if let Some(mut formatter) = ui.status_formatter() {
            if insert_destination_commit {
                write!(formatter, "Created new commit ")?;
                tx.write_commit_summary(formatter.as_mut(), &commit)?;
                writeln!(formatter)?;
            }
            if num_rebased > 0 {
                writeln!(formatter, "Rebased {num_rebased} descendant commits")?;
            }
        }
    } else {
        if diff_selector.is_interactive() {
            return Err(user_error("No changes selected"));
        }

        if let Some(mut formatter) = ui.status_formatter() {
            if insert_destination_commit {
                write!(formatter, "Created new commit ")?;
                tx.write_commit_summary(formatter.as_mut(), &destination)?;
                writeln!(formatter)?;
            }
            if num_rebased > 0 {
                writeln!(formatter, "Rebased {num_rebased} descendant commits")?;
            }
        }

        if let [only_path] = &*args.paths {
            let no_rev_arg = args.revision.is_none() && args.from.is_empty() && args.into.is_none();
            if no_rev_arg
                && tx
                    .base_workspace_helper()
                    .parse_revset(ui, &RevisionArg::from(only_path.to_owned()))
                    .is_ok()
            {
                writeln!(
                    ui.warning_default(),
                    "The argument {only_path:?} is being interpreted as a fileset expression. To \
                     specify a revset, pass -r {only_path:?} instead."
                )?;
            }
        }
    }
    tx.finish(ui, tx_description)?;
    Ok(())
}

enum SquashedDescription {
    // Use this exact description.
    Exact(String),
    // Use the destination's description and discard the descriptions of the
    // source revisions.
    UseDestination,
    // Combine the descriptions of the source and destination revisions.
    Combine,
}

impl SquashedDescription {
    fn from_args(args: &SquashArgs) -> Self {
        // These options are incompatible and Clap is configured to prevent this.
        assert!(args.message_paragraphs.is_empty() || !args.use_destination_message);

        if !args.message_paragraphs.is_empty() {
            let desc = join_message_paragraphs(&args.message_paragraphs);
            Self::Exact(desc)
        } else if args.use_destination_message {
            Self::UseDestination
        } else {
            Self::Combine
        }
    }
}

fn select_diff(
    tx: &WorkspaceCommandTransaction,
    sources: &[Commit],
    destination: &Commit,
    matcher: &dyn Matcher,
    diff_selector: &DiffSelector,
) -> Result<Vec<CommitWithSelection>, CommandError> {
    let mut source_commits = vec![];
    for source in sources {
        let parent_tree = source.parent_tree(tx.repo())?;
        let source_tree = source.tree();
        let format_instructions = || {
            formatdoc! {"
                You are moving changes from: {source}
                into commit: {destination}

                The left side of the diff shows the contents of the parent commit. The
                right side initially shows the contents of the commit you're moving
                changes from.

                Adjust the right side until the diff shows the changes you want to move
                to the destination. If you don't make any changes, then all the changes
                from the source will be moved into the destination.
                ",
                source = tx.format_commit_summary(source),
                destination = tx.format_commit_summary(destination),
            }
        };
        let selected_tree = diff_selector.select(
            Diff::new(&parent_tree, &source_tree),
            matcher,
            format_instructions,
        )?;
        source_commits.push(CommitWithSelection {
            commit: source.clone(),
            selected_tree,
            parent_tree,
        });
    }
    Ok(source_commits)
}
