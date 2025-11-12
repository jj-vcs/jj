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

use clap::Command;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

// Using an explicit `doc` attribute prevents rustfmt from mangling the list
// formatting without disabling rustfmt for the entire struct.
#[doc = r#"Print a command-line-completion script

Apply it by running one of these:

- Bash: `source <(jj util completion bash)`
- Fish: `jj util completion fish | source`
- Nushell:
     ```nu
     jj util completion nushell | save -f "completions-jj.nu"
     use "completions-jj.nu" *  # Or `source "completions-jj.nu"`
     ```
- Zsh:
     ```shell
     autoload -U compinit
     compinit
     source <(jj util completion zsh)
     ```

See the docs on [command-line completion] for more details.

[command-line completion]:
    https://docs.jj-vcs.dev/latest/install-and-setup/#command-line-completion
"#]
#[derive(clap::Args, Clone, Debug)]
#[command(verbatim_doc_comment)]
pub struct UtilCompletionArgs {
    shell: ShellCompletion,
}

pub fn cmd_util_completion(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &UtilCompletionArgs,
) -> Result<(), CommandError> {
    let mut app = command.app().clone();
    let buf = args.shell.generate(&mut app);
    ui.stdout().write_all(&buf)?;
    Ok(())
}

/// Available shell completions
#[derive(clap::ValueEnum, Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ShellCompletion {
    Bash,
    Elvish,
    Fish,
    Nushell,
    PowerShell,
    Zsh,
}

impl ShellCompletion {
    fn generate(&self, cmd: &mut Command) -> Vec<u8> {
        use clap_complete::Shell;
        use clap_complete::generate;
        use clap_complete_nushell::Nushell;

        let mut buf = Vec::new();

        let bin_name = "jj";

        match self {
            Self::Bash => generate(Shell::Bash, cmd, bin_name, &mut buf),
            Self::Elvish => generate(Shell::Elvish, cmd, bin_name, &mut buf),
            Self::Fish => generate(Shell::Fish, cmd, bin_name, &mut buf),
            Self::Nushell => generate(Nushell, cmd, bin_name, &mut buf),
            Self::PowerShell => generate(Shell::PowerShell, cmd, bin_name, &mut buf),
            Self::Zsh => generate(Shell::Zsh, cmd, bin_name, &mut buf),
        }

        buf
    }
}
