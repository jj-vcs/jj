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

use std::cmp::min;

use clap_complete::ArgValueCandidates;
use clap_complete::ArgValueCompleter;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::stream;
use futures::stream::LocalBoxStream;
use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::graph::GraphEdge;
use jj_lib::graph::GraphEdgeType;
use jj_lib::graph::GraphNode;
use jj_lib::graph::TopoGroupedGraph;
use jj_lib::graph::reverse_graph;
use jj_lib::repo::Repo as _;
use jj_lib::revset::RevsetEvaluationError;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::RevsetFilterPredicate;
use jj_lib::revset::RevsetStreamExt as _;
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
    #[arg(long = "revision", short, value_name = "REVSETS", alias = "revisions")]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    revisions: Vec<RevisionArg>,

    /// Show revisions modifying the given paths
    #[arg(value_name = "FILESETS", value_hint = clap::ValueHint::AnyPath)]
    #[arg(add = ArgValueCompleter::new(complete::log_files))]
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
    #[arg(long, short = 'T')]
    #[arg(add = ArgValueCandidates::new(complete::template_aliases))]
    template: Option<String>,

    /// Show patch
    #[arg(long, short = 'p')]
    patch: bool,

    /// Print the number of commits instead of showing them
    #[arg(long, conflicts_with_all = ["DiffFormatArgs", "no_graph", "patch", "reversed", "template"])]
    count: bool,

    #[command(flatten)]
    diff_format: DiffFormatArgs,
}

