// Copyright 2020-2023 The Jujutsu Authors
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
use std::sync::Arc;

use clap::ArgGroup;
use clap_complete::ArgValueCompleter;
use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::revset::RevsetExpression;
use jj_lib::rewrite::EmptyBehavior;
use jj_lib::rewrite::MoveCommitsLocation;
use jj_lib::rewrite::MoveCommitsStats;
use jj_lib::rewrite::MoveCommitsTarget;
use jj_lib::rewrite::RebaseOptions;
use jj_lib::rewrite::RewriteRefsOptions;
use jj_lib::rewrite::compute_move_commits;
use jj_lib::rewrite::find_duplicate_divergent_commits;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::compute_commit_location;
use crate::cli_util::print_updated_commits;
use crate::cli_util::short_commit_hash;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::complete;
use crate::ui::Ui;

/// Move revisions to different parent(s)
///
/// This command moves revisions to different parent(s) while preserving the
/// changes (diff) in the revisions.
///
/// There are three different ways of specifying which revisions to rebase:
///
/// * `--source/-s` to rebase a revision and its descendants
/// * `--branch/-b` to rebase a whole branch, relative to the destination
/// * `--revisions/-r` to rebase the specified revisions without their
///   descendants
///
/// If no option is specified, it defaults to `-b @`.
///
/// There are three different ways of specifying where the revisions should be
/// rebased to:
///
/// * `--onto/-o` to rebase the revisions onto the specified targets
/// * `--insert-after/-A` to rebase the revisions onto the specified targets and
///   to rebase the targets' descendants onto the rebased revisions
/// * `--insert-before/-B` to rebase the revisions onto the specified targets'
///   parents and to rebase the targets and their descendants onto the rebased
///   revisions
///
/// See the sections below for details about the different ways of specifying
/// which revisions to rebase where.
///
/// If a working-copy revision gets abandoned, it will be given a new, empty
/// revision. This is true in general; it is not specific to this command.
///
/// ### Specifying which revisions to rebase
///
/// With `--source/-s`, the command rebases the specified revision and its
/// descendants to the destination. For example, `jj rebase -s M -o O` would
/// transform your history like this (letters followed by an apostrophe are
/// post-rebase versions):
///
/// ```text
/// O           N'
/// |           |
/// | N         M'
/// | |         |
/// | M         O
/// | |    =>   |
/// | | L       | L
/// | |/        | |
/// | K         | K
/// |/          |/
/// J           J
/// ```
///
/// Each revision passed to `-s` will become a direct child of the destination,
/// so if you instead run `jj rebase -s M -s N -o O` (or `jj rebase -s 'M|N' -o
/// O`) in the example above, then N' would instead be a direct child of O.
///
/// With `--branch/-b`, the command rebases the whole "branch" containing the
/// specified revision. A "branch" is the set of revisions that includes:
///
/// * the specified revision and ancestors that are not also ancestors of the
///   destination
/// * all descendants of those revisions
///
/// In other words, `jj rebase -b X -o Y` rebases revisions in the revset
/// `(Y..X)::` (which is equivalent to `jj rebase -s 'roots(Y..X)' -o Y` for a
/// single root). For example, either `jj rebase -b L -o O` or `jj rebase -b M
/// -o O` would transform your history like this (because `L` and `M` are on the
/// same "branch", relative to the destination):
///
/// ```text
/// O           N'
/// |           |
/// | N         M'
/// | |         |
/// | M         | L'
/// | |    =>   |/
/// | | L       K'
/// | |/        |
/// | K         O
/// |/          |
/// J           J
/// ```
///
/// With `--revisions/-r`, the command rebases only the specified revisions to
/// the destination. Any "hole" left behind will be filled by rebasing
/// descendants onto the specified revisions' parent(s). For example,
/// `jj rebase -r K -o M` would transform your history like this:
///
/// ```text
/// M          K'
/// |          |
/// | L        M
/// | |   =>   |
/// | K        | L'
/// |/         |/
/// J          J
/// ```
///
/// Multiple revisions can be specified, and any dependencies (graph edges)
/// within the set will be preserved. For example, `jj rebase -r 'K|N' -o O`
/// would transform your history like this:
///
/// ```text
/// O           N'
/// |           |
/// | N         K'
/// | |         |
/// | M         O
/// | |    =>   |
/// | | L       | M'
/// | |/        |/
/// | K         | L'
/// |/          |/
/// J           J
/// ```
///
/// `jj rebase -s X` is similar to `jj rebase -r X::` and will behave the same
/// if X is a single revision. However, if X is a set of multiple revisions,
/// or if you passed multiple `-s` arguments, then `jj rebase -s` will make each
/// of the specified revisions an immediate child of the destination, while
/// `jj rebase -r` will preserve dependencies within the set.
///
/// Note that you can create a merge revision by repeating the `-o` argument.
/// For example, if you realize that revision L actually depends on revision M
/// in order to work (in addition to its current parent K), you can run `jj
/// rebase -s L -o K -o M`:
///
/// ```text
/// M          L'
/// |          |\
/// | L        M |
/// | |   =>   | |
/// | K        | K
/// |/         |/
/// J          J
/// ```
///
/// ### Specifying where to rebase the revisions
///
/// With `--onto/-o`, the command rebases the selected revisions onto the
/// targets. Existing descendants of the targets will not be affected. See
/// the section above for examples.
///
/// With `--insert-after/-A`, the selected revisions will be inserted after the
/// targets. This is similar to `-o`, but if the targets have any existing
/// descendants, then those will be rebased onto the rebased selected revisions.
///
/// For example, `jj rebase -r K -A L` will rewrite history like this:
/// ```text
/// N           N'
/// |           |
/// | M         | M'
/// |/          |/
/// L      =>   K'
/// |           |
/// | K         L
/// |/          |
/// J           J
/// ```
///
/// The `-A` (and `-B`) argument can also be used for reordering revisions. For
/// example, `jj rebase -r M -A J` will rewrite history like this:
/// ```text
/// M          L'
/// |          |
/// L          K'
/// |     =>   |
/// K          M'
/// |          |
/// J          J
/// ```
///
/// With `--insert-before/-B`, the selected revisions will be inserted before
/// the targets. This is achieved by rebasing the selected revisions onto the
/// target revisions' parents, and then rebasing the target revisions and their
/// descendants onto the rebased revisions.
///
/// For example, `jj rebase -r K -B L` will rewrite history like this:
/// ```text
/// N           N'
/// |           |
/// | M         | M'
/// |/          |/
/// L     =>    L'
/// |           |
/// | K         K'
/// |/          |
/// J           J
/// ```
///
/// The `-A` and `-B` arguments can also be combined, which can be useful around
/// merges. For example, you can use `jj rebase -r K -A J -B M` to create a new
/// merge (but `jj rebase -r M -o L -o K` might be simpler in this particular
/// case):
/// ```text
/// M           M'
/// |           |\
/// L           L |
/// |     =>    | |
/// | K         | K'
/// |/          |/
/// J           J
/// ```
///
/// To insert a commit inside an existing merge with `jj rebase -r O -A K -B M`:
/// ```text
/// O           N'
/// |           |\
/// N           | M'
/// |\          | |\
/// | M         | O'|
/// | |    =>   |/ /
/// | L         | L
/// | |         | |
/// K |         K |
/// |/          |/
/// J           J
/// ```
#[derive(clap::Args, Clone, Debug)]
#[command(verbatim_doc_comment)]
#[command(group(ArgGroup::new("to_rebase").args(&["branch", "source", "revisions"])))]
pub(crate) struct RebaseArgs {
    /// Rebase the whole branch relative to destination's ancestors (can be
    /// repeated)
    ///
    /// `jj rebase -b=br -o=dst` is equivalent to `jj rebase '-s=roots(dst..br)'
    /// -o=dst`.
    ///
    /// If none of `-b`, `-s`, or `-r` is provided, then the default is `-b @`.
    #[arg(
        long,
        short,
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_mutable),
    )]
    branch: Vec<RevisionArg>,

    /// Rebase specified revision(s) together with their trees of descendants
    /// (can be repeated)
    ///
    /// Each specified revision will become a direct child of the destination
    /// revision(s), even if some of the source revisions are descendants
    /// of others.
    ///
    /// If none of `-b`, `-s`, or `-r` is provided, then the default is `-b @`.
    #[arg(
        long,
        short,
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_mutable),
    )]
    source: Vec<RevisionArg>,
    /// Rebase the given revisions, rebasing descendants onto this revision's
    /// parent(s)
    ///
    /// Unlike `-s` or `-b`, you may `jj rebase -r` a revision `A` onto a
    /// descendant of `A`.
    ///
    /// If none of `-b`, `-s`, or `-r` is provided, then the default is `-b @`.
    #[arg(
        long,
        short,
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_mutable),
    )]
    revisions: Vec<RevisionArg>,

    #[command(flatten)]
    destination: RebaseDestinationArgs,

    /// If true, when rebasing would produce an empty commit, the commit is
    /// abandoned. It will not be abandoned if it was already empty before the
    /// rebase. Will never skip merge commits with multiple non-empty
    /// parents.
    #[arg(long)]
    skip_emptied: bool,

    /// Keep divergent commits while rebasing
    ///
    /// Without this flag, divergent commits are abandoned while rebasing if
    /// another commit with the same change ID is already present in the
    /// destination with identical changes.
    #[arg(long)]
    keep_divergent: bool,
}

