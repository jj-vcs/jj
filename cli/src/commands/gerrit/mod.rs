// Copyright 2024 The Jujutsu Authors
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

use std::fmt::Debug;

use clap::Subcommand;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::commands::gerrit;
use crate::ui::Ui;

/// Interact with Gerrit Code Review.
#[derive(Subcommand, Clone, Debug)]
pub enum GerritCommand {
    Upload(gerrit::upload::UploadArgs),
}

pub fn cmd_gerrit(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &GerritCommand,
) -> Result<(), CommandError> {
    match subcommand {
        GerritCommand::Upload(review) => gerrit::upload::cmd_gerrit_upload(ui, command, review),
    }
}

mod upload;
