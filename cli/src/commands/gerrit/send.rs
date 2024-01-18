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

use std::fmt::Debug;
use std::io::Write as _;
use std::rc::Rc;
use std::sync::Arc;

use bstr::BStr;
use hex::ToHex as _;
use indexmap::IndexMap;
use itertools::Itertools as _;
use jj_lib::commit::Commit;
use jj_lib::commit::CommitIteratorExt as _;
use jj_lib::content_hash::blake2b_hash;
use jj_lib::git::GitRefUpdate;
use jj_lib::git::{self};
use jj_lib::repo::Repo as _;
use jj_lib::revset::RevsetExpression;
use jj_lib::settings::UserSettings;
use jj_lib::store::Store;
use jj_lib::trailer::Trailer;
use jj_lib::trailer::parse_description_trailers;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::short_commit_hash;
use crate::command_error::CommandError;
use crate::command_error::internal_error;
use crate::command_error::user_error;
use crate::command_error::user_error_with_hint;
use crate::command_error::user_error_with_message;
use crate::git_util::with_remote_git_callbacks;
use crate::ui::Ui;

#[derive(clap::Args, Clone, Debug)]
pub struct SendArgs {
    /// The revset, selecting which commits are sent in to Gerrit. This can be
    /// any arbitrary set of commits; they will be modified to include a
    /// `Change-Id` footer if one does not already exist, and then sent off to
    /// Gerrit for review.
    #[arg(long, short = 'r')]
    revisions: Vec<RevisionArg>,

    /// The location where your changes are intended to land. This should be
    /// an upstream branch.
    #[arg(long = "remote-branch", short = 'b')]
    remote_branch: Option<String>,

    /// The Gerrit remote to push to. Can be configured with the `gerrit.remote`
    /// repository option as well. This is typically a full SSH URL for your
    /// Gerrit instance.
    #[arg(long)]
    remote: Option<String>,

    /// If true, do not actually add `Change-Id`s to commits, and do not push
    /// the changes to Gerrit.
    #[arg(long = "dry-run", short = 'n')]
    dry_run: bool,
}

/// calculate push remote. The logic is:
/// 1. If the user specifies `--remote`, use that
/// 2. If the user has 'gerrit.remote' configured, use that
/// 3. If there is a default push remote, use that
/// 4. If the user has a remote named 'gerrit', use that
/// 5. otherwise, bail out
fn calculate_push_remote(
    store: &Arc<Store>,
    config: &UserSettings,
    remote: Option<String>,
) -> Result<String, CommandError> {
    let git_repo = git::get_git_repo(store)?; // will fail if not a git repo
    let remotes = git_repo.remote_names();

    // case 1
    if let Some(remote) = remote {
        if remotes.contains(BStr::new(&remote)) {
            return Ok(remote);
        }
        return Err(user_error(format!(
            "The remote '{remote}' (specified via `--remote`) does not exist",
        )));
    }

    // case 2
    if let Ok(remote) = config.get_string("gerrit.default-remote") {
        if remotes.contains(BStr::new(&remote)) {
            return Ok(remote);
        }
        return Err(user_error(format!(
            "The remote '{remote}' (configured via `gerrit.default-remote`) does not exist",
        )));
    }

    // case 3
    if let Some(remote) = git_repo.remote_default_name(gix::remote::Direction::Push) {
        return Ok(remote.to_string());
    }

    // case 4
    if remotes.iter().any(|r| **r == "gerrit") {
        return Ok("gerrit".to_owned());
    }

    // case 5
    Err(user_error(
        "No remote specified, and no 'gerrit' remote was found",
    ))
}

/// Determine what Gerrit ref and remote to use. The logic is:
///
/// 1. If the user specifies `--remote-branch branch`, use that
/// 2. If the user has 'gerrit.default-remote-branch' configured, use that
/// 3. Otherwise, bail out
fn calculate_push_ref(
    config: &UserSettings,
    remote_branch: Option<String>,
) -> Result<String, CommandError> {
    // case 1
    if let Some(remote_branch) = remote_branch {
        return Ok(remote_branch);
    }

    // case 2
    if let Ok(branch) = config.get_string("gerrit.default-remote-branch") {
        return Ok(branch);
    }

    // case 3
    Err(user_error(
        "No target branch specified via --remote-branch, and no 'gerrit.default-remote-branch' \
         was found",
    ))
}

