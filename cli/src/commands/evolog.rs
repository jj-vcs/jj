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
use jj_lib::commit::Commit;
use jj_lib::evolution::CommitEvolutionEntry;
use jj_lib::evolution::walk_predecessors;
use jj_lib::graph::GraphEdge;
use jj_lib::graph::TopoGroupedGraphIterator;
use jj_lib::graph::reverse_graph;
use jj_lib::matchers::EverythingMatcher;
use pollster::FutureExt as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::LogContentFormat;
use crate::cli_util::RevisionArg;
use crate::cli_util::format_template;
use crate::command_error::CommandError;
use crate::complete;
use crate::diff_util::DiffFormatArgs;
use crate::graphlog::GraphStyle;
use crate::graphlog::get_graphlog;
use crate::templater::TemplateRenderer;
use crate::ui::Ui;

/// Show how a change has evolved over time
///
/// Lists the previous commits which a change has pointed to. The current commit
/// of a change evolves when the change is updated, rebased, etc.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct EvologArgs {
    /// Follow changes from these revisions
    #[arg(
        long,
        short,
        default_value = "@",
        value_name = "REVSETS",
        alias = "revision"
    )]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    revisions: Vec<RevisionArg>,

    /// Limit number of revisions to show
    ///
    /// Applied after revisions are reordered topologically, but before being
    /// reversed.
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
    /// All 0-argument methods of the [`CommitEvolutionEntry` type] are
    /// available as keywords in the template expression. See [`jj help -k
    /// templates`] for more information.
    ///
    /// If not specified, this defaults to the `templates.evolog` setting.
    ///
    /// [`CommitEvolutionEntry` type]:
    ///     https://docs.jj-vcs.dev/latest/templates/#commitevolutionentry-type
    ///
    /// [`jj help -k templates`]:
    ///     https://docs.jj-vcs.dev/latest/templates/
    #[arg(long, short = 'T')]
    #[arg(add = ArgValueCandidates::new(complete::template_aliases))]
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

    let start_commit_ids: Vec<_> = workspace_command
        .parse_union_revsets(ui, &args.revisions)?
        .evaluate_to_commit_ids()?
        .try_collect()?;

    let diff_renderer = workspace_command.diff_renderer_for_log(&args.diff_format, args.patch)?;
    let graph_style = GraphStyle::from_settings(workspace_command.settings())?;
    let with_content_format = LogContentFormat::new(ui, workspace_command.settings())?;

    let template: TemplateRenderer<CommitEvolutionEntry>;
    let node_template: TemplateRenderer<Option<Commit>>;
    {
        let language = workspace_command.commit_template_language();
        let template_string = match &args.template {
            Some(value) => value.clone(),
            None => workspace_command.settings().get("templates.evolog")?,
        };
        template = workspace_command
            .parse_template(ui, &language, &template_string)?
            .labeled(["evolog"]); // TODO: add label for the context type?
        node_template = workspace_command
            .parse_template(
                ui,
                &language,
                // TODO: should we add templates.evolog_node?
                &workspace_command
                    .settings()
                    .get_string("templates.log_node")?,
            )?
            .labeled(["evolog", "commit", "node"]);
    }

    ui.request_pager();
    let mut formatter = ui.stdout_formatter();
    let formatter = formatter.as_mut();

    let repo = workspace_command.repo();
    let evolution_entries = walk_predecessors(repo, &start_commit_ids);
    if !args.no_graph {
        let mut raw_output = formatter.raw()?;
        let mut graph = get_graphlog(graph_style, raw_output.as_mut());

        let evolution_nodes = evolution_entries.map_ok(|entry| {
            let ids = entry.predecessor_ids();
            let edges = ids.iter().cloned().map(GraphEdge::direct).collect_vec();
            (entry, edges)
        });
        // TopoGroupedGraphIterator also helps emit squashed commits in reverse
        // chronological order. Predecessors don't need to follow any defined
        // order. However in practice, if there are multiple predecessors, then
        // usually the first predecessor is the previous version of the same
        // change, and the other predecessors are commits that were squashed
        // into it. If multiple commits are squashed at once, then they are
        // usually recorded in chronological order. We want to show squashed
        // commits in reverse chronological order, and we also want to show
        // squashed commits before the squash destination (since the
        // destination's subgraph may contain earlier squashed commits as well.
        let evolution_nodes =
            TopoGroupedGraphIterator::new(evolution_nodes, |node| node.commit.id());

        let evolution_nodes = evolution_nodes.take(args.limit.unwrap_or(usize::MAX));
        let evolution_nodes: Box<dyn Iterator<Item = _>> = if args.reversed {
            let nodes = reverse_graph(evolution_nodes, |entry| entry.commit.id())?;
            Box::new(nodes.into_iter().map(Ok))
        } else {
            Box::new(evolution_nodes)
        };

        for node in evolution_nodes {
            let (entry, edges) = node?;
            let mut buffer = vec![];
            let within_graph =
                with_content_format.sub_width(graph.width(entry.commit.id(), &edges));
            within_graph.write(ui.new_formatter(&mut buffer).as_mut(), |formatter| {
                template.format(&entry, formatter)
            })?;
            if let Some(renderer) = &diff_renderer {
                let predecessors: Vec<_> = entry.predecessors().try_collect()?;
                let mut formatter = ui.new_formatter(&mut buffer);
                renderer
                    .show_inter_diff(
                        ui,
                        formatter.as_mut(),
                        &predecessors,
                        &entry.commit,
                        &EverythingMatcher,
                        within_graph.width(),
                    )
                    .block_on()?;
            }
            let node_symbol = format_template(ui, &Some(entry.commit.clone()), &node_template);
            graph.add_node(
                entry.commit.id(),
                &edges,
                &node_symbol,
                &String::from_utf8_lossy(&buffer),
            )?;
        }
    } else {
        let evolution_entries = evolution_entries.take(args.limit.unwrap_or(usize::MAX));
        let evolution_entries: Box<dyn Iterator<Item = _>> = if args.reversed {
            let entries: Vec<_> = evolution_entries.try_collect()?;
            Box::new(entries.into_iter().rev().map(Ok))
        } else {
            Box::new(evolution_entries)
        };

        for entry in evolution_entries {
            let entry = entry?;
            with_content_format.write(formatter, |formatter| template.format(&entry, formatter))?;
            if let Some(renderer) = &diff_renderer {
                let predecessors: Vec<_> = entry.predecessors().try_collect()?;
                let width = ui.term_width();
                renderer
                    .show_inter_diff(
                        ui,
                        formatter,
                        &predecessors,
                        &entry.commit,
                        &EverythingMatcher,
                        width,
                    )
                    .block_on()?;
            }
        }
    }

    Ok(())
}
