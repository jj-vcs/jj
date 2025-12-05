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
use jj_lib::str_util::StringExpression;

use crate::cli_util::CommandHelper;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::WorkspaceCommandTransaction;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::commands::git::get_single_remote;
use crate::complete;
use crate::git_util::load_git_import_options;
use crate::git_util::print_git_import_stats;
use crate::git_util::with_remote_git_callbacks;
use crate::revset_util::parse_union_name_patterns;
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
        add = ArgValueCandidates::new(complete::bookmarks),
    )]
    branch: Option<Vec<String>>,
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
        add = ArgValueCandidates::new(complete::git_remotes),
    )]
    remotes: Option<Vec<String>>,
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
    let remote_expr = if args.all_remotes {
        StringExpression::all()
    } else if let Some(remotes) = &args.remotes {
        parse_union_name_patterns(ui, remotes)?
    } else {
        get_default_fetch_remotes(ui, &workspace_command)?
    };
    let remote_matcher = remote_expr.to_matcher();

    let all_remotes = git::get_all_remote_names(workspace_command.repo().store())?;
    let matching_remotes: Vec<&RemoteName> = all_remotes
        .iter()
        .filter(|r| remote_matcher.is_match(r.as_str()))
        .map(AsRef::as_ref)
        .collect();
    let mut unmatched_remotes = remote_expr
        .exact_strings()
        .map(RemoteName::new)
        // do linear search. all_remotes should be small.
        .filter(|&name| all_remotes.iter().all(|r| r != name))
        .peekable();
    if unmatched_remotes.peek().is_some() {
        writeln!(
            ui.warning_default(),
            "No matching remotes for names: {}",
            unmatched_remotes.map(|name| name.as_symbol()).join(", ")
        )?;
    }
    if matching_remotes.is_empty() {
        return Err(user_error("No git remotes to fetch from"));
    }

    let mut tx = workspace_command.start_transaction();

    let common_bookmark_expr = match &args.branch {
        Some(texts) => Some(parse_union_name_patterns(ui, texts)?),
        None => None,
    };
    let mut expansions = Vec::with_capacity(matching_remotes.len());
    if args.tracked {
        for remote in &matching_remotes {
            let bookmark_expr = StringExpression::union_all(
                tx.repo()
                    .view()
                    .local_remote_bookmarks(remote)
                    .filter(|(_, targets)| targets.remote_ref.is_tracked())
                    .map(|(name, _)| StringExpression::exact(name))
                    .collect(),
            );
            expansions.push((remote, expand_fetch_refspecs(remote, bookmark_expr)?));
        }
    } else if let Some(bookmark_expr) = &common_bookmark_expr {
        for remote in &matching_remotes {
            let expanded = expand_fetch_refspecs(remote, bookmark_expr.clone())?;
            expansions.push((remote, expanded));
        }
    } else {
        let git_repo = get_git_backend(tx.repo_mut().store())?.git_repo();
        for remote in &matching_remotes {
            let (ignored, expanded) = expand_default_fetch_refspecs(remote, &git_repo)?;
            warn_ignored_refspecs(ui, remote, ignored)?;
            expansions.push((remote, expanded));
        }
    };

    let git_settings = GitSettings::from_settings(tx.settings())?;
    let remote_settings = tx.settings().remote_settings()?;
    let import_options = load_git_import_options(ui, &git_settings, &remote_settings)?;
    let mut git_fetch = GitFetch::new(tx.repo_mut(), &git_settings, &import_options)?;

    for (remote, expanded) in expansions {
        with_remote_git_callbacks(ui, |callbacks| {
            git_fetch.fetch(remote, expanded, callbacks, None, None)
        })?;
    }

    let import_stats = git_fetch.import_refs()?;
    print_git_import_stats(ui, tx.repo(), &import_stats, true)?;
    if let Some(bookmark_expr) = &common_bookmark_expr {
        warn_if_branches_not_found(ui, &tx, bookmark_expr, &matching_remotes)?;
    }
    tx.finish(
        ui,
        format!(
            "fetch from git remote(s) {}",
            matching_remotes.iter().map(|n| n.as_symbol()).join(",")
        ),
    )?;
    Ok(())
}

const DEFAULT_REMOTE: &RemoteName = RemoteName::new("origin");

fn get_default_fetch_remotes(
    ui: &Ui,
    workspace_command: &WorkspaceCommandHelper,
) -> Result<StringExpression, CommandError> {
    const KEY: &str = "git.fetch";
    let settings = workspace_command.settings();
    if let Ok(remotes) = settings.get::<Vec<String>>(KEY) {
        parse_union_name_patterns(ui, &remotes)
    } else if let Some(remote) = settings.get_string(KEY).optional()? {
        parse_union_name_patterns(ui, [&remote])
    } else if let Some(remote) = get_single_remote(workspace_command.repo().store())? {
        // if nothing was explicitly configured, try to guess
        if remote != DEFAULT_REMOTE {
            writeln!(
                ui.hint_default(),
                "Fetching from the only existing remote: {remote}",
                remote = remote.as_symbol()
            )?;
        }
        Ok(StringExpression::exact(remote))
    } else {
        Ok(StringExpression::exact(DEFAULT_REMOTE))
    }
}

fn warn_if_branches_not_found(
    ui: &mut Ui,
    tx: &WorkspaceCommandTransaction,
    bookmark_expr: &StringExpression,
    remotes: &[&RemoteName],
) -> io::Result<()> {
    let bookmark_matcher = bookmark_expr.to_matcher();
    let mut missing_branches = bookmark_expr
        .exact_strings()
        .filter(|name| bookmark_matcher.is_match(name)) // exclude negative patterns
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
