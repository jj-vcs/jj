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

use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// Display version information
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct VersionArgs {
    /// Display only the version number and nothing else.
    #[arg(long)]
    pub(crate) numeric_only: bool,

    /// Display build information.
    #[arg(long, conflicts_with = "numeric_only")]
    pub(crate) details: bool,
}

#[instrument(skip_all)]
pub(crate) fn cmd_version(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &VersionArgs,
) -> Result<(), CommandError> {
    let base_version = command.app().get_version().unwrap();
    let (version, git_commit) = if let Some(git_commit) = option_env!("JJ_GIT_COMMIT") {
        // in a release build, don't include the commit hash in the
        // `--numeric-version` or normal output. GH-3629
        if option_env!("JJ_RELEASE_BUILD").is_some() {
            (String::from(base_version), git_commit)
        } else {
            let short_commit = &git_commit[..12];
            (format!("{base_version}-{short_commit}"), git_commit)
        }
    } else {
        (String::from(base_version), "unknown")
    };

    let build_suffix = option_env!("JJ_BUILD_SUFFIX")
        .map(|s| format!("-{s}"))
        .unwrap_or_default();
    let version = format!("{version}{build_suffix}");

    if args.numeric_only {
        writeln!(ui.stdout(), "{version}")?;
        return Ok(());
    }

    writeln!(ui.stdout(), "Jujutsu version control system; jj {version}")?;

    if !args.details {
        writeln!(ui.stdout(), "For more details: run `jj version --details`",)?;
        return Ok(());
    }

    write!(
        ui.stdout(),
        r#"Copyright (C) 2019-2025 The Jujutsu Authors

License: Apache License, Version 2.0
Homepage: <https://jj-vcs.github.io/jj>
Report bugs: <https://github.com/jj-vcs/jj/issues>
"#,
    )?;

    writeln!(ui.stdout())?;
    writeln!(ui.stdout(), "Target: {}", env!("JJ_CARGO_TARGET"))?;
    writeln!(ui.stdout(), "Commit: {git_commit}{build_suffix}")?;
    writeln!(
        ui.stdout(),
        "Release: {}",
        option_env!("JJ_RELEASE_BUILD").is_some()
    )?;

    Ok(())
}
