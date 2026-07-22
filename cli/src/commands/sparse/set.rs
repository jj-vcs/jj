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

use jj_lib::fileset::FilesetExpression;
use tracing::instrument;

use super::update_sparse_patterns_with;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Update the patterns that are present in the working copy
///
/// For example, if all you need is the `README.md` and the `lib/`
/// directory, use `jj sparse set --clear --add README.md --add lib`.
/// If you no longer need the `lib` directory, use `jj sparse set --remove lib`.
#[derive(clap::Args, Clone, Debug)]
pub struct SparseSetArgs {
    /// Fileset expressions to add to the working copy
    #[arg(long, value_name = "FILESETS", value_hint = clap::ValueHint::AnyPath)]
    add: Vec<String>,

    /// Fileset expressions to remove from the working copy
    #[arg(
        long,
        conflicts_with = "clear",
        value_name = "FILESETS",
        value_hint = clap::ValueHint::AnyPath
    )]
    remove: Vec<String>,

    /// Include no files in the working copy (combine with --add)
    #[arg(long)]
    clear: bool,

    /// Fileset expressions to set in the working copy
    #[arg(
        value_name = "FILESETS",
        conflicts_with = "clear",
        value_hint = clap::ValueHint::AnyPath
    )]
    filesets: Vec<String>,
}

#[instrument(skip_all)]
pub async fn cmd_sparse_set(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &SparseSetArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui).await?;
    let fileset_expr = if !args.filesets.is_empty() {
        Some(workspace_command.parse_union_filesets(ui, &args.filesets)?)
    } else {
        None
    };
    let add_expr = if !args.add.is_empty() {
        Some(workspace_command.parse_union_filesets(ui, &args.add)?)
    } else {
        None
    };
    let remove_expr = if !args.remove.is_empty() {
        Some(workspace_command.parse_union_filesets(ui, &args.remove)?)
    } else {
        None
    };
    update_sparse_patterns_with(ui, &mut workspace_command, |_ui, old_patterns| {
        let mut expr = if let Some(base) = fileset_expr {
            base
        } else if args.clear {
            FilesetExpression::none()
        } else {
            old_patterns.clone()
        };
        if let Some(add) = add_expr {
            if matches!(expr, FilesetExpression::None) {
                expr = add;
            } else if !matches!(expr, FilesetExpression::All) {
                expr = FilesetExpression::union_all(vec![expr, add]);
            }
        }
        if let Some(remove) = remove_expr
            && !matches!(expr, FilesetExpression::None)
        {
            expr = FilesetExpression::difference(expr, remove);
        }
        Ok(expr)
    })
    .await
}