#[derive(clap::Args, Clone, Debug)]
#[group(required = true)]
pub struct RebaseDestinationArgs {
    /// The revision(s) to rebase onto (can be repeated to create a merge
    /// commit)
    #[arg(
        long,
        alias = "destination",
        short,
        short_alias = 'd',
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_all),
    )]
    onto: Option<Vec<RevisionArg>>,
    /// The revision(s) to insert after (can be repeated to create a merge
    /// commit)
    #[arg(
        long,
        short = 'A',
        visible_alias = "after",
        conflicts_with = "onto",
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_all),
    )]
    insert_after: Option<Vec<RevisionArg>>,
    /// The revision(s) to insert before (can be repeated to create a merge
    /// commit)
    #[arg(
        long,
        short = 'B',
        visible_alias = "before",
        conflicts_with = "onto",
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_mutable),
    )]
    insert_before: Option<Vec<RevisionArg>>,
}

#[instrument(skip_all)]
pub(crate) fn cmd_rebase(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &RebaseArgs,
) -> Result<(), CommandError> {
    let rebase_options = RebaseOptions {
        empty: match args.skip_emptied {
            true => EmptyBehavior::AbandonNewlyEmpty,
            false => EmptyBehavior::Keep,
        },
        rewrite_refs: RewriteRefsOptions {
            delete_abandoned_bookmarks: false,
        },
        simplify_ancestor_merge: false,
    };
    let mut workspace_command = command.workspace_helper(ui)?;
    let loc = if !args.revisions.is_empty() {
        plan_rebase_revisions(ui, &workspace_command, &args.revisions, &args.destination)?
    } else if !args.source.is_empty() {
        plan_rebase_source(ui, &workspace_command, &args.source, &args.destination)?
    } else {
        plan_rebase_branch(ui, &workspace_command, &args.branch, &args.destination)?
    };

    let mut tx = workspace_command.start_transaction();
    let mut computed_move = compute_move_commits(tx.repo(), &loc)?;
    if !args.keep_divergent {
        let abandoned_divergent =
            find_duplicate_divergent_commits(tx.repo(), &loc.new_parent_ids, &loc.target)?;
        computed_move.record_to_abandon(abandoned_divergent.iter().map(Commit::id).cloned());
        if !abandoned_divergent.is_empty()
            && let Some(mut formatter) = ui.status_formatter()
        {
            writeln!(
                formatter,
                "Abandoned {} divergent commits that were already present in the destination:",
                abandoned_divergent.len(),
            )?;
            print_updated_commits(
                formatter.as_mut(),
                &tx.base_workspace_helper().commit_summary_template(),
                &abandoned_divergent,
            )?;
        }
    };
    let stats = computed_move.apply(tx.repo_mut(), &rebase_options)?;
    print_move_commits_stats(ui, &stats)?;
    tx.finish(ui, tx_description(&loc.target))?;

    Ok(())
}

