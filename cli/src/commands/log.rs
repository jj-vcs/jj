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

use clap_complete::ArgValueCandidates;
use clap_complete::ArgValueCompleter;
use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::graph::GraphEdge;
use jj_lib::graph::GraphEdgeType;
use jj_lib::graph::TopoGroupedGraphIterator;
use jj_lib::graph::reverse_graph;
use jj_lib::repo::Repo as _;
use jj_lib::revset::RevsetEvaluationError;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::RevsetFilterPredicate;
use jj_lib::revset::RevsetIteratorExt as _;
use pollster::FutureExt as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::LogContentFormat;
use crate::cli_util::RevisionArg;
use crate::cli_util::format_template;
use crate::command_error::CommandError;
use crate::complete;
use crate::diff_util::DiffFormatArgs;
use crate::formatter::FormatterExt as _;
use crate::graphlog::GraphStyle;
use crate::graphlog::get_graphlog;
use crate::templater::TemplateRenderer;
use crate::ui::Ui;

/// Show revision history
///
/// Renders a graphical view of the project's history, ordered with children
/// before parents. By default, the output only includes mutable revisions,
/// along with some additional revisions for context. Use `jj log -r ::` to see
/// all revisions. See [`jj help -k revsets`] for information about the syntax.
///
/// [`jj help -k revsets`]:
///     https://docs.jj-vcs.dev/latest/revsets/
///
/// Spans of revisions that are not included in the graph per `--revisions` are
/// rendered as a synthetic node labeled "(elided revisions)".
///
/// The working-copy commit is indicated by a `@` symbol in the graph.
/// [Immutable revisions] have a `◆` symbol. Other commits have a `○` symbol.
/// All of these symbols can be [customized].
///
/// [Immutable revisions]:
///     https://docs.jj-vcs.dev/latest/config/#set-of-immutable-commits
///
/// [customized]:
///     https://docs.jj-vcs.dev/latest/config/#node-style
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct LogArgs {
    /// Which revisions to show
    ///
    /// If no paths nor revisions are specified, this defaults to the
    /// `revsets.log` setting.
    #[arg(
        long,
        short,
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_all),
    )]
    revisions: Vec<RevisionArg>,
    /// Show revisions modifying the given paths
    #[arg(
        value_name = "FILESETS",
        value_hint = clap::ValueHint::AnyPath,
        add = ArgValueCompleter::new(complete::log_files),
    )]
    paths: Vec<String>,
    /// Limit number of revisions to show
    ///
    /// Applied after revisions are filtered and reordered topologically, but
    /// before being reversed.
    #[arg(long, short = 'n')]
    limit: Option<usize>,
    /// Show revisions in the opposite order (older revisions first)
    #[arg(long)]
    reversed: bool,
    /// Don't show the graph, show a flat list of revisions
    #[arg(long, short = 'G')]
    no_graph: bool,
    /// Render each revision using the given template
    ///
    /// Run `jj log -T` to list the built-in templates.
    ///
    /// You can also specify arbitrary template expressions using the
    /// [built-in keywords]. See [`jj help -k templates`] for more
    /// information.
    ///
    /// If not specified, this defaults to the `templates.log` setting.
    ///
    /// [built-in keywords]:
    ///     https://docs.jj-vcs.dev/latest/templates/#commit-keywords
    ///
    /// [`jj help -k templates`]:
    ///     https://docs.jj-vcs.dev/latest/templates/
    #[arg(long, short = 'T', add = ArgValueCandidates::new(complete::template_aliases))]
    template: Option<String>,
    /// Show patch
    #[arg(long, short = 'p')]
    patch: bool,
    #[command(flatten)]
    diff_format: DiffFormatArgs,
}

