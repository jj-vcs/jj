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

use camino::Utf8PathBuf;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Install Jujutsu's manpages to the provided path
#[derive(clap::Args, Clone, Debug)]
pub struct UtilInstallManPagesArgs {
    /// The path where manpages will installed. An example path might be
    /// `/usr/share/man`. The provided path will be appended with `man1`,
    /// etc., as appropriate
    path: Utf8PathBuf,
}

pub fn cmd_util_install_man_pages(
    _ui: &mut Ui,
    command: &CommandHelper,
    args: &UtilInstallManPagesArgs,
) -> Result<(), CommandError> {
    let man1_dir = args.path.join("man1");
    std::fs::create_dir_all(&man1_dir)?;
    let app = command.app().clone();
    clap_mangen::generate_to(app, man1_dir)?;
    Ok(())
}
