use std::collections::HashSet;

use clap_complete::ArgValueCompleter;
use itertools::Itertools as _;
use jj_lib::backend::BackendError;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::complete;
use crate::ui::Ui;

/// Simplify parent edges for the specified revision(s).
///
/// Removes all parents of each of the specified revisions that are also
/// indirect ancestors of the same revisions through other parents. This has no
/// effect on any revision's contents, including the working copy.
///
/// In other words, for all (A, B, C) where A has (B, C) as parents and C is an
/// ancestor of B, A will be rewritten to have only B as a parent instead of
/// B+C.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct SimplifyParentsArgs {
    /// Simplify specified revision(s) together with their trees of descendants
    /// (can be repeated)
    #[arg(
        long,
        short,
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_mutable),
    )]
    source: Vec<RevisionArg>,

    /// Simplify specified revision(s) (can be repeated)
    ///
    /// If both `--source` and `--revisions` are not provided, this defaults to
    /// the `revsets.simplify-parents` setting, or `reachable(@, mutable())`
    /// if it is not set.
    #[arg(
        long,
        short,
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_mutable),
    )]
    revisions: Vec<RevisionArg>,
}

pub(crate) fn cmd_simplify_parents(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &SimplifyParentsArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let revs = if args.source.is_empty() && args.revisions.is_empty() {
        let revs = workspace_command
            .settings()
            .get_string("revsets.simplify-parents")?;
        workspace_command
            .parse_revset(ui, &RevisionArg::from(revs))?
            .resolve()?
    } else {
        workspace_command
            .parse_union_revsets(ui, &args.source)?
            .resolve()?
            .descendants()
            .union(
                &workspace_command
                    .parse_union_revsets(ui, &args.revisions)?
                    .resolve()?,
            )
    };
    workspace_command.check_rewritable_expr(&revs)?;
    let commit_ids: Vec<_> = revs
        .evaluate(workspace_command.repo().as_ref())?
        .iter()
        .try_collect()?;
    let commit_ids_set: HashSet<_> = commit_ids.iter().cloned().collect();
    let num_orig_commits = commit_ids.len();

    let mut tx = workspace_command.start_transaction();
    let mut simplified_commits = 0;
    let mut edges = 0;
    let mut reparented_descendants = 0;

    tx.repo_mut()
        .transform_descendants(commit_ids, async |mut rewriter| {
            let num_old_heads = rewriter.new_parents().len();
            if commit_ids_set.contains(rewriter.old_commit().id()) && num_old_heads > 1 {
                // TODO: BackendError is not the right error here because
                // the error does not come from `Backend`, but `Index`.
                rewriter
                    .simplify_ancestor_merge()
                    .map_err(|err| BackendError::Other(err.into()))?;
            }
            let num_new_heads = rewriter.new_parents().len();

            if rewriter.parents_changed() {
                rewriter.reparent().write()?;

                if num_new_heads < num_old_heads {
                    simplified_commits += 1;
                    edges += num_old_heads - num_new_heads;
                } else {
                    reparented_descendants += 1;
                }
            }
            Ok(())
        })?;

    if let Some(mut formatter) = ui.status_formatter()
        && simplified_commits > 0
    {
        writeln!(
            formatter,
            "Removed {edges} edges from {simplified_commits} out of {num_orig_commits} commits.",
        )?;
        if reparented_descendants > 0 {
            writeln!(
                formatter,
                "Rebased {reparented_descendants} descendant commits",
            )?;
        }
    }
    tx.finish(ui, format!("simplify {num_orig_commits} commits"))?;

    Ok(())
}
