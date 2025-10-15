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
use jj_lib::str_util::StringPattern;

use super::find_local_tags;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::complete;
use crate::ui::Ui;

/// Delete existing tags
///
/// Revisions referred to by the deleted tags are not abandoned.
#[derive(clap::Args, Clone, Debug)]
pub struct TagDeleteArgs {
    /// Tag names to delete
    ///
    /// By default, the specified name matches exactly. Use `glob:` prefix to
    /// select tags by [wildcard pattern].
    ///
    /// [wildcard pattern]:
    ///     https://jj-vcs.github.io/jj/latest/revsets/#string-patterns
    #[arg(
        required = true,
        value_parser = StringPattern::parse,
        add = ArgValueCandidates::new(complete::local_tags),
    )]
    names: Vec<StringPattern>,
}

pub fn cmd_tag_delete(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &TagDeleteArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo().clone();
    let matched_tags = find_local_tags(repo.view(), &args.names)?;
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
