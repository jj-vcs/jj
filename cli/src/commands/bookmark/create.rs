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

use clap_complete::ArgValueCandidates;
use clap_complete::ArgValueCompleter;
use itertools::Itertools as _;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_store::RefTarget;
use jj_lib::ref_name::RefNameBuf;

use crate::cli_util::has_tracked_remote_bookmarks;
use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::user_error_with_hint;
use crate::command_error::CommandError;
use crate::complete;
use crate::revset_util;
use crate::ui::Ui;

/// Create a new bookmark
#[derive(clap::Args, Clone, Debug)]
pub struct BookmarkCreateArgs {
    // TODO(#5374): Make required in jj 0.32+
    /// The bookmark's target revision
    //
    // The `--to` alias exists for making it easier for the user to switch
    // between `bookmark create`, `bookmark move`, and `bookmark set`. Currently target revision
    // defaults to the working copy if not specified, but in the near future it will be required to
    // explicitly specify it.
    #[arg(
        long, short,
        visible_alias = "to",
        value_name = "REVSET",
        add = ArgValueCompleter::new(complete::revset_expression_all),
    )]
    revision: Option<RevisionArg>,

    /// The bookmarks to create
    #[arg(
        required = true,
        value_parser = revset_util::parse_bookmark_name,
        add = ArgValueCandidates::new(complete::bookmarks),
    )]
    names: Vec<RefNameBuf>,
}

pub fn cmd_bookmark_create(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BookmarkCreateArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    if args.revision.is_none() {
        writeln!(
            ui.warning_default(),
            "Target revision was not specified, defaulting to the working copy (-r@). In the near \
             future it will be required to explicitly specify target revision."
        )?;
    }
    let target_commit = workspace_command
        .resolve_single_rev(ui, args.revision.as_ref().unwrap_or(&RevisionArg::AT))?;
    let view = workspace_command.repo().view();
    let bookmark_names = &args.names;
    for name in bookmark_names {
        if view.get_local_bookmark(name).is_present() {
            return Err(user_error_with_hint(
                format!("Bookmark already exists: {name}", name = name.as_symbol()),
                "Use `jj bookmark set` to update it.",
            ));
        }
        if has_tracked_remote_bookmarks(view, name) {
            return Err(user_error_with_hint(
                format!(
                    "Tracked remote bookmarks exist for deleted bookmark: {name}",
                    name = name.as_symbol()
                ),
                format!(
                    "Use `jj bookmark set` to recreate the local bookmark. Run `jj bookmark \
                     untrack 'glob:{name}@*'` to disassociate them.",
                    name = name.as_symbol()
                ),
            ));
        }
    }

    let mut tx = workspace_command.start_transaction();
    for name in bookmark_names {
        tx.repo_mut()
            .set_local_bookmark_target(name, RefTarget::normal(target_commit.id().clone()));
    }

    if let Some(mut formatter) = ui.status_formatter() {
        write!(
            formatter,
            "Created {} bookmarks pointing to ",
            bookmark_names.len()
        )?;
        tx.write_commit_summary(formatter.as_mut(), &target_commit)?;
        writeln!(formatter)?;
    }
    tx.finish(
        ui,
        format!(
            "create bookmark {names} pointing to commit {id}",
            names = bookmark_names.iter().map(|n| n.as_symbol()).join(", "),
            id = target_commit.id().hex()
        ),
    )?;
    Ok(())
}
