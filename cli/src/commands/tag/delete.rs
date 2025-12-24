// Copyright 2025 The Jujutsu Authors
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
use itertools::Itertools as _;
use jj_lib::op_store::RefTarget;

use super::warn_unmatched_local_tags;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::complete;
use crate::revset_util::parse_union_name_patterns;
use crate::ui::Ui;

/// Delete existing tags
///
/// Revisions referred to by the deleted tags are not abandoned.
#[derive(clap::Args, Clone, Debug)]
pub struct TagDeleteArgs {
    /// Tag names to delete
    ///
    /// By default, the specified pattern matches tag names with glob syntax.
    /// You can also use other [string pattern syntax].
    ///
    /// [string pattern syntax]:
    ///     https://docs.jj-vcs.dev/latest/revsets/#string-patterns
    #[arg(required = true)]
    #[arg(add = ArgValueCandidates::new(complete::local_tags))]
    names: Vec<String>,
}

pub fn cmd_tag_delete(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &TagDeleteArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo().clone();
    let name_expr = parse_union_name_patterns(ui, &args.names)?;
    let name_matcher = name_expr.to_matcher();
    let matched_tags = repo.view().local_tags_matching(&name_matcher).collect_vec();
    warn_unmatched_local_tags(ui, repo.view(), &name_expr)?;
    if matched_tags.is_empty() {
        writeln!(ui.status(), "No tags to delete.")?;
        return Ok(());
    }

    let mut tx = workspace_command.start_transaction();
    for (name, _) in &matched_tags {
        tx.repo_mut()
            .set_local_tag_target(name, RefTarget::absent());
    }
    writeln!(ui.status(), "Deleted {} tags.", matched_tags.len())?;
    tx.finish(
        ui,
        format!(
            "delete tag {names}",
            names = matched_tags.iter().map(|(n, _)| n.as_symbol()).join(", ")
        ),
    )?;
    Ok(())
}
