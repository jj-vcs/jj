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

use std::path::PathBuf;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Generate and write manpages.
#[derive(clap::Args, Clone, Debug)]
pub struct UtilMangenArgs {
    /// The destination where manpages will be written to.
    #[arg(default_value = "man")]
    destination: PathBuf,
}

pub fn cmd_util_mangen(
    _ui: &mut Ui,
    command: &CommandHelper,
    args: &UtilMangenArgs,
) -> Result<(), CommandError> {
    let man1_dir = args.destination.join("man1");
    std::fs::create_dir_all(&man1_dir)?;
    let app = command.app().clone();
    clap_mangen::generate_to(app, man1_dir)?;
    Ok(())
}
