// Copyright 2024 The Jujutsu Authors
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
use std::slice;
use std::sync::Arc;

use clap_complete::ArgValueCandidates;
use itertools::Itertools as _;
use jj_lib::backend::ChangeId;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::evolution::accumulate_predecessors;
use jj_lib::graph::TopoGroupedGraphIterator;
use jj_lib::matchers::EverythingMatcher;
use jj_lib::op_store::RefTarget;
use jj_lib::op_store::RemoteRef;
use jj_lib::op_store::RemoteRefState;
use jj_lib::refs::diff_named_commit_ids;
use jj_lib::refs::diff_named_ref_targets;
use jj_lib::refs::diff_named_remote_refs;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo;
use jj_lib::revset::RevsetExpression;
use pollster::FutureExt as _;

use crate::cli_util::CommandHelper;
use crate::cli_util::LogContentFormat;
use crate::command_error::CommandError;
use crate::complete;
use crate::diff_util::DiffFormatArgs;
use crate::diff_util::DiffRenderer;
use crate::diff_util::diff_formats_for_log;
use crate::formatter::Formatter;
use crate::formatter::FormatterExt as _;
use crate::graphlog::GraphStyle;
use crate::graphlog::get_graphlog;
use crate::templater::TemplateRenderer;
use crate::ui::Ui;

/// Compare changes to the repository between two operations
#[derive(clap::Args, Clone, Debug)]
pub struct OperationDiffArgs {
    /// Show repository changes in this operation, compared to its parent
    #[arg(
        long,
        visible_alias = "op",
        add = ArgValueCandidates::new(complete::operations),
    )]
    operation: Option<String>,
    /// Show repository changes from this operation
    #[arg(
        long, short,
        conflicts_with = "operation",
        add = ArgValueCandidates::new(complete::operations),
    )]
    from: Option<String>,
    /// Show repository changes to this operation
    #[arg(
        long, short,
        conflicts_with = "operation",
        add = ArgValueCandidates::new(complete::operations),
    )]
    to: Option<String>,
    /// Don't show the graph, show a flat list of modified changes
    #[arg(long, short = 'G')]
    no_graph: bool,
    /// Show patch of modifications to changes
    ///
    /// If the previous version has different parents, it will be temporarily
    /// rebased to the parents of the new version, so the diff is not
    /// contaminated by unrelated changes.
    #[arg(long, short = 'p')]
    patch: bool,
    #[command(flatten)]
    diff_format: DiffFormatArgs,
}

pub fn cmd_op_diff(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &OperationDiffArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let workspace_env = workspace_command.env();
    let repo_loader = workspace_command.workspace().repo_loader();
    let settings = workspace_command.settings();
    let from_ops;
    let to_op;
    if args.from.is_some() || args.to.is_some() {
        from_ops = vec![workspace_command.resolve_single_op(args.from.as_deref().unwrap_or("@"))?];
        to_op = workspace_command.resolve_single_op(args.to.as_deref().unwrap_or("@"))?;
    } else {
        to_op = workspace_command.resolve_single_op(args.operation.as_deref().unwrap_or("@"))?;
        from_ops = to_op.parents().try_collect()?;
    }
    let graph_style = GraphStyle::from_settings(settings)?;
    let with_content_format = LogContentFormat::new(ui, settings)?;

    let merged_from_op = repo_loader.merge_operations(from_ops.clone(), None)?;
    let from_repo = repo_loader.load_at(&merged_from_op)?;
    let to_repo = repo_loader.load_at(&to_op)?;

    // Create a new transaction starting from `to_repo`.
    let mut tx = to_repo.start_transaction();
    // Merge index from `from_repo` to `to_repo`, so commits in `from_repo` are
    // accessible.
    tx.repo_mut().merge_index(&from_repo)?;
    let merged_repo = tx.repo();

    let diff_renderer = {
        let formats = diff_formats_for_log(settings, &args.diff_format, args.patch)?;
        let path_converter = workspace_env.path_converter();
        let conflict_marker_style = workspace_env.conflict_marker_style();
        (!formats.is_empty())
            .then(|| DiffRenderer::new(merged_repo, path_converter, conflict_marker_style, formats))
    };
    let id_prefix_context = workspace_env.new_id_prefix_context();
    let commit_summary_template = {
        let language = workspace_env.commit_template_language(merged_repo, &id_prefix_context);
        let text = settings.get_string("templates.commit_summary")?;
        workspace_env
            .parse_template(ui, &language, &text)?
            .labeled(["op_diff", "commit"])
    };

    let op_summary_template = workspace_command
        .operation_summary_template()
        .labeled(["op_diff"]);
    ui.request_pager();
    let mut formatter = ui.stdout_formatter();
    for op in &from_ops {
        write!(formatter, "From operation: ")?;
        op_summary_template.format(op, &mut *formatter)?;
        writeln!(formatter)?;
    }
    //                "From operation: "
    write!(formatter, "  To operation: ")?;
    op_summary_template.format(&to_op, &mut *formatter)?;
    writeln!(formatter)?;

    show_op_diff(
        ui,
        formatter.as_mut(),
        merged_repo,
        &from_repo,
        &to_repo,
        &commit_summary_template,
        (!args.no_graph).then_some(graph_style),
        &with_content_format,
        diff_renderer.as_ref(),
    )
}

