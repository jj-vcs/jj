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

use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use jj_lib::repo::Repo;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::gerrit_util::calculate_gerrit_remote_branch;
use crate::gerrit_util::generate_gerrit_change_id;
use crate::gerrit_util::gerrit_change_id;
use crate::gerrit_util::get_gerrit_repo;
use crate::gerrit_util::get_gerrit_review_url;
use crate::ui::Ui;

#[derive(clap::Args, Clone, Debug)]
pub struct QueryArgs {
    /// The revision to fetch comments for (must evaluate to a single revision).
    #[arg(long, short = 'r', default_value = "@")]
    pub revision: RevisionArg,
}

// Repo names frequently have '/' in them, and need to be escaped.
fn url_encode(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*b as char);
            }
            _ => write!(&mut encoded, "%{b:02X}").unwrap(),
        }
    }
    encoded
}

impl QueryArgs {
    async fn rev_id(&self, ui: &mut Ui, command: &CommandHelper) -> Result<String, CommandError> {
        let workspace_command = command.workspace_helper(ui)?;
        let commit = workspace_command
            .resolve_single_rev(ui, &self.revision)
            .await?;
        let change_id =
            gerrit_change_id(&commit)?.unwrap_or_else(|| generate_gerrit_change_id(&commit));

        let store = workspace_command.repo().store();
        let repo = get_gerrit_repo(store, command.settings())?;
        let branch =
            calculate_gerrit_remote_branch(command.settings(), None)?;

        Ok(format!("{}~{}~{}", repo, &branch, &change_id))
    }

    /// Queries a Gerrit REST API endpoint via curl. I tried several rust
    /// libraries instead of curl but all failed even with native TLS enabled.
    /// These rust libraries appear to not play nice with corporate networks,
    /// so curl seems like the most reliable method.
    pub async fn query<T: serde::de::DeserializeOwned>(
        &self,
        ui: &mut Ui,
        command: &CommandHelper,
        prefix: &str,
        suffix: &str,
    ) -> Result<T, CommandError> {
        let url = format!(
            "{}/a/{}/{}/{}",
            get_gerrit_review_url(command.settings())?,
            prefix,
            url_encode(&self.rev_id(ui, command).await?),
            suffix,
        );

        // Curl exists on windows too since windows 10.
        let mut cmd = Command::new("curl");
        cmd.arg("-s");

        let cookie_file_path =
            jj_lib::git_backend::GitBackend::cookie_file(Path::new(".")).unwrap_or_default();
        if cookie_file_path.exists() {
            cmd.arg("-b").arg(cookie_file_path);
        }

        cmd.arg(url);

        tracing::debug!("Running gerrit query command: {cmd:?}");
        let output = cmd
            .output()
            .map_err(|e| user_error_with_message("Failed to execute curl", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(user_error(format!("curl request failed: {stderr}")));
        }

        let body = String::from_utf8_lossy(&output.stdout);

        // Gerrit responses start with the magic prefix )]}'
        // Note: for now, we are printing the HTTP response of the error message.
        // We should probably improve on this by parsing the body out of it in the
        // future.
        let json_str = body
            .strip_prefix(")]}'\n")
            .ok_or_else(|| user_error(format!("Gerrit query failed: {body}")))?;

        let data: T = serde_json::from_str(json_str)
            .map_err(|e| user_error_with_message("Failed to deserialize JSON", e))?;

        Ok(data)
    }
}
