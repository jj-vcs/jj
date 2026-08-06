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

use std::collections::HashSet;
use std::io;
use std::io::Read as _;

use itertools::Itertools as _;
use jj_lib::repo_path::RepoPathBuf;
use tracing::instrument;

use super::update_sparse_patterns_with;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error_with_message;
use crate::ui::Ui;

/// Update the patterns that are present in the working copy
///
/// For example, if all you need is the `README.md` and the `lib/`
/// directory, use `jj sparse set --clear --add README.md --add lib`.
/// If you no longer need the `lib` directory, use `jj sparse set --remove lib`.
#[derive(clap::Args, Clone, Debug)]
pub struct SparseSetArgs {
    /// Patterns to add to the working copy
    #[arg(
        long,
        value_hint = clap::ValueHint::AnyPath,
        value_parser = |s: &str| RepoPathBuf::from_relative_path(s),
    )]
    add: Vec<RepoPathBuf>,

    /// Patterns to remove from the working copy
    #[arg(
        long,
        conflicts_with = "clear",
        value_hint = clap::ValueHint::AnyPath,
        value_parser = |s: &str| RepoPathBuf::from_relative_path(s),
    )]
    remove: Vec<RepoPathBuf>,

    /// Include no files in the working copy (combine with --add)
    #[arg(long)]
    clear: bool,

    /// Read the replacement sparse patterns from stdin, one path per line
    #[arg(long)]
    stdin: bool,
}

#[instrument(skip_all)]
pub async fn cmd_sparse_set(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &SparseSetArgs,
) -> Result<(), CommandError> {
    let stdin_patterns = if args.stdin {
        let mut input = String::new();
        io::stdin().read_to_string(&mut input)?;
        input
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| {
                RepoPathBuf::from_relative_path(line).map_err(|err| {
                    user_error_with_message(format!("Failed to parse sparse pattern: {line}"), err)
                })
            })
            .try_collect::<_, Vec<_>, _>()?
    } else {
        Vec::new()
    };

    let mut workspace_command = command.workspace_helper(ui).await?;
    update_sparse_patterns_with(ui, &mut workspace_command, |_ui, old_patterns| {
        let mut new_patterns = HashSet::new();
        if !args.clear && !args.stdin {
            new_patterns.extend(old_patterns.iter().cloned());
        }
        new_patterns.extend(stdin_patterns);
        for path in &args.remove {
            new_patterns.remove(path);
        }
        for path in &args.add {
            new_patterns.insert(path.to_owned());
        }
        Ok(new_patterns.into_iter().sorted_unstable().collect())
    })
    .await
}