/// Computes and shows the differences between two operations, using the given
/// `ReadonlyRepo`s for the operations.
/// `current_repo` should contain a `Repo` with the indices of both repos merged
/// into it.
#[expect(clippy::too_many_arguments)]
pub fn show_op_diff(
    ui: &Ui,
    formatter: &mut dyn Formatter,
    current_repo: &dyn Repo,
    from_repo: &Arc<ReadonlyRepo>,
    to_repo: &Arc<ReadonlyRepo>,
    commit_summary_template: &TemplateRenderer<Commit>,
    graph_style: Option<GraphStyle>,
    with_content_format: &LogContentFormat,
    diff_renderer: Option<&DiffRenderer>,
) -> Result<(), CommandError> {
    let changes = compute_operation_commits_diff(current_repo, from_repo, to_repo)?;
    if !changes.is_empty() {
        let revset =
            RevsetExpression::commits(changes.keys().cloned().collect()).evaluate(current_repo)?;
        writeln!(formatter)?;
        with_content_format.write(formatter, |formatter| {
            writeln!(formatter, "Changed commits:")
        })?;
        if let Some(graph_style) = graph_style {
            let mut raw_output = formatter.raw()?;
            let mut graph = get_graphlog(graph_style, raw_output.as_mut());
            let graph_iter = TopoGroupedGraphIterator::new(revset.iter_graph(), |id| id);
            for node in graph_iter {
                let (commit_id, mut edges) = node?;
                let modified_change = changes.get(&commit_id).unwrap();
                // Omit "missing" edge to keep the graph concise.
                edges.retain(|edge| !edge.is_missing());

                let mut buffer = vec![];
                let within_graph = with_content_format.sub_width(graph.width(&commit_id, &edges));
                within_graph.write(ui.new_formatter(&mut buffer).as_mut(), |formatter| {
                    write_modified_change_summary(
                        formatter,
                        commit_summary_template,
                        modified_change,
                    )
                })?;
                if !buffer.ends_with(b"\n") {
                    buffer.push(b'\n');
                }
                if let Some(diff_renderer) = diff_renderer {
                    let mut formatter = ui.new_formatter(&mut buffer);
                    show_change_diff(
                        ui,
                        formatter.as_mut(),
                        diff_renderer,
                        modified_change,
                        within_graph.width(),
                    )
                    .block_on()?;
                }

                // TODO: customize node symbol?
                let node_symbol = "○";
                graph.add_node(
                    &commit_id,
                    &edges,
                    node_symbol,
                    &String::from_utf8_lossy(&buffer),
                )?;
            }
        } else {
            for commit_id in revset.iter() {
                let commit_id = commit_id?;
                let modified_change = changes.get(&commit_id).unwrap();
                with_content_format.write(formatter, |formatter| {
                    write_modified_change_summary(
                        formatter,
                        commit_summary_template,
                        modified_change,
                    )
                })?;
                if let Some(diff_renderer) = diff_renderer {
                    let width = with_content_format.width();
                    show_change_diff(ui, formatter, diff_renderer, modified_change, width)
                        .block_on()?;
                }
            }
        }
    }

    let changed_working_copies = diff_named_commit_ids(
        from_repo.view().wc_commit_ids(),
        to_repo.view().wc_commit_ids(),
    )
    .collect_vec();
    if !changed_working_copies.is_empty() {
        writeln!(formatter)?;
        for (name, (from_commit, to_commit)) in changed_working_copies {
            with_content_format.write(formatter, |formatter| {
                // Usually, there is at most one working copy changed per operation, so we put
                // the working copy name in the heading.
                write!(formatter, "Changed working copy ")?;
                write!(formatter.labeled("working_copies"), "{}@", name.as_symbol())?;
                writeln!(formatter, ":")?;
                write_ref_target_summary(
                    formatter,
                    current_repo,
                    commit_summary_template,
                    &RefTarget::resolved(to_commit.cloned()),
                    true,
                    None,
                )?;
                write_ref_target_summary(
                    formatter,
                    current_repo,
                    commit_summary_template,
                    &RefTarget::resolved(from_commit.cloned()),
                    false,
                    None,
                )
            })?;
        }
    }

    let changed_local_bookmarks = diff_named_ref_targets(
        from_repo.view().local_bookmarks(),
        to_repo.view().local_bookmarks(),
    )
    .collect_vec();
    if !changed_local_bookmarks.is_empty() {
        writeln!(formatter)?;
        with_content_format.write(formatter, |formatter| {
            writeln!(formatter, "Changed local bookmarks:")
        })?;
        for (name, (from_target, to_target)) in changed_local_bookmarks {
            with_content_format.write(formatter, |formatter| {
                writeln!(formatter, "{name}:", name = name.as_symbol())?;
                write_ref_target_summary(
                    formatter,
                    current_repo,
                    commit_summary_template,
                    to_target,
                    true,
                    None,
                )?;
                write_ref_target_summary(
                    formatter,
                    current_repo,
                    commit_summary_template,
                    from_target,
                    false,
                    None,
                )
            })?;
        }
    }

    let changed_local_tags =
        diff_named_ref_targets(from_repo.view().local_tags(), to_repo.view().local_tags())
            .collect_vec();
    if !changed_local_tags.is_empty() {
        writeln!(formatter)?;
        with_content_format.write(formatter, |formatter| writeln!(formatter, "Changed tags:"))?;
        for (name, (from_target, to_target)) in changed_local_tags {
            with_content_format.write(formatter, |formatter| {
                writeln!(formatter, "{name}:", name = name.as_symbol())?;
                write_ref_target_summary(
                    formatter,
                    current_repo,
                    commit_summary_template,
                    to_target,
                    true,
                    None,
                )?;
                write_ref_target_summary(
                    formatter,
                    current_repo,
                    commit_summary_template,
                    from_target,
                    false,
                    None,
                )
            })?;
        }
        writeln!(formatter)?;
    }

    let changed_remote_bookmarks = diff_named_remote_refs(
        from_repo.view().all_remote_bookmarks(),
        to_repo.view().all_remote_bookmarks(),
    )
    // Skip updates to the local git repo, since they should typically be covered in
    // local branches.
    .filter(|(symbol, _)| !jj_lib::git::is_special_git_remote(symbol.remote))
    .collect_vec();
    if !changed_remote_bookmarks.is_empty() {
        writeln!(formatter)?;
        with_content_format.write(formatter, |formatter| {
            writeln!(formatter, "Changed remote bookmarks:")
        })?;
        let get_remote_ref_prefix = |remote_ref: &RemoteRef| match remote_ref.state {
            RemoteRefState::New => "untracked",
            RemoteRefState::Tracked => "tracked",
        };
        for (symbol, (from_ref, to_ref)) in changed_remote_bookmarks {
            with_content_format.write(formatter, |formatter| {
                writeln!(formatter, "{symbol}:")?;
                write_ref_target_summary(
                    formatter,
                    current_repo,
                    commit_summary_template,
                    &to_ref.target,
                    true,
                    Some(get_remote_ref_prefix(to_ref)),
                )?;
                write_ref_target_summary(
                    formatter,
                    current_repo,
                    commit_summary_template,
                    &from_ref.target,
                    false,
                    Some(get_remote_ref_prefix(from_ref)),
                )
            })?;
        }
    }

    Ok(())
}