fn plan_rebase_revisions(
    ui: &Ui,
    workspace_command: &WorkspaceCommandHelper,
    revisions: &[RevisionArg],
    rebase_destination: &RebaseDestinationArgs,
) -> Result<MoveCommitsLocation, CommandError> {
    let target_expr = workspace_command
        .parse_union_revsets(ui, revisions)?
        .resolve()?;
    workspace_command.check_rewritable_expr(&target_expr)?;
    let target_commit_ids: Vec<_> = target_expr
        .evaluate(workspace_command.repo().as_ref())?
        .iter()
        .try_collect()?; // in reverse topological order

    let (new_parent_ids, new_child_ids) = compute_commit_location(
        ui,
        workspace_command,
        rebase_destination.onto.as_deref(),
        rebase_destination.insert_after.as_deref(),
        rebase_destination.insert_before.as_deref(),
        "rebased commits",
    )?;
    if rebase_destination.onto.is_some() {
        for id in &target_commit_ids {
            if new_parent_ids.contains(id) {
                return Err(user_error(format!(
                    "Cannot rebase {} onto itself",
                    short_commit_hash(id),
                )));
            }
        }
    }
    Ok(MoveCommitsLocation {
        new_parent_ids,
        new_child_ids,
        target: MoveCommitsTarget::Commits(target_commit_ids),
    })
}

