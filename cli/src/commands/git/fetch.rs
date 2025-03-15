// Copyright 2020-2023 The Jujutsu Authors
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

use std::collections::HashSet;

use clap_complete::ArgValueCandidates;
use itertools::Itertools as _;
use jj_lib::config::ConfigGetResultExt as _;
use jj_lib::git;
use jj_lib::git::GitFetch;
use jj_lib::ref_name::RemoteName;
use jj_lib::repo::Repo as _;
use jj_lib::str_util::StringPattern;

use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::WorkspaceCommandTransaction;
use crate::command_error::config_error;
use crate::command_error::user_error;
use crate::command_error::CommandError;
use crate::commands::git::get_single_remote;
use crate::complete;
use crate::git_util::print_git_import_stats;
use crate::git_util::with_remote_git_callbacks;
use crate::ui::Ui;

/// Fetch from a Git remote
///
/// If a working-copy commit gets abandoned, it will be given a new, empty
/// commit. This is true in general; it is not specific to this command.
#[derive(clap::Args, Clone, Debug)]
pub struct GitFetchArgs {
    /// Fetch only some of the branches
    ///
    /// By default, the specified name matches exactly. Use `glob:` prefix to
    /// expand `*` as a glob, e.g. `--branch 'glob:push-*'`. Other wildcard
    /// characters such as `?` are *not* supported.
    #[arg(
        long, short,
        alias = "bookmark",
        default_value = "glob:*",
        value_parser = StringPattern::parse,
        add = ArgValueCandidates::new(complete::bookmarks),
    )]
    branch: Vec<StringPattern>,
    /// The remote to fetch from (only named remotes are supported, can be
    /// repeated)
    ///
    /// This defaults to the `git.fetch` setting. If that is not configured, and
    /// if there are multiple remotes, the remote named "origin" will be used.
    ///
    /// By default, the specified remote names matches exactly. Use a [string
    /// pattern], e.g. `--remote 'glob:*'`, to select remotes using
    /// patterns.
    ///
    /// [string pattern]:
    ///     https://jj-vcs.github.io/jj/latest/revsets#string-patterns
    #[arg(
        long = "remote",
        value_name = "REMOTE",
        value_parser = StringPattern::parse,
        add = ArgValueCandidates::new(complete::git_remotes),
    )]
    remotes: Vec<StringPattern>,
    /// Fetch from all remotes
    #[arg(long, conflicts_with = "remotes")]
    all_remotes: bool,
}

#[tracing::instrument(skip_all)]
pub fn cmd_git_fetch(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &GitFetchArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let remote_patterns = if args.all_remotes {
        vec![StringPattern::everything()]
    } else if args.remotes.is_empty() {
        get_default_fetch_remotes(ui, &workspace_command)?
    } else {
        args.remotes.clone()
    };

    let all_remotes = git::get_all_remote_names(workspace_command.repo().store())?;

    let mut matching_remotes = HashSet::new();
    for pattern in remote_patterns {
        let remotes = all_remotes
            .iter()
            .filter(|r| pattern.matches(r.as_str()))
            .collect_vec();
        if remotes.is_empty() {
            writeln!(ui.warning_default(), "No git remotes matching '{pattern}'")?;
        } else {
            matching_remotes.extend(remotes);
        }
    }

    if matching_remotes.is_empty() {
        return Err(user_error("No git remotes to push"));
    }

    let remotes = matching_remotes
        .iter()
        .map(|r| r.as_ref())
        .sorted()
        .collect_vec();

    let mut tx = workspace_command.start_transaction();
    do_git_fetch(ui, &mut tx, &remotes, &args.branch)?;
    tx.finish(
        ui,
        format!(
            "fetch from git remote(s) {}",
            remotes.iter().map(|n| n.as_symbol()).join(",")
        ),
    )?;
    Ok(())
}

const DEFAULT_REMOTE: &RemoteName = RemoteName::new("origin");

fn get_default_fetch_remotes(
    ui: &Ui,
    workspace_command: &WorkspaceCommandHelper,
) -> Result<Vec<StringPattern>, CommandError> {
    const KEY: &str = "git.fetch";
    let settings = workspace_command.settings();
    if let Ok(remotes) = settings.get::<Vec<String>>(KEY) {
        remotes
            .into_iter()
            .map(|r| parse_remote_pattern(&r))
            .try_collect()
    } else if let Some(remote) = settings.get_string(KEY).optional()? {
        Ok(vec![parse_remote_pattern(&remote)?])
    } else if let Some(remote) = get_single_remote(workspace_command.repo().store())? {
        // if nothing was explicitly configured, try to guess
        if remote != DEFAULT_REMOTE {
            writeln!(
                ui.hint_default(),
                "Fetching from the only existing remote: {remote}",
                remote = remote.as_symbol()
            )?;
        }
        Ok(vec![StringPattern::exact(remote)])
    } else {
        Ok(vec![StringPattern::exact(DEFAULT_REMOTE)])
    }
}

fn parse_remote_pattern(remote: &str) -> Result<StringPattern, CommandError> {
    StringPattern::parse(remote).map_err(config_error)
}

fn do_git_fetch(
    ui: &mut Ui,
    tx: &mut WorkspaceCommandTransaction,
    remotes: &[&RemoteName],
    branch_names: &[StringPattern],
) -> Result<(), CommandError> {
    let git_settings = tx.settings().git_settings()?;
    let mut git_fetch = GitFetch::new(tx.repo_mut(), &git_settings)?;

    for remote_name in remotes {
        with_remote_git_callbacks(ui, |callbacks| {
            git_fetch.fetch(remote_name, branch_names, callbacks, None)
        })?;
    }
    let import_stats = git_fetch.import_refs()?;
    print_git_import_stats(ui, tx.repo(), &import_stats, true)?;
    warn_if_branches_not_found(ui, tx, branch_names, remotes)
}

fn warn_if_branches_not_found(
    ui: &mut Ui,
    tx: &WorkspaceCommandTransaction,
    branches: &[StringPattern],
    remotes: &[&RemoteName],
) -> Result<(), CommandError> {
    for branch in branches {
        let matches = remotes.iter().any(|&remote| {
            let remote = StringPattern::exact(remote);
            tx.repo()
                .view()
                .remote_bookmarks_matching(branch, &remote)
                .next()
                .is_some()
                || tx
                    .base_repo()
                    .view()
                    .remote_bookmarks_matching(branch, &remote)
                    .next()
                    .is_some()
        });
        if !matches {
            writeln!(
                ui.warning_default(),
                "No branch matching `{branch}` found on any specified/configured remote",
            )?;
        }
    }

    Ok(())
}
