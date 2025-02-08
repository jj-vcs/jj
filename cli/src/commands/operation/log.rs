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

use std::slice;

use clap_complete::ArgValueCandidates;
use indexmap::IndexMap;
use itertools::Itertools as _;
use jj_lib::config::ConfigGetError;
use jj_lib::config::ConfigGetResultExt as _;
use jj_lib::graph;
use jj_lib::graph::reverse_graph;
use jj_lib::graph::GraphEdge;
use jj_lib::graph::GraphEdgeType;
use jj_lib::op_walk;
use jj_lib::operation::Operation;
use jj_lib::repo::RepoLoader;
use jj_lib::settings::UserSettings;
use jj_lib::str_util::StringPattern;
use jj_lib::view::View;

use super::diff::show_op_diff;
use crate::cli_util::format_template;
use crate::cli_util::CommandHelper;
use crate::cli_util::LogContentFormat;
use crate::cli_util::WorkspaceCommandEnvironment;
use crate::command_error::CommandError;
use crate::commit_templater::CommitTemplateLanguage;
use crate::complete;
use crate::diff_util::diff_formats_for_log;
use crate::diff_util::DiffFormatArgs;
use crate::diff_util::DiffRenderer;
use crate::formatter::Formatter;
use crate::graphlog::get_graphlog;
use crate::graphlog::GraphStyle;
use crate::operation_templater::OperationTemplateLanguage;
use crate::ui::Ui;

/// Show the operation log
///
/// Like other commands, `jj op log` snapshots the current working-copy changes
/// and reconciles divergent operations. Use `--at-op=@ --ignore-working-copy`
/// to inspect the current state without mutation.
#[derive(clap::Args, Clone, Debug)]
pub struct OperationLogArgs {
    /// Limit number of operations to show
    ///
    /// Applied after operations are reordered topologically, but before being
    /// reversed.
    #[arg(long, short = 'n')]
    limit: Option<usize>,
    /// Show operations in the opposite order (older operations first)
    #[arg(long)]
    reversed: bool,
    /// Don't show the graph, show a flat list of operations
    #[arg(long)]
    no_graph: bool,
    /// Render each operation using the given template
    ///
    /// You can specify arbitrary [template expressions] using the
    /// [built-in keywords].
    ///
    /// [template expression]:
    ///     https://jj-vcs.github.io/jj/latest/templates/
    ///
    /// [built-in keywords]:
    ///     https://jj-vcs.github.io/jj/latest/templates/#operation-keywords
    #[arg(long, short = 'T', add = ArgValueCandidates::new(complete::template_aliases))]
    template: Option<String>,
    /// Show changes to the repository at each operation
    #[arg(long)]
    op_diff: bool,
    /// Display only operations which change the given local bookmark, or local
    /// bookmarks matching a pattern (can be repeated)
    #[arg(
        long, short,
        value_parser = StringPattern::parse,
        add = ArgValueCandidates::new(complete::local_bookmarks),
    )]
    bookmark: Vec<StringPattern>,
    /// Show patch of modifications to changes (implies --op-diff)
    ///
    /// If the previous version has different parents, it will be temporarily
    /// rebased to the parents of the new version, so the diff is not
    /// contaminated by unrelated changes.
    #[arg(long, short = 'p')]
    patch: bool,
    #[command(flatten)]
    diff_format: DiffFormatArgs,
}

pub fn cmd_op_log(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &OperationLogArgs,
) -> Result<(), CommandError> {
    if command.is_working_copy_writable() {
        let workspace_command = command.workspace_helper(ui)?;
        let current_op = workspace_command.repo().operation();
        let repo_loader = workspace_command.workspace().repo_loader();
        do_op_log(ui, workspace_command.env(), repo_loader, current_op, args)
    } else {
        // Don't load the repo so that the operation history can be inspected
        // even with a corrupted repo state. For example, you can find the first
        // bad operation id to be abandoned.
        let workspace = command.load_workspace()?;
        let workspace_env = command.workspace_environment(ui, &workspace)?;
        let repo_loader = workspace.repo_loader();
        let current_op = command.resolve_operation(ui, workspace.repo_loader())?;
        do_op_log(ui, &workspace_env, repo_loader, &current_op, args)
    }
}

