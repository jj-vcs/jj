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
use std::collections::HashSet;

use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::fileset::FilesetExpression;
use jj_lib::settings::UserSettings;

use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::commands::fix::fix_revisions;
use crate::ui::Ui;

/// A hook in the `hooks.pre-upload` config table.
enum PreUploadToolConfig {
    FixTool,
}

/// Parses the `hooks.pre-upload` config table.
fn pre_upload_tools(settings: &UserSettings) -> Result<Vec<PreUploadToolConfig>, CommandError> {
    // Simplifies deserialization of the config values while building a ToolConfig.
    #[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize)]
    #[serde(rename_all = "kebab-case")]
    struct RawPreUploadToolConfig {
        enabled: bool,
    }

    let mut tools: Vec<_> = vec![];
    for name in settings
        .table_keys("hooks.pre-upload")
        // Sort keys so errors are deterministic.
        .sorted()
    {
        let tool: RawPreUploadToolConfig = settings.get(["hooks", "pre-upload", name])?;
        if tool.enabled {
            tools.push(if name == "fix" {
                PreUploadToolConfig::FixTool
            } else {
                return Err(user_error(
                    "Generic pre-upload hooks are currently unsupported. Only fix is supported \
                     for now",
                ));
            });
        }
    }
    Ok(tools)
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
    workspace_command: &mut WorkspaceCommandHelper,
    commit_ids: &[CommitId],
) -> Result<HashMap<CommitId, Vec<CommitId>>, CommandError> {
    // Rewrites are a many-to-many relationship.
    let mut rewrites = HashMap::<CommitId, Vec<CommitId>>::new();
    let mut current_commits = commit_ids.to_vec();
    for tool in pre_upload_tools(command.settings())? {
        let next_rewrites: HashMap<CommitId, Vec<CommitId>> = match tool {
            PreUploadToolConfig::FixTool => {
                let mut tx = workspace_command.start_transaction();
                let summary = fix_revisions(
                    ui,
                    &mut tx,
                    &current_commits,
                    &FilesetExpression::all().to_matcher(),
                    false,
                )?;
                tx.finish(ui, format!("fixed {} commits", summary.num_fixed_commits))?;
                summary
                    .rewrites
                    .into_iter()
                    .map(|(k, v)| (k, vec![v]))
                    .collect()
            }
        };

        current_commits = apply_rewrites(&next_rewrites, current_commits);

        // Apply transitive rewrites.
        for v in rewrites.values_mut() {
            *v = apply_rewrites(&next_rewrites, v.clone());
        }

        for (from, to) in next_rewrites {
            rewrites.insert(from, to);
        }
    }
    Ok(rewrites)
}

pub(crate) fn apply_rewrites(
    rewrites: &HashMap<CommitId, Vec<CommitId>>,
    commits: Vec<CommitId>,
) -> Vec<CommitId> {
    let rewritten: Vec<_> = commits
        .into_iter()
        .flat_map(|c| rewrites.get(&c).cloned().unwrap_or(vec![c]))
        .collect();
    let mut filtered = vec![];
    let mut seen = HashSet::new();
    for commit in &rewritten {
        if seen.insert(commit) {
            filtered.push(commit.clone());
        }
    }
    filtered
}
