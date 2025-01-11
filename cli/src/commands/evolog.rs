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

use std::convert::Infallible;

use clap_complete::ArgValueCandidates;
use itertools::Itertools;
use jj_lib::backend::BackendError;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::dag_walk::topo_order_reverse_ok;
use jj_lib::graph::reverse_graph;
use jj_lib::graph::GraphEdge;
use jj_lib::graph::GraphNode;
use jj_lib::matchers::EverythingMatcher;
use tracing::instrument;

use super::log::get_node_template;
use crate::cli_util::format_template;
use crate::cli_util::CommandHelper;
use crate::cli_util::LogContentFormat;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::commit_templater::CommitTemplateLanguage;
use crate::complete;
use crate::diff_util::DiffFormatArgs;
use crate::graphlog::convert_edge_type;
use crate::graphlog::get_graphlog;
use crate::graphlog::Edge;
use crate::graphlog::GraphStyle;
use crate::ui::Ui;

/// Show how a change has evolved over time
///
/// Lists the previous commits which a change has pointed to. The current commit
/// of a change evolves when the change is updated, rebased, etc.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct EvologArgs {
    #[arg(
        long, short,
        default_value = "@",
        value_name = "REVSET",
        add = ArgValueCandidates::new(complete::all_revisions),
    )]
    revision: RevisionArg,
    /// Limit number of revisions to show
    #[arg(long, short = 'n')]
    limit: Option<usize>,
    /// Show revisions in the opposite order (older revisions first)
    #[arg(long)]
    reversed: bool,
    /// Don't show the graph, show a flat list of revisions
    #[arg(long)]
    no_graph: bool,
    /// Render each revision using the given template
    ///
    /// For the syntax, see https://jj-vcs.github.io/jj/latest/templates/
    #[arg(long, short = 'T')]
    template: Option<String>,
    /// Show patch compared to the previous version of this change
    ///
    /// If the previous version has different parents, it will be temporarily
    /// rebased to the parents of the new version, so the diff is not
    /// contaminated by unrelated changes.
    #[arg(long, short = 'p')]
    patch: bool,
    #[command(flatten)]
    diff_format: DiffFormatArgs,
}

#[instrument(skip_all)]
pub(crate) fn cmd_evolog(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &EvologArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;

    let start_commit = workspace_command.resolve_single_rev(ui, &args.revision)?;

    let diff_renderer = workspace_command.diff_renderer_for_log(&args.diff_format, args.patch)?;
    let graph_style = GraphStyle::from_settings(workspace_command.settings())?;
    let with_content_format = LogContentFormat::new(ui, workspace_command.settings())?;

    let template;
    let node_template;
    {
        let language = workspace_command.commit_template_language();
        let template_string = match &args.template {
            Some(value) => value.to_string(),
            None => workspace_command.settings().get_string("templates.log")?,
        };
        template = workspace_command
            .parse_template(
                ui,
                &language,
                &template_string,
                CommitTemplateLanguage::wrap_commit,
            )?
            .labeled("log");
        node_template = workspace_command
            .parse_template(
                ui,
                &language,
                &get_node_template(graph_style, workspace_command.settings())?,
                CommitTemplateLanguage::wrap_commit_opt,
            )?
            .labeled("node");
    }

    ui.request_pager("evolog");
    let mut formatter = ui.stdout_formatter();
    let formatter = formatter.as_mut();

    let mut commits = topo_order_reverse_ok(
        vec![Ok(start_commit)],
        |commit: &Commit| commit.id().clone(),
        |commit: &Commit| {
            let mut predecessors = commit.predecessors().collect_vec();
            // Predecessors don't need to follow any defined order. However in
            // practice, if there are multiple predecessors, then usually the
            // first predecessor is the previous version of the same change, and
            // the other predecessors are commits that were squashed into it. If
            // multiple commits are squashed at once, then they are usually
            // recorded in chronological order. We want to show squashed commits
            // in reverse chronological order, and we also want to show squashed
            // commits before the squash destination (since the destination's
            // subgraph may contain earlier squashed commits as well), so we
            // visit the predecessors in reverse order.
            predecessors.reverse();
            predecessors
        },
    )?;
    if let Some(n) = args.limit {
        commits.truncate(n);
    }
    if !args.no_graph {
        let mut raw_output = formatter.raw()?;
        let mut graph = get_graphlog(graph_style, raw_output.as_mut());

        let commit_dag: Vec<GraphNode<Commit>> = commits
            .into_iter()
            .map(|c| -> Result<_, BackendError> {
                let edges = c.predecessors().map_ok(GraphEdge::direct).try_collect()?;
                Ok((c, edges))
            })
            .try_collect()?;

        let iter_nodes = if args.reversed {
            reverse_graph(commit_dag.into_iter().map(Result::<_, Infallible>::Ok)).unwrap()
        } else {
            commit_dag
        };

        for node in iter_nodes {
            let (commit, edges) = node;
            // Slightly different edge type used for display.
            let graphlog_edges: Vec<Edge<CommitId>> = edges
                .into_iter()
                .map(|e| convert_edge_type(e).map(|e| e.id().clone()))
                .collect();
            let mut buffer = vec![];
            let within_graph =
                with_content_format.sub_width(graph.width(commit.id(), &graphlog_edges));
            within_graph.write(ui.new_formatter(&mut buffer).as_mut(), |formatter| {
                template.format(&commit, formatter)
            })?;
            if !buffer.ends_with(b"\n") {
                buffer.push(b'\n');
            }
            if let Some(renderer) = &diff_renderer {
                let predecessors: Vec<_> = commit.predecessors().try_collect()?;
                let mut formatter = ui.new_formatter(&mut buffer);
                renderer.show_inter_diff(
                    ui,
                    formatter.as_mut(),
                    &predecessors,
                    &commit,
                    &EverythingMatcher,
                    within_graph.width(),
                )?;
            }
            let node_symbol = format_template(ui, &Some(commit.clone()), &node_template);
            graph.add_node(
                commit.id(),
                &graphlog_edges,
                &node_symbol,
                &String::from_utf8_lossy(&buffer),
            )?;
        }
    } else {
        if args.reversed {
            commits.reverse();
        }

        for commit in commits {
            with_content_format
                .write(formatter, |formatter| template.format(&commit, formatter))?;
            if let Some(renderer) = &diff_renderer {
                let predecessors: Vec<_> = commit.predecessors().try_collect()?;
                let width = ui.term_width();
                renderer.show_inter_diff(
                    ui,
                    formatter,
                    &predecessors,
                    &commit,
                    &EverythingMatcher,
                    width,
                )?;
            }
        }
    }

    Ok(())
}