fn do_op_log(
    ui: &mut Ui,
    workspace_env: &WorkspaceCommandEnvironment,
    repo_loader: &RepoLoader,
    current_op: &Operation,
    args: &OperationLogArgs,
) -> Result<(), CommandError> {
    let settings = repo_loader.settings();
    let graph_style = GraphStyle::from_settings(settings)?;
    let use_elided_nodes = settings.get_bool("ui.log-synthetic-elided-nodes")?;
    let with_content_format = LogContentFormat::new(ui, settings)?;

    let template;
    let op_node_template;
    {
        let language = OperationTemplateLanguage::new(
            repo_loader,
            Some(current_op.id()),
            workspace_env.operation_template_extensions(),
        );
        let text = match &args.template {
            Some(value) => value.to_owned(),
            None => settings.get_string("templates.op_log")?,
        };
        template = workspace_env
            .parse_template(
                ui,
                &language,
                &text,
                OperationTemplateLanguage::wrap_operation,
            )?
            .labeled("operation")
            .labeled("op_log");
        op_node_template = workspace_env
            .parse_template(
                ui,
                &language,
                &get_node_template(graph_style, settings)?,
                OperationTemplateLanguage::wrap_operation_opt,
            )?
            .labeled("node");
    }

    let diff_formats = diff_formats_for_log(settings, &args.diff_format, args.patch)?;
    let maybe_show_op_diff = if args.op_diff || !diff_formats.is_empty() {
        let template_text = settings.get_string("templates.commit_summary")?;
        let show = move |ui: &Ui,
                         formatter: &mut dyn Formatter,
                         op: &Operation,
                         with_content_format: &LogContentFormat| {
            let parents: Vec<_> = op.parents().try_collect()?;
            let parent_op = repo_loader.merge_operations(parents, None)?;
            let parent_repo = repo_loader.load_at(&parent_op)?;
            let repo = repo_loader.load_at(op)?;

            let id_prefix_context = workspace_env.new_id_prefix_context();
            let commit_summary_template = {
                let language =
                    workspace_env.commit_template_language(repo.as_ref(), &id_prefix_context);
                workspace_env.parse_template(
                    ui,
                    &language,
                    &template_text,
                    CommitTemplateLanguage::wrap_commit,
                )?
            };
            let path_converter = workspace_env.path_converter();
            let conflict_marker_style = workspace_env.conflict_marker_style();
            let diff_renderer = (!diff_formats.is_empty()).then(|| {
                DiffRenderer::new(
                    repo.as_ref(),
                    path_converter,
                    conflict_marker_style,
                    diff_formats.clone(),
                )
            });

            show_op_diff(
                ui,
                formatter,
                repo.as_ref(),
                &parent_repo,
                &repo,
                &commit_summary_template,
                (!args.no_graph).then_some(graph_style),
                with_content_format,
                diff_renderer.as_ref(),
            )
        };
        Some(show)
    } else {
        None
    };

    ui.request_pager();
    let mut formatter = ui.stdout_formatter();
    let formatter = formatter.as_mut();
    let limit = args.limit.unwrap_or(usize::MAX);

    let iter: Box<dyn Iterator<Item = Result<_, CommandError>>> = if args.bookmark.is_empty() {
        let iter = op_walk::walk_ancestors(slice::from_ref(current_op));
        let iter = iter
            .map(|op| -> Result<_, CommandError> {
                let op = op?;
                let ids = op.parent_ids();
                let edges = ids.iter().cloned().map(GraphEdge::direct).collect();
                Ok((op, edges))
            })
            .take(limit);
        Box::new(iter)
    } else {
        let ops = graph::topo_order_reverse_graph_filter_ok(
            slice::from_ref(current_op).iter().cloned().map(Ok),
            |op| op.id().clone(),
            |op| {
                let view = op.view()?;
                let parents: Vec<_> = op.parents().try_collect()?;
                // TODO: Fix error types?
                let parent_op = repo_loader.merge_operations(parents, None).unwrap();
                let parent_view = parent_op.view()?;

                let get_matching_bookmarks = |view: &View| {
                    let mut bookmarks = IndexMap::new();
                    for pattern in &args.bookmark {
                        let matches = view
                            .local_bookmarks_matching(pattern)
                            .map(|(name, targets)| (name.to_owned(), targets.to_owned()));
                        bookmarks.extend(matches);
                    }
                    bookmarks
                };

                let op_bookmarks = get_matching_bookmarks(&view);
                let parent_bookmarks = get_matching_bookmarks(&parent_view);

                Ok(op_bookmarks != parent_bookmarks)
            },
            |op| op.parents().collect_vec(),
        )?;
        let iter = ops.into_iter().map(Ok).take(limit);
        Box::new(iter)
    };

    if !args.no_graph {
        let mut raw_output = formatter.raw()?;
        let mut graph = get_graphlog(graph_style, raw_output.as_mut());
        let iter_nodes: Box<dyn Iterator<Item = _>> = if args.reversed {
            Box::new(reverse_graph(iter, Operation::id)?.into_iter().map(Ok))
        } else {
            Box::new(iter)
        };
        for node in iter_nodes {
            let (op, edges) = node?;

            // The graph is keyed by (OperationId, is_synthetic)
            let mut graphlog_edges = vec![];
            let mut missing_edge_id = None;
            let mut elided_targets = vec![];
            for edge in edges {
                let id = edge.target.clone();
                match edge.edge_type {
                    GraphEdgeType::Missing => {
                        missing_edge_id = Some(id);
                    }
                    GraphEdgeType::Direct => {
                        graphlog_edges.push(GraphEdge::direct((id, false)));
                    }
                    GraphEdgeType::Indirect => {
                        if use_elided_nodes {
                            elided_targets.push(id.clone());
                            graphlog_edges.push(GraphEdge::direct((id, true)));
                        } else {
                            graphlog_edges.push(GraphEdge::indirect((id, false)));
                        }
                    }
                }
            }
            if let Some(missing_edge_id) = missing_edge_id {
                graphlog_edges.push(GraphEdge::missing((missing_edge_id, false)));
            }
            let mut buffer = vec![];
            let key = (op.id().clone(), false);
            let within_graph = with_content_format.sub_width(graph.width(&key, &graphlog_edges));
            within_graph.write(ui.new_formatter(&mut buffer).as_mut(), |formatter| {
                template.format(&op, formatter)
            })?;
            if !buffer.ends_with(b"\n") {
                buffer.push(b'\n');
            }
            if let Some(show) = &maybe_show_op_diff {
                let mut formatter = ui.new_formatter(&mut buffer);
                show(ui, formatter.as_mut(), &op, &within_graph)?;
            }
            let node_symbol = format_template(ui, &Some(op), &op_node_template);
            graph.add_node(
                &key,
                &graphlog_edges,
                &node_symbol,
                &String::from_utf8_lossy(&buffer),
            )?;

            for elided_target in elided_targets {
                let elided_key = (elided_target, true);
                let real_key = (elided_key.0.clone(), false);
                let edges = [GraphEdge::direct(real_key)];
                let mut buffer = vec![];
                let within_graph = with_content_format.sub_width(graph.width(&elided_key, &edges));
                within_graph.write(ui.new_formatter(&mut buffer).as_mut(), |formatter| {
                    writeln!(formatter.labeled("elided"), "(elided revisions)")
                })?;
                let node_symbol = format_template(ui, &None, &op_node_template);
                graph.add_node(
                    &elided_key,
                    &edges,
                    &node_symbol,
                    &String::from_utf8_lossy(&buffer),
                )?;
            }
        }
    } else {
        let iter: Box<dyn Iterator<Item = _>> = if args.reversed {
            Box::new(iter.collect_vec().into_iter().rev())
        } else {
            Box::new(iter)
        };
        for op in iter {
            let (op, _) = op?;
            with_content_format.write(formatter, |formatter| template.format(&op, formatter))?;
            if let Some(show) = &maybe_show_op_diff {
                show(ui, formatter, &op, &with_content_format)?;
            }
        }
    }

    Ok(())
}

fn get_node_template(style: GraphStyle, settings: &UserSettings) -> Result<String, ConfigGetError> {
    let symbol = settings.get_string("templates.op_log_node").optional()?;
    let default = if style.is_ascii() {
        "builtin_op_log_node_ascii"
    } else {
        "builtin_op_log_node"
    };
    Ok(symbol.unwrap_or_else(|| default.to_owned()))
}
