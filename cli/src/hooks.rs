// Copyright 2025 The Jujutsu Authors
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

use std::collections::HashMap;

use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::fileset::FilesetExpression;
use jj_lib::settings::UserSettings;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::commands::fix::fix_revisions;
use crate::ui::Ui;

/// A hook in the `hooks.pre-upload` config table.
enum PreUploadToolConfig {
    FixTool,
}

impl PreUploadToolConfig {
    fn name(&self) -> &str {
        match self {
            Self::FixTool => "fix",
        }
    }
}

/// Parses the `hooks.pre-upload` config table.
fn pre_upload_tools(settings: &UserSettings) -> Result<Vec<PreUploadToolConfig>, CommandError> {
    fn default_true() -> bool {
        true
    }

    // Simplifies deserialization of the config values while building a ToolConfig.
    #[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize)]
    #[serde(rename_all = "kebab-case")]
    struct RawPreUploadToolConfig {
        #[serde(default = "default_true")]
        enabled: bool,

        order: i32,
    }

    let mut tools: Vec<_> = vec![];
    for name in settings
        .table_keys("hooks.pre-upload")
        // Sort keys so errors are deterministic.
        .sorted()
    {
        let tool: RawPreUploadToolConfig = settings.get(["hooks", "pre-upload", name])?;
        if tool.enabled {
            tools.push((
                (tool.order, name),
                if name == "fix" {
                    PreUploadToolConfig::FixTool
                } else {
                    return Err(user_error(
                        "Generic pre-upload hooks are currently unsupported. Only fix is \
                         supported for now",
                    ));
                },
            ));
        }
    }
    Ok(tools
        .into_iter()
        .sorted_by(|lhs, rhs| Ord::cmp(&lhs.0, &rhs.0))
        .map(|(_, value)| value)
        .collect())
}

/// Triggered every time a user runs something that semantically approximates
/// an "upload".
///
/// Currently, this triggers on `jj gerrit upload`. Other forges which
/// implement custom upload scripts should also call this.
///
/// This should ideally work for `jj git push` too, but doing so has
/// consequences. `git push` can be used to upload to code review, but it can
/// do many other things as well. We need to ensure the UX works well before
/// adding it to `git push`.
///
/// This function may create transactions that rewrite commits, so is not
/// allowed to be called while a transaction is ongoing.
/// It returns a mapping of rewrites, and users are expected to update any
/// references to point at the new revision.
pub(crate) fn run_pre_upload_hooks(
    ui: &mut Ui,
    command: &CommandHelper,
    workspace_command: &mut crate::cli_util::WorkspaceCommandHelper,
    commit_ids: &[CommitId],
) -> Result<HashMap<CommitId, CommitId>, CommandError> {
    let mut rewrites: HashMap<CommitId, CommitId> = Default::default();
    let mut rewrites_reversed: HashMap<CommitId, CommitId> = Default::default();
    let mut current_commits: Vec<CommitId> = commit_ids.to_vec();
    for tool in pre_upload_tools(command.settings())? {
        // Tools must perform 1-1 rewrites.
        // They cannot split commits (1-2), squash commits (2-1), or abandon commits
        // (1-0). We deal with any instance of the above.
        let next_rewrites = match tool {
            PreUploadToolConfig::FixTool => fix_revisions(
                ui,
                workspace_command,
                commit_ids,
                &FilesetExpression::all().to_matcher(),
                false,
            )?,
        };
        current_commits = current_commits
            .into_iter()
            .map(|c| next_rewrites.get(&c).cloned().unwrap_or(c))
            .collect();
        for (from, to) in next_rewrites {
            let from = rewrites_reversed.get(&from).cloned().unwrap_or(from);
            rewrites.insert(from, to);
        }
        rewrites_reversed = Default::default();
        for (from, to) in &rewrites {
            if rewrites_reversed.insert(from.clone(), to.clone()).is_some() {
                return Err(user_error(format!(
                    "The pre-upload tool {} combined two commits into one",
                    tool.name()
                )));
            };
        }
    }
    Ok(rewrites)
}

pub(crate) fn apply_rewrites(
    rewrites: &HashMap<CommitId, CommitId>,
    commits: Vec<CommitId>,
) -> Vec<CommitId> {
    commits
        .into_iter()
        .map(|c| rewrites.get(&c).cloned().unwrap_or(c))
        .collect()
}
