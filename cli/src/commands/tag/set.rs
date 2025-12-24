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
use clap_complete::ArgValueCompleter;
use itertools::Itertools as _;
use jj_lib::op_store::RefTarget;
use jj_lib::ref_name::RefNameBuf;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::command_error::user_error_with_hint;
use crate::complete;
use crate::revset_util;
use crate::ui::Ui;

/// Create or update tags
#[derive(clap::Args, Clone, Debug)]
pub struct TagSetArgs {
    /// Target revision to point to
    #[arg(
        long,
        short,
        default_value = "@",
        visible_alias = "to",
        value_name = "REVSET"
    )]
    #[arg(add = ArgValueCompleter::new(complete::revset_expression_all))]
    revision: RevisionArg,

    /// Allow moving existing tags
    #[arg(long)]
    allow_move: bool,

    /// Tag names to create or update
    #[arg(required = true, value_parser = revset_util::parse_tag_name)]
    #[arg(add = ArgValueCandidates::new(complete::local_tags))]
    names: Vec<RefNameBuf>,
}

pub fn cmd_tag_set(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &TagSetArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let target_commit = workspace_command.resolve_single_rev(ui, &args.revision)?;
    let repo = workspace_command.repo().as_ref();

    let mut new_count = 0;
    let mut moved_count = 0;
    for name in &args.names {
        let old_target = repo.view().get_local_tag(name);
        // TODO: If we add support for tracked remote tags, deleted tags should
        // be considered present until they get pushed. See cmd_bookmark_set().
        if old_target.is_present() && !args.allow_move {
            return Err(user_error_with_hint(
                format!("Refusing to move tag: {name}", name = name.as_symbol()),
                "Use --allow-move to update existing tags.",
            ));
        }
        if old_target.is_absent() {
            new_count += 1;
        } else if old_target.as_normal() != Some(target_commit.id()) {
            moved_count += 1;
        }
    }
    if target_commit.is_discardable(repo)? {
        writeln!(ui.warning_default(), "Target revision is empty.")?;
    }

    let mut tx = workspace_command.start_transaction();
    for name in &args.names {
        tx.repo_mut()
            .set_local_tag_target(name, RefTarget::normal(target_commit.id().clone()));
    }

    if let Some(mut formatter) = ui.status_formatter() {
        if new_count > 0 {
            write!(formatter, "Created {new_count} tags pointing to ")?;
            tx.write_commit_summary(formatter.as_mut(), &target_commit)?;
            writeln!(formatter)?;
        }
        if moved_count > 0 {
            write!(formatter, "Moved {moved_count} tags to ")?;
            tx.write_commit_summary(formatter.as_mut(), &target_commit)?;
            writeln!(formatter)?;
        }
    }

    tx.finish(
        ui,
        format!(
            "set tag {names} to commit {id}",
            names = args.names.iter().map(|n| n.as_symbol()).join(", "),
            id = target_commit.id()
        ),
    )?;
    Ok(())
}