#[instrument(skip_all)]
pub(crate) async fn cmd_log(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &LogArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui).await?;
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

    let revset = revset_expression.evaluate()?;

    if args.count {
        let (lower, upper) = revset.count_estimate()?;
        let limit = args.limit.unwrap_or(usize::MAX);
        let count = if limit <= lower {
            limit
        } else if upper == Some(lower) {
            min(lower, limit)
        } else {
            revset
                .stream()
                .take(limit)
                .try_fold(0, |count, _| async move { Ok(count + 1) })
                .await?
        };
        let mut formatter = ui.stdout_formatter();
        writeln!(formatter, "{count}")?;
        return Ok(());
    }

    let prio_revset = settings.get_string("revsets.log-graph-prioritize")?;
    let mut prio_revset = workspace_command.parse_revset(ui, &RevisionArg::from(prio_revset))?;
    prio_revset.intersect_with(revset_expression.expression());

    let repo = workspace_command.repo();
    let matcher = fileset_expression.to_matcher();

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
            let mut stream: LocalBoxStream<_> = {
                let mut topo_order = TopoGroupedGraph::new(revset.stream_graph(), |id| id);

                let mut prio_stream = prio_revset.evaluate_to_commit_ids()?;
                while let Some(prio) = prio_stream.try_next().await? {
                    topo_order.prioritize_branch(prio);
                }
                let forward_stream = topo_order.stream();

                // The input to TopoGroupedGraph shouldn't be truncated because the prioritized
                // commit must exist in the input set.
                let forward_stream = forward_stream.take(args.limit.unwrap_or(usize::MAX));

                let display_stream =
                    edges_to_display(forward_stream.boxed_local(), use_elided_nodes)
                        .map(|r| r.map_err(CommandError::from));

                if args.reversed {
                    let nodes: Vec<_> = display_stream.collect().await;
                    let nodes = reverse_graph(nodes.into_iter(), |id| id)?;
                    stream::iter(nodes.into_iter().map(Ok)).boxed_local()
                } else {
                    display_stream.boxed_local()
                }
            };
            while let Some((node, edges)) = stream.try_next().await? {
                let mut label_buffer = vec![];
                let commit = match &node {
                    DisplayNode::Present(commit_id) => {
                        let commit = store.get_commit_async(commit_id).await?;
                        let within_graph =
                            with_content_format.sub_width(graph.width(&node, &edges));
                        within_graph
                            .write(
                                ui.new_formatter(&mut label_buffer).as_mut(),
                                async |formatter| template.format(&commit, formatter),
                            )
                            .await?;
                        if let Some(renderer) = &diff_renderer {
                            let mut formatter = ui.new_formatter(&mut label_buffer);
                            renderer
                                .show_patch(
                                    ui,
                                    formatter.as_mut(),
                                    &commit,
                                    matcher.as_ref(),
                                    within_graph.width(),
                                )
                                .await?;
                        }

                        let tree = commit.tree();
                        // TODO: propagate errors
                        explicit_paths
                            .retain(|&path| tree.path_value(path).block_on().unwrap().is_absent());

                        Some(commit)
                    }
                    DisplayNode::IndirectPath { .. } => {
                        // Give the (elided revisions) label only for intermediate missing nodes
                        let within_graph =
                            with_content_format.sub_width(graph.width(&node, &edges));
                        within_graph
                            .write(
                                ui.new_formatter(&mut label_buffer).as_mut(),
                                async |formatter| {
                                    writeln!(formatter.labeled("elided"), "(elided revisions)")
                                },
                            )
                            .await?;
                        None
                    }
                    DisplayNode::MissingParentsOf(..) => None,
                };

                let node_symbol = format_template(ui, &commit, &node_template);
                graph.add_node(
                    &node,
                    &edges,
                    &node_symbol,
                    &String::from_utf8_lossy(&label_buffer),
                )?;
            }
        } else {
            let id_stream: LocalBoxStream<Result<CommitId, RevsetEvaluationError>> = {
                let forward_stream = revset.stream().take(args.limit.unwrap_or(usize::MAX));
                if args.reversed {
                    let entries: Vec<_> = forward_stream.try_collect().await?;
                    stream::iter(entries.into_iter().rev().map(Ok)).boxed_local()
                } else {
                    forward_stream.boxed_local()
                }
            };
            let mut commit_stream = id_stream.commits(store);
            while let Some(commit) = commit_stream.try_next().await? {
                with_content_format
                    .write(formatter, async |formatter| {
                        template.format(&commit, formatter)
                    })
                    .await?;
                if let Some(renderer) = &diff_renderer {
                    let width = ui.term_width();
                    renderer
                        .show_patch(ui, formatter, &commit, matcher.as_ref(), width)
                        .await?;
                }

                let tree = commit.tree();
                // TODO: propagate errors
                explicit_paths
                    .retain(|&path| tree.path_value(path).block_on().unwrap().is_absent());
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

/// Indicates how the graph node should be displayed.
#[derive(Clone, PartialEq, Eq, Hash)]
enum DisplayNode {
    /// Prints the commit normally
    Present(CommitId),
    /// Prints the "(elided revisions)" between child and (present) ancestor
    IndirectPath { child: CommitId, ancestor: CommitId },
    /// Prints the representation of all elided parents of the commit
    MissingParentsOf(CommitId),
}

/// Convert a revision graph into a display-suitable format.
///
/// This method creates synthetic nodes to allow clearly differentiating elided
/// revisions and showing elided roots (otherwise not possible in reversed mode
/// due to limitations of sapling-renderdag).
fn edges_to_display<'a, E: 'a>(
    input: impl stream::Stream<Item = Result<GraphNode<CommitId, CommitId>, E>> + 'a,
    use_elided_nodes: bool,
) -> impl stream::Stream<Item = Result<GraphNode<DisplayNode, DisplayNode>, E>> + 'a {
    stream::unfold(
        (
            std::collections::VecDeque::<GraphNode<DisplayNode, DisplayNode>>::new(),
            Box::pin(input.fuse()),
        ),
        move |(mut pending, mut input)| async move {
            if let Some(node) = pending.pop_front() {
                return Some((Ok(node), (pending, input)));
            }
            let item = input.next().await?;
            let (node, edges) = match item {
                Ok(v) => v,
                Err(e) => return Some((Err(e), (pending, input))),
            };
            let mut has_missing_parents = false;
            let mut new_edges = Vec::with_capacity(edges.len());
            for edge in edges {
                match edge.edge_type {
                    GraphEdgeType::Direct => {
                        new_edges.push(GraphEdge::direct(DisplayNode::Present(edge.target)));
                    }
                    GraphEdgeType::Indirect => {
                        if !use_elided_nodes {
                            // print the elided nodes using a dashed line
                            new_edges.push(GraphEdge::indirect(DisplayNode::Present(edge.target)));
                        } else {
                            // create a synthetic elided nodes, use direct edges
                            let synthetic_node = DisplayNode::IndirectPath {
                                child: node.clone(),
                                ancestor: edge.target.clone(),
                            };
                            let real_target = DisplayNode::Present(edge.target);
                            pending.push_back((
                                synthetic_node.clone(),
                                vec![GraphEdge::direct(real_target)],
                            ));
                            new_edges.push(GraphEdge::direct(synthetic_node));
                        }
                    }
                    GraphEdgeType::Missing => {
                        if use_elided_nodes && !has_missing_parents {
                            // print missing parents as a single elided node
                            has_missing_parents = true;
                            let node = DisplayNode::MissingParentsOf(node.clone());
                            pending.push_back((node.clone(), vec![]));
                            new_edges.push(GraphEdge::direct(node));
                        }
                    }
                }
            }
            Some((
                Ok((DisplayNode::Present(node), new_edges)),
                (pending, input),
            ))
        },
    )
}