/// Writes a summary for the given `ModifiedChange`.
fn write_modified_change_summary(
    formatter: &mut dyn Formatter,
    commit_summary_template: &TemplateRenderer<Commit>,
    modified_change: &ModifiedChange,
) -> Result<(), std::io::Error> {
    for commit in modified_change.added_commits() {
        write!(formatter.labeled("diff").labeled("added"), "+")?;
        write!(formatter, " ")?;
        commit_summary_template.format(commit, formatter)?;
        writeln!(formatter)?;
    }
    for commit in modified_change.removed_commits() {
        write!(formatter.labeled("diff").labeled("removed"), "-")?;
        write!(formatter, " ")?;
        commit_summary_template.format(commit, formatter)?;
        writeln!(formatter)?;
    }
    Ok(())
}

/// Writes a summary for the given `RefTarget`.
fn write_ref_target_summary(
    formatter: &mut dyn Formatter,
    repo: &dyn Repo,
    commit_summary_template: &TemplateRenderer<Commit>,
    ref_target: &RefTarget,
    added: bool,
    prefix: Option<&str>,
) -> Result<(), CommandError> {
    let write_prefix = |formatter: &mut dyn Formatter,
                        added: bool,
                        prefix: Option<&str>|
     -> Result<(), CommandError> {
        if added {
            write!(formatter.labeled("diff").labeled("added"), "+")?;
        } else {
            write!(formatter.labeled("diff").labeled("removed"), "-")?;
        }
        write!(formatter, " ")?;
        if let Some(prefix) = prefix {
            write!(formatter, "{prefix} ")?;
        }
        Ok(())
    };
    if ref_target.is_absent() {
        write_prefix(formatter, added, prefix)?;
        writeln!(formatter, "(absent)")?;
    } else if ref_target.has_conflict() {
        for commit_id in ref_target.added_ids() {
            write_prefix(formatter, added, prefix)?;
            write!(formatter, "(added) ")?;
            let commit = repo.store().get_commit(commit_id)?;
            commit_summary_template.format(&commit, formatter)?;
            writeln!(formatter)?;
        }
        for commit_id in ref_target.removed_ids() {
            write_prefix(formatter, added, prefix)?;
            write!(formatter, "(removed) ")?;
            let commit = repo.store().get_commit(commit_id)?;
            commit_summary_template.format(&commit, formatter)?;
            writeln!(formatter)?;
        }
    } else {
        write_prefix(formatter, added, prefix)?;
        let commit_id = ref_target.as_normal().unwrap();
        let commit = repo.store().get_commit(commit_id)?;
        commit_summary_template.format(&commit, formatter)?;
        writeln!(formatter)?;
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ModifiedChange {
    /// Created or rewritten commit.
    Existing {
        commit: Commit,
        predecessors: Vec<Commit>,
    },
    /// Abandoned commit.
    Abandoned { commit: Commit },
}

impl ModifiedChange {
    fn removed_commits(&self) -> &[Commit] {
        match self {
            Self::Existing { predecessors, .. } => predecessors,
            Self::Abandoned { commit } => slice::from_ref(commit),
        }
    }

    fn added_commits(&self) -> &[Commit] {
        match self {
            Self::Existing { commit, .. } => slice::from_ref(commit),
            Self::Abandoned { .. } => &[],
        }
    }
}

/// Computes created/rewritten/abandoned commits between two operations.
///
/// Returns a map of [`ModifiedChange`]s containing the new and old commits. For
/// created/rewritten commits, the map entries are indexed by new ids. For
/// abandoned commits, the entries are indexed by old ids.
fn compute_operation_commits_diff(
    repo: &dyn Repo,
    from_repo: &ReadonlyRepo,
    to_repo: &ReadonlyRepo,
) -> Result<HashMap<CommitId, ModifiedChange>, CommandError> {
    let store = repo.store();
    let from_heads = from_repo.view().heads().iter().cloned().collect_vec();
    let to_heads = to_repo.view().heads().iter().cloned().collect_vec();
    let from_expr = RevsetExpression::commits(from_heads);
    let to_expr = RevsetExpression::commits(to_heads);

    let predecessor_commits = accumulate_predecessors(
        slice::from_ref(to_repo.operation()),
        slice::from_ref(from_repo.operation()),
    )?;

    // Collect hidden commits to find abandoned/rewritten changes.
    let mut hidden_commits_by_change: HashMap<ChangeId, CommitId> = HashMap::new();
    let mut abandoned_commits: HashSet<CommitId> = HashSet::new();
    let newly_hidden = to_expr.range(&from_expr).evaluate(repo)?;
    for item in newly_hidden.commit_change_ids() {
        let (commit_id, change_id) = item?;
        // Just pick one if diverged. Divergent commits shouldn't be considered
        // "squashed" into the new commit.
        hidden_commits_by_change
            .entry(change_id)
            .or_insert_with(|| commit_id.clone());
        abandoned_commits.insert(commit_id);
    }

    // For each new commit, copy/deduce predecessors based on change id.
    let mut changes: HashMap<CommitId, ModifiedChange> = HashMap::new();
    let newly_visible = from_expr.range(&to_expr).evaluate(repo)?;
    for item in newly_visible.commit_change_ids() {
        let (commit_id, change_id) = item?;
        let predecessor_ids = if let Some(ids) = predecessor_commits.get(&commit_id) {
            ids // including visible predecessors
        } else if let Some(id) = hidden_commits_by_change.get(&change_id) {
            slice::from_ref(id)
        } else {
            &[]
        };
        for id in predecessor_ids {
            abandoned_commits.remove(id);
        }
        let change = ModifiedChange::Existing {
            commit: store.get_commit(&commit_id)?,
            predecessors: predecessor_ids
                .iter()
                .map(|id| store.get_commit(id))
                .try_collect()?,
        };
        changes.insert(commit_id, change);
    }

    // Record remainders as abandoned.
    for commit_id in abandoned_commits {
        let change = ModifiedChange::Abandoned {
            commit: store.get_commit(&commit_id)?,
        };
        changes.insert(commit_id, change);
    }

    Ok(changes)
}

/// Displays the diffs of a modified change.
///
/// For created/rewritten commits, the diff is shown between the old (or
/// predecessor) commits and the new commit. The old commits are temporarily
/// rebased onto the new commit's parents. For abandoned commits, the diff is
/// shown of that commit's contents.
async fn show_change_diff(
    ui: &Ui,
    formatter: &mut dyn Formatter,
    diff_renderer: &DiffRenderer<'_>,
    change: &ModifiedChange,
    width: usize,
) -> Result<(), CommandError> {
    match change {
        ModifiedChange::Existing {
            commit,
            predecessors,
        } => {
            diff_renderer
                .show_inter_diff(
                    ui,
                    formatter,
                    // TODO: It's technically wrong to show diffs from the first
                    // predecessor, but diff of partial "squash" operation would be
                    // unreadable otherwise. We have the same problem in "evolog",
                    // but it's less of an issue there because "evolog" shows the
                    // predecessors recursively.
                    predecessors.get(..1).unwrap_or(&[]),
                    commit,
                    &EverythingMatcher,
                    width,
                )
                .await?;
        }
        ModifiedChange::Abandoned { commit } => {
            // TODO: Should we show a reverse diff?
            diff_renderer
                .show_patch(ui, formatter, commit, &EverythingMatcher, width)
                .await?;
        }
    }
    Ok(())
}
