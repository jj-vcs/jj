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

use std::io::Write as _;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Print the JSON schema for the jj TOML config format.
#[derive(clap::Args, Clone, Debug)]
pub struct UtilConfigSchemaArgs {}

pub fn cmd_util_config_schema(
    ui: &mut Ui,
    _command: &CommandHelper,
    _args: &UtilConfigSchemaArgs,
) -> Result<(), CommandError> {
    // TODO(#879): Consider generating entire schema dynamically vs. static file.
    let buf = include_bytes!("../../config-schema.json");
    ui.stdout().write_all(buf)?;
    Ok(())
}