fn plan_rebase_source(
    ui: &Ui,
    workspace_command: &WorkspaceCommandHelper,
    source: &[RevisionArg],
    rebase_destination: &RebaseDestinationArgs,
) -> Result<MoveCommitsLocation, CommandError> {
    let source_commit_ids = Vec::from_iter(workspace_command.resolve_some_revsets(ui, source)?);
    workspace_command.check_rewritable(&source_commit_ids)?;

    let (new_parent_ids, new_child_ids) = compute_commit_location(
        ui,
        workspace_command,
        rebase_destination.onto.as_deref(),
        rebase_destination.insert_after.as_deref(),
        rebase_destination.insert_before.as_deref(),
        "rebased commits",
    )?;
    if rebase_destination.onto.is_some() {
        for id in &source_commit_ids {
            let commit = workspace_command.repo().store().get_commit(id)?;
            check_rebase_destinations(workspace_command.repo(), &new_parent_ids, &commit)?;
        }
    }

    Ok(MoveCommitsLocation {
        new_parent_ids,
        new_child_ids,
        target: MoveCommitsTarget::Roots(source_commit_ids),
    })
}

fn plan_rebase_branch(
    ui: &Ui,
    workspace_command: &WorkspaceCommandHelper,
    branch: &[RevisionArg],
    rebase_destination: &RebaseDestinationArgs,
) -> Result<MoveCommitsLocation, CommandError> {
    let branch_commit_ids: Vec<_> = if branch.is_empty() {
        vec![
            workspace_command
                .resolve_single_rev(ui, &RevisionArg::AT)?
                .id()
                .clone(),
        ]
    } else {
        workspace_command
            .resolve_some_revsets(ui, branch)?
            .into_iter()
            .collect()
    };

    let (new_parent_ids, new_child_ids) = compute_commit_location(
        ui,
        workspace_command,
        rebase_destination.onto.as_deref(),
        rebase_destination.insert_after.as_deref(),
        rebase_destination.insert_before.as_deref(),
        "rebased commits",
    )?;
    let roots_expression = RevsetExpression::commits(new_parent_ids.clone())
        .range(&RevsetExpression::commits(branch_commit_ids))
        .roots();
    workspace_command.check_rewritable_expr(&roots_expression)?;
    let root_commit_ids: Vec<_> = roots_expression
        .evaluate(workspace_command.repo().as_ref())
        .unwrap()
        .iter()
        .try_collect()?;
    if rebase_destination.onto.is_some() {
        for id in &root_commit_ids {
            let commit = workspace_command.repo().store().get_commit(id)?;
            check_rebase_destinations(workspace_command.repo(), &new_parent_ids, &commit)?;
        }
    }

    Ok(MoveCommitsLocation {
        new_parent_ids,
        new_child_ids,
        target: MoveCommitsTarget::Roots(root_commit_ids),
    })
}

fn check_rebase_destinations(
    repo: &Arc<ReadonlyRepo>,
    new_parents: &[CommitId],
    commit: &Commit,
) -> Result<(), CommandError> {
    for parent_id in new_parents {
        if parent_id == commit.id() {
            return Err(user_error(format!(
                "Cannot rebase {} onto itself",
                short_commit_hash(commit.id()),
            )));
        }
        if repo.index().is_ancestor(commit.id(), parent_id)? {
            return Err(user_error(format!(
                "Cannot rebase {} onto descendant {}",
                short_commit_hash(commit.id()),
                short_commit_hash(parent_id)
            )));
        }
    }
    Ok(())
}

fn tx_description(target: &MoveCommitsTarget) -> String {
    match &target {
        MoveCommitsTarget::Commits(ids) => match &ids[..] {
            [] => format!("rebase {} commits", ids.len()),
            [id] => format!("rebase commit {}", id.hex()),
            [first, others @ ..] => {
                format!("rebase commit {} and {} more", first.hex(), others.len())
            }
        },
        MoveCommitsTarget::Roots(ids) => match &ids[..] {
            [id] => format!("rebase commit {} and descendants", id.hex()),
            _ => format!("rebase {} commits and their descendants", ids.len()),
        },
    }
}

/// Print details about the provided [`MoveCommitsStats`].
fn print_move_commits_stats(ui: &Ui, stats: &MoveCommitsStats) -> std::io::Result<()> {
    let Some(mut formatter) = ui.status_formatter() else {
        return Ok(());
    };
    let &MoveCommitsStats {
        num_rebased_targets,
        num_rebased_descendants,
        num_skipped_rebases,
        num_abandoned_empty,
        rebased_commits: _,
    } = stats;
    if num_skipped_rebases > 0 {
        writeln!(
            formatter,
            "Skipped rebase of {num_skipped_rebases} commits that were already in place"
        )?;
    }
    if num_rebased_targets > 0 {
        writeln!(
            formatter,
            "Rebased {num_rebased_targets} commits to destination"
        )?;
    }
    if num_rebased_descendants > 0 {
        writeln!(
            formatter,
            "Rebased {num_rebased_descendants} descendant commits"
        )?;
    }
    if num_abandoned_empty > 0 {
        writeln!(
            formatter,
            "Abandoned {num_abandoned_empty} newly emptied commits"
        )?;
    }
    Ok(())
}
