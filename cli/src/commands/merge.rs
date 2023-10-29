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

use tracing::instrument;

use super::new;
use crate::cli_util::{CommandError, CommandHelper};
use crate::ui::Ui;

#[instrument(skip_all)]
pub(crate) fn cmd_merge(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &new::NewArgs,
) -> Result<(), CommandError> {
    if args.revisions.len() < 2 {
        return Err(CommandError::CliError(String::from(
            "Merge requires at least two revisions",
        )));
    }
    new::cmd_new(ui, command, args)
}