pub fn cmd_send(ui: &mut Ui, command: &CommandHelper, send: &SendArgs) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    let revisions: Vec<_> = workspace_command
        .parse_union_revsets(ui, &send.revisions)?
        .evaluate_to_commits()?
        .try_collect()?;
    if revisions.is_empty() {
        writeln!(ui.status(), "No revisions to send.")?;
        return Ok(());
    }

    if revisions
        .iter()
        .any(|commit| commit.id() == workspace_command.repo().store().root_commit_id())
    {
        return Err(user_error("Cannot send the virtual 'root()' commit"));
    }

    workspace_command.check_rewritable(revisions.iter().ids())?;

    // If you have the changes main -> A -> B, and then run `jj gerrit send B`,
    // then that uploads both A and B. Thus, we need to ensure that A also
    // has a Change-ID.
    // We make an assumption here that all immutable commits already have a
    // Change-ID.
    let to_send: Vec<Commit> = workspace_command
        .attach_revset_evaluator(
            // I'm unsure, but this *might* have significant performance
            // implications. If so, we can change it to a maximum depth.
            Rc::new(RevsetExpression::Difference(
                // Unfortunately, DagRange{root: immutable_heads, heads: commits}
                // doesn't work if you're, for example, working on top of an
                // immutable commit that isn't in immutable_heads().
                Rc::new(RevsetExpression::Ancestors {
                    heads: RevsetExpression::commits(
                        revisions.iter().ids().cloned().collect::<Vec<_>>(),
                    ),
                    generation: jj_lib::revset::GENERATION_RANGE_FULL,
                    parents_range: jj_lib::revset::PARENTS_RANGE_FULL,
                }),
                workspace_command.env().immutable_expression().clone(),
            )),
        )
        .evaluate_to_commits()?
        .try_collect()?;

    let mut tx = workspace_command.start_transaction();
    let base_repo = tx.base_repo().clone();
    let store = base_repo.store();

    let git_settings = command.settings().git_settings()?;
    let remote = calculate_push_remote(store, command.settings(), send.remote.clone())?;
    let remote_branch = calculate_push_ref(command.settings(), send.remote_branch.clone())?;

    // immediately error and reject any discardable commits, i.e. the
    // the empty wcc
    for commit in &to_send {
        if commit.is_discardable(tx.repo_mut())? {
            return Err(user_error_with_hint(
                format!(
                    "Refusing to send commit {} because it is an empty commit with no description",
                    short_commit_hash(commit.id())
                ),
                "Perhaps you squashed then ran send? Maybe you meant to send the parent commit \
                 instead (eg. @-)",
            ));
        }
    }

    let mut old_to_new: IndexMap<Commit, Commit> = IndexMap::new();
    for commit_id in to_send.iter().map(|c| c.id()).rev() {
        let original_commit = store.get_commit(commit_id).unwrap();
        let description = original_commit.description().to_owned();
        let trailers = parse_description_trailers(&description);

        if !trailers.is_empty() {
            let change_id_trailers: Vec<&Trailer> = trailers
                .iter()
                .filter(|trailer| trailer.key == "Change-Id")
                .collect();
            // first, figure out if there are multiple Change-Id fields; if so, then we
            // error and continue
            if change_id_trailers.len() > 1 {
                writeln!(
                    ui.warning_default(),
                    "warning: multiple Change-Id footers in commit {}",
                    short_commit_hash(original_commit.id()),
                )?;
                continue;
            }

            if let Some(trailer) = change_id_trailers.first() {
                // map the old commit to itself
                old_to_new.insert(original_commit.clone(), original_commit.clone());

                // check the change-id format is correct in any case
                if trailer.value.len() != 41 || !trailer.value.starts_with('I') {
                    // Intentionally leave the invalid change IDs as-is.
                    writeln!(
                        ui.warning_default(),
                        "warning: invalid Change-Id footer in commit {}",
                        short_commit_hash(original_commit.id()),
                    )?;
                } else {
                    writeln!(
                        ui.status(),
                        "Skipped adding Change-Id (it already exists) for {}",
                        short_commit_hash(original_commit.id())
                    )?;
                }

                continue; // fallthrough
            }
        }

        // NOTE: Gerrit's change ID is not compatible with the alphabet used by
        // jj, and the needed length of the change-id is different as well.
        //
        // for us, we convert to gerrit's format: the character 'I', followed by
        // 40 characters of the blake2 hash of a random binary blob. we use the hash
        // so that any instance of `ContentHash` can be used to generate a unique
        // id, if we ever need it.
        let mut rand_id: [u8; 32] = [0; 32];
        rand::Rng::fill(&mut rand::rng(), &mut rand_id);

        let hashed_id: String = blake2b_hash(&rand_id).encode_hex();
        let gerrit_change_id = format!("I{}", hashed_id.chars().take(40).collect::<String>());

        let new_description = format!(
            "{}{}Change-Id: {}\n",
            description.trim(),
            if trailers.is_empty() { "\n\n" } else { "\n" },
            gerrit_change_id
        );

        let new_parents = original_commit
            .parents()
            .map(|parent| {
                let p = parent.unwrap();
                if let Some(rewritten_parent) = old_to_new.get(&p) {
                    rewritten_parent
                } else {
                    &p
                }
                .id()
                .clone()
            })
            .collect();

        if send.dry_run {
            // We actually do add Change-ID, but we discard the transaction,
            // so from a user's perspective, we don't.
            write!(ui.status(), "Dry-run: would have added Change-Id to ")?;
        } else {
            write!(ui.status(), "Added Change-Id footer to ")?;
        }
        if let Some(mut formatter) = ui.status_formatter() {
            tx.write_commit_summary(formatter.as_mut(), &original_commit)?;
        }
        writeln!(ui.status())?;

        // rewrite the set of parents to point to the commits that were
        // previously rewritten in toposort order
        //
        // TODO FIXME (aseipp): this whole dance with toposorting, calculating
        // new_parents, and then doing rewrite_commit is roughly equivalent to
        // what we do in duplicate.rs as well. we should probably refactor this?
        let new_commit = tx
            .repo_mut()
            .rewrite_commit(&original_commit)
            .set_description(new_description)
            .set_parents(new_parents)
            .write()?;

        old_to_new.insert(original_commit.clone(), new_commit.clone());
    }
    writeln!(ui.stderr())?;

    if send.dry_run {
        writeln!(
            ui.stderr(),
            "Found {} commits to push to Gerrit (remote '{}'), target branch '{}'",
            to_send.len(),
            remote,
            remote_branch,
        )?;
        for commit in to_send {
            write!(ui.stderr(), "Would push ")?;
            // Use the base workspace helper so that it doesn't appear as a conflict.
            tx.base_workspace_helper()
                .write_commit_summary(ui.stderr_formatter().as_mut(), &commit)?;
            writeln!(ui.stderr())?;
        }
        writeln!(
            ui.stderr(),
            "Dry-run: not performing push, as `--dry-run` was provided"
        )?;
        return Ok(());
    }

    tx.finish(
        ui,
        format!(
            "adding Change-ID to {} commit(s) for sending to gerrit",
            old_to_new.len()
        ),
    )?;
    let mut tx = workspace_command.start_transaction();
    let base_repo = tx.base_repo().clone();

    let new_commits = old_to_new.values().collect::<Vec<&Commit>>();
    let new_heads = base_repo
        .index()
        .heads(&mut new_commits.iter().map(|c| c.id()))
        .map_err(internal_error)?;
    let remote_ref = format!("refs/for/{remote_branch}");

    writeln!(
        ui.stderr(),
        "Found {} heads to push to Gerrit (remote '{}'), target branch '{}'",
        new_heads.len(),
        remote,
        remote_branch,
    )?;

    for head in &new_heads {
        let head_commit = store.get_commit(head).unwrap();

        write!(ui.stderr(), "    ")?;
        tx.write_commit_summary(ui.stderr_formatter().as_mut(), &head_commit)?;
        writeln!(ui.stderr())?;
    }
    writeln!(ui.stderr())?;

    // NOTE (aseipp): because we are pushing everything to the same remote ref,
    // we have to loop and push each commit one at a time, even though
    // push_updates in theory supports multiple GitRefUpdates at once, because
    // we obviously can't push multiple heads to the same ref.
    for head in &new_heads {
        let head_commit = store.get_commit(head).unwrap();
        let head_id = head_commit.id().clone();

        write!(ui.stderr(), "Pushing ")?;
        tx.write_commit_summary(ui.stderr_formatter().as_mut(), &head_commit)?;
        writeln!(ui.stderr())?;

        // how do we get better errors from the remote? 'git push' tells us
        // about rejected refs AND ALSO '(nothing changed)' when there are no
        // changes to push, but we don't get that here.
        with_remote_git_callbacks(ui, |cb| {
            git::push_updates(
                tx.repo_mut(),
                &git_settings,
                remote.as_ref(),
                &[GitRefUpdate {
                    qualified_name: remote_ref.clone().into(),
                    expected_current_target: None,
                    new_target: Some(head_id),
                }],
                cb,
            )
        })
        // Despite the fact that a manual git push will error out with 'no new
        // changes' if you're up to date, this git backend appears to silently
        // succeed - no idea why.
        // It'd be nice if we could distinguish this. We should ideally succeed,
        // but give the user a warning.
        .map_err(|err| match err {
            git::GitPushError::NoSuchRemote(_)
            | git::GitPushError::RemoteName(_)
            | git::GitPushError::UnexpectedBackend(_) => user_error(err),
            git::GitPushError::Subprocess(_) => {
                user_error_with_message("Internal git error while pushing to gerrit", err)
            }
        })?;
    }

    Ok(())
}