#[instrument(skip_all)]
pub(crate) fn cmd_log(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &LogArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let settings = workspace_command.settings();

    let fileset_expression = workspace_command.parse_file_patterns(ui, &args.paths)?;
    let mut explicit_paths = fileset_expression.explicit_paths().collect_vec();
    let revset_expression = {
        // only use default revset if neither revset nor path are specified
        let mut expression = if args.revisions.is_empty() && args.paths.is_empty() {
            let revset_string = settings.get_string("revsets.log")?;
            workspace_command.parse_revset(ui, &RevisionArg::from(revset_string))?
        } else if !args.revisions.is_empty() {
            workspace_command.parse_union_revsets(ui, &args.revisions)?
        } else {
            // a path was specified so we use all() and add path filter later
            workspace_command.attach_revset_evaluator(RevsetExpression::all())
        };
        if !args.paths.is_empty() {
            // Beware that args.paths = ["root:."] is not identical to []. The
            // former will filter out empty commits.
            let predicate = RevsetFilterPredicate::File(fileset_expression.clone());
            expression.intersect_with(&RevsetExpression::filter(predicate));
        }
        expression
    };
    let prio_revset = settings.get_string("revsets.log-graph-prioritize")?;
    let prio_revset = workspace_command.parse_revset(ui, &RevisionArg::from(prio_revset))?;

    let repo = workspace_command.repo();
    let matcher = fileset_expression.to_matcher();
    let revset = revset_expression.evaluate()?;

    let store = repo.store();
    let diff_renderer = workspace_command.diff_renderer_for_log(&args.diff_format, args.patch)?;
    let graph_style = GraphStyle::from_settings(settings)?;

    let use_elided_nodes = settings.get_bool("ui.log-synthetic-elided-nodes")?;
    let with_content_format = LogContentFormat::new(ui, settings)?;

    let template: TemplateRenderer<Commit>;
    let node_template: TemplateRenderer<Option<Commit>>;
    {
        let language = workspace_command.commit_template_language();
        let template_string = match &args.template {
            Some(value) => value.clone(),
            None => settings.get_string("templates.log")?,
        };
        template = workspace_command
            .parse_template(ui, &language, &template_string)?
            .labeled(["log", "commit"]);
        node_template = workspace_command
            .parse_template(ui, &language, &settings.get_string("templates.log_node")?)?
            .labeled(["log", "commit", "node"]);
    }

    {
        ui.request_pager();
        let mut formatter = ui.stdout_formatter();
        let formatter = formatter.as_mut();

        if !args.no_graph {
            let mut raw_output = formatter.raw()?;
            let mut graph = get_graphlog(graph_style, raw_output.as_mut());
            let iter: Box<dyn Iterator<Item = _>> = {
                let mut forward_iter = TopoGroupedGraphIterator::new(revset.iter_graph(), |id| id);

                let has_commit = revset.containing_fn();

                for prio in prio_revset.evaluate_to_commit_ids()? {
                    let prio = prio?;
                    if has_commit(&prio)? {
                        forward_iter.prioritize_branch(prio);
                    }
                }

                // The input to TopoGroupedGraphIterator shouldn't be truncated
                // because the prioritized commit must exist in the input set.
                let forward_iter = forward_iter.take(args.limit.unwrap_or(usize::MAX));
                if args.reversed {
                    Box::new(reverse_graph(forward_iter, |id| id)?.into_iter().map(Ok))
                } else {
                    Box::new(forward_iter)
                }
            };
            for node in iter {
                let (commit_id, edges) = node?;

                // The graph is keyed by (CommitId, is_synthetic)
                let mut graphlog_edges = vec![];
                // TODO: Should we update revset.iter_graph() to yield a `has_missing` flag
                // instead of all the missing edges since we don't care about
                // where they point here anyway?
                let mut missing_edge_id = None;
                let mut elided_targets = vec![];
                for edge in edges {
                    match edge.edge_type {
                        GraphEdgeType::Missing => {
                            missing_edge_id = Some(edge.target);
                        }
                        GraphEdgeType::Direct => {
                            graphlog_edges.push(GraphEdge::direct((edge.target, false)));
                        }
                        GraphEdgeType::Indirect => {
                            if use_elided_nodes {
                                elided_targets.push(edge.target.clone());
                                graphlog_edges.push(GraphEdge::direct((edge.target, true)));
                            } else {
                                graphlog_edges.push(GraphEdge::indirect((edge.target, false)));
                            }
                        }
                    }
                }
                if let Some(missing_edge_id) = missing_edge_id {
                    graphlog_edges.push(GraphEdge::missing((missing_edge_id, false)));
                }
                let mut buffer = vec![];
                let key = (commit_id, false);
                let commit = store.get_commit(&key.0)?;
                let within_graph =
                    with_content_format.sub_width(graph.width(&key, &graphlog_edges));
                within_graph.write(ui.new_formatter(&mut buffer).as_mut(), |formatter| {
                    template.format(&commit, formatter)
                })?;
                if !buffer.ends_with(b"\n") {
                    buffer.push(b'\n');
                }
                if let Some(renderer) = &diff_renderer {
                    let mut formatter = ui.new_formatter(&mut buffer);
                    renderer
                        .show_patch(
                            ui,
                            formatter.as_mut(),
                            &commit,
                            matcher.as_ref(),
                            within_graph.width(),
                        )
                        .block_on()?;
                }

                let commit = Some(commit);
                let node_symbol = format_template(ui, &commit, &node_template);
                graph.add_node(
                    &key,
                    &graphlog_edges,
                    &node_symbol,
                    &String::from_utf8_lossy(&buffer),
                )?;

                let tree = commit.map(|c| c.tree()).unwrap();
                // TODO: propagate errors
                explicit_paths.retain(|&path| tree.path_value(path).unwrap().is_absent());

                for elided_target in elided_targets {
                    let elided_key = (elided_target, true);
                    let real_key = (elided_key.0.clone(), false);
                    let edges = [GraphEdge::direct(real_key)];
                    let mut buffer = vec![];
                    let within_graph =
                        with_content_format.sub_width(graph.width(&elided_key, &edges));
                    within_graph.write(ui.new_formatter(&mut buffer).as_mut(), |formatter| {
                        writeln!(formatter.labeled("elided"), "(elided revisions)")
                    })?;
                    let node_symbol = format_template(ui, &None, &node_template);
                    graph.add_node(
                        &elided_key,
                        &edges,
                        &node_symbol,
                        &String::from_utf8_lossy(&buffer),
                    )?;
                }
            }
        } else {
            let iter: Box<dyn Iterator<Item = Result<CommitId, RevsetEvaluationError>>> = {
                let forward_iter = revset.iter().take(args.limit.unwrap_or(usize::MAX));
                if args.reversed {
                    let entries: Vec<_> = forward_iter.try_collect()?;
                    Box::new(entries.into_iter().rev().map(Ok))
                } else {
                    Box::new(forward_iter)
                }
            };
            for commit_or_error in iter.commits(store) {
                let commit = commit_or_error?;
                with_content_format
                    .write(formatter, |formatter| template.format(&commit, formatter))?;
                if let Some(renderer) = &diff_renderer {
                    let width = ui.term_width();
                    renderer
                        .show_patch(ui, formatter, &commit, matcher.as_ref(), width)
                        .block_on()?;
                }

                let tree = commit.tree();
                // TODO: propagate errors
                explicit_paths.retain(|&path| tree.path_value(path).unwrap().is_absent());
            }
        }

        if !explicit_paths.is_empty() {
            let ui_paths = explicit_paths
                .iter()
                .map(|&path| workspace_command.format_file_path(path))
                .join(", ");
            writeln!(
                ui.warning_default(),
                "No matching entries for paths: {ui_paths}"
            )?;
        }
    }

    // Check to see if the user might have specified a path when they intended
    // to specify a revset.
    if let ([], [only_path]) = (args.revisions.as_slice(), args.paths.as_slice()) {
        if only_path == "." && workspace_command.parse_file_path(only_path)?.is_root() {
            // For users of e.g. Mercurial, where `.` indicates the current commit.
            writeln!(
                ui.warning_default(),
                "The argument {only_path:?} is being interpreted as a fileset expression, but \
                 this is often not useful because all non-empty commits touch '.'. If you meant \
                 to show the working copy commit, pass -r '@' instead."
            )?;
        } else if revset.is_empty()
            && workspace_command
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

    Ok(())
}
