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
use std::io;

use clap_complete::ArgValueCandidates;
use itertools::Itertools as _;
use jj_lib::config::ConfigGetResultExt as _;
use jj_lib::git;
use jj_lib::git::GitFetch;
use jj_lib::git::GitSettings;
use jj_lib::git::IgnoredRefspec;
use jj_lib::git::IgnoredRefspecs;
use jj_lib::git::expand_default_fetch_refspecs;
use jj_lib::git::expand_fetch_refspecs;
use jj_lib::git::get_git_backend;
use jj_lib::ref_name::RefName;
use jj_lib::ref_name::RemoteName;
use jj_lib::repo::Repo as _;
use jj_lib::str_util::StringPattern;

use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::WorkspaceCommandTransaction;
use crate::command_error::CommandError;
use crate::command_error::config_error;
use crate::command_error::user_error;
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
    /// characters such as `?` are *not* supported. Can be repeated to specify
    /// multiple branches.
    #[arg(
        long, short,
        alias = "bookmark",
        value_parser = StringPattern::parse,
        add = ArgValueCandidates::new(complete::bookmarks),
    )]
    branch: Option<Vec<StringPattern>>,
    /// Fetch only tracked bookmarks
    ///
    /// This fetches only bookmarks that are already tracked from the specified
    /// remote(s).
    #[arg(long, conflicts_with = "branch")]
    tracked: bool,
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
    ///     https://docs.jj-vcs.dev/latest/revsets#string-patterns
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
        vec![StringPattern::all()]
    } else if args.remotes.is_empty() {
        get_default_fetch_remotes(ui, &workspace_command)?
    } else {
        args.remotes.clone()
    };

    let all_remotes = git::get_all_remote_names(workspace_command.repo().store())?;

    let mut matching_remotes = HashSet::new();
    let mut unmatched_remotes = Vec::new();
    for pattern in &remote_patterns {
        let remotes = all_remotes
            .iter()
            .filter(|r| pattern.is_match(r.as_str()))
            .collect_vec();
        if remotes.is_empty() {
            unmatched_remotes.extend(pattern.as_exact().map(RemoteName::new));
        } else {
            matching_remotes.extend(remotes);
        }
    }

    if !unmatched_remotes.is_empty() {
        writeln!(
            ui.warning_default(),
            "No matching remotes for names: {}",
            unmatched_remotes
                .iter()
                .map(|name| name.as_symbol())
                .join(", ")
        )?;
    }
    if matching_remotes.is_empty() {
        return Err(user_error("No git remotes to fetch from"));
    }

    let remotes = matching_remotes
        .iter()
        .map(|r| r.as_ref())
        .sorted()
        .collect_vec();

    let mut tx = workspace_command.start_transaction();

    let mut expansions = Vec::with_capacity(remotes.len());
    if args.tracked {
        for remote in &remotes {
            let tracked_branches = tx
                .repo()
                .view()
                .local_remote_bookmarks(remote)
                .filter(|(_, targets)| targets.remote_ref.is_tracked())
                .map(|(name, _)| StringPattern::exact(name))
                .collect_vec();
            expansions.push((remote, expand_fetch_refspecs(remote, tracked_branches)?));
        }
    } else if let Some(branches) = &args.branch {
        for remote in &remotes {
            let expanded = expand_fetch_refspecs(remote, branches.clone())?;
            expansions.push((remote, expanded));
        }
    } else {
        let git_repo = get_git_backend(tx.repo_mut().store())?.git_repo();
        for remote in &remotes {
            let (ignored, expanded) = expand_default_fetch_refspecs(remote, &git_repo)?;
            warn_ignored_refspecs(ui, remote, ignored)?;
            expansions.push((remote, expanded));
        }
    };

    let git_settings = GitSettings::from_settings(tx.settings())?;
    let mut git_fetch = GitFetch::new(tx.repo_mut(), &git_settings)?;

    for (remote, expanded) in expansions {
        with_remote_git_callbacks(ui, |callbacks| {
            git_fetch.fetch(remote, expanded, callbacks, None, None)
        })?;
    }

    let import_stats = git_fetch.import_refs()?;
    print_git_import_stats(ui, tx.repo(), &import_stats, true)?;
    if let Some(branches) = &args.branch {
        warn_if_branches_not_found(ui, &tx, branches, &remotes)?;
    }
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

fn warn_if_branches_not_found(
    ui: &mut Ui,
    tx: &WorkspaceCommandTransaction,
    branches: &[StringPattern],
    remotes: &[&RemoteName],
) -> io::Result<()> {
    let mut missing_branches = branches
        .iter()
        .filter_map(StringPattern::as_exact)
        .map(RefName::new)
        .filter(|name| {
            remotes.iter().all(|&remote| {
                let symbol = name.to_remote_symbol(remote);
                let view = tx.repo().view();
                let base_view = tx.base_repo().view();
                view.get_remote_bookmark(symbol).is_absent()
                    && base_view.get_remote_bookmark(symbol).is_absent()
            })
        })
        .peekable();
    if missing_branches.peek().is_none() {
        return Ok(());
    }
    writeln!(
        ui.warning_default(),
        "No matching branches found on any specified/configured remote: {}",
        missing_branches.map(|name| name.as_symbol()).join(", ")
    )
}

fn warn_ignored_refspecs(
    ui: &Ui,
    remote_name: &RemoteName,
    IgnoredRefspecs(ignored_refspecs): IgnoredRefspecs,
) -> Result<(), CommandError> {
    let remote_name = remote_name.as_symbol();
    for IgnoredRefspec { refspec, reason } in ignored_refspecs {
        writeln!(
            ui.warning_default(),
            "Ignored refspec `{refspec}` from `{remote_name}`: {reason}",
        )?;
    }

    Ok(())
}
