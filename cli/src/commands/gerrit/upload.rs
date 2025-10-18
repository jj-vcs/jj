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

use std::collections::HashMap;
use std::fmt::Debug;
use std::io::Write as _;
use std::sync::Arc;

use bstr::BStr;
use itertools::Itertools as _;
use jj_lib::backend::BackendError;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::git::GitRefUpdate;
use jj_lib::git::{self};
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo::Repo as _;
use jj_lib::revset::RevsetExpression;
use jj_lib::settings::UserSettings;
use jj_lib::store::Store;
use jj_lib::trailer::Trailer;
use jj_lib::trailer::parse_description_trailers;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::short_change_hash;
use crate::command_error::CommandError;
use crate::command_error::internal_error;
use crate::command_error::user_error;
use crate::command_error::user_error_with_hint;
use crate::command_error::user_error_with_message;
use crate::git_util::with_remote_git_callbacks;
use crate::ui::Ui;

/// Upload changes to Gerrit for code review, or update existing changes.
///
/// Uploading in a set of revisions to Gerrit creates a single "change" for
/// each revision included in the revset. These changes are then available
/// for review on your Gerrit instance.
///
/// Note: The gerrit commit Id may not match that of your local commit Id,
/// since we add a `Change-Id` footer to the commit message if one does not
/// already exist. This ID is based off the jj Change-Id, but is not the same.
///
/// If a change already exists for a given revision (i.e. it contains the
/// same `Change-Id`), this command will update the contents of the existing
/// change to match.
///
/// Note: this command takes 1-or-more revsets arguments, each of which can
/// resolve to multiple revisions; so you may post trees or ranges of
/// commits to Gerrit for review all at once.
#[derive(clap::Args, Clone, Debug)]
pub struct UploadArgs {
    /// The revset, selecting which revisions are sent in to Gerrit
    ///
    /// This can be any arbitrary set of commits. Note that when you push a
    /// commit at the head of a stack, all ancestors are pushed too. This means
    /// that `jj gerrit upload -r foo` is equivalent to `jj gerrit upload -r
    /// 'mutable()::foo`.
    #[arg(long, short = 'r')]
    revisions: Vec<RevisionArg>,

    /// The location where your changes are intended to land
    ///
    /// This should be a branch on the remote. Can be configured with the
    /// `gerrit.default-branch` repository option.
    #[arg(long = "remote-branch", short = 'b')]
    remote_branch: Option<String>,

    /// The Gerrit remote to push to
    ///
    /// Can be configured with the `gerrit.default-remote` repository option as
    /// well. This is typically a full SSH URL for your Gerrit instance.
    #[arg(long)]
    remote: Option<String>,

    /// Do not actually push the changes to Gerrit
    #[arg(long = "dry-run", short = 'n')]
    dry_run: bool,
}

fn calculate_push_remote(
    store: &Arc<Store>,
    settings: &UserSettings,
    remote: Option<&str>,
) -> Result<String, CommandError> {
    let git_repo = git::get_git_repo(store)?; // will fail if not a git repo
    let remotes = git_repo.remote_names();

    // If --remote was provided, use that
    if let Some(remote) = remote {
        if remotes.contains(BStr::new(&remote)) {
            return Ok(remote.to_string());
        }
        return Err(user_error(format!(
            "The remote '{remote}' (specified via `--remote`) does not exist",
        )));
    }

    // If the Gerrit-specific config was set, use that
    if let Ok(remote) = settings.get_string("gerrit.default-remote") {
        if remotes.contains(BStr::new(&remote)) {
            return Ok(remote);
        }
        return Err(user_error(format!(
            "The remote '{remote}' (configured via `gerrit.default-remote`) does not exist",
        )));
    }

    // If a general push remote was configured, use that
    if let Some(remote) = git_repo.remote_default_name(gix::remote::Direction::Push) {
        return Ok(remote.to_string());
    }

    // If there is a Git remote called "gerrit", use that
    if remotes.iter().any(|r| **r == "gerrit") {
        return Ok("gerrit".to_owned());
    }

    // Otherwise error out
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
    settings: &UserSettings,
    remote_branch: Option<String>,
) -> Result<String, CommandError> {
    // case 1
    if let Some(remote_branch) = remote_branch {
        return Ok(remote_branch);
    }

    // case 2
    if let Ok(branch) = settings.get_string("gerrit.default-remote-branch") {
        return Ok(branch);
    }

    // case 3
    Err(user_error(
        "No target branch specified via --remote-branch, and no 'gerrit.default-remote-branch' \
         was found",
    ))
}

pub fn cmd_gerrit_upload(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &UploadArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    let target_expr = workspace_command
        .parse_union_revsets(ui, &args.revisions)?
        .resolve()?;
    workspace_command.check_rewritable_expr(&target_expr)?;
    let revisions: Vec<_> = target_expr
        .evaluate(workspace_command.repo().as_ref())?
        .iter()
        .try_collect()?;
    if revisions.is_empty() {
        writeln!(ui.status(), "No revisions to upload.")?;
        return Ok(());
    }

    // If you have the changes main -> A -> B, and then run `jj gerrit upload -r B`,
    // then that uploads both A and B. Thus, we need to ensure that A also
    // has a Change-ID.
    // We make an assumption here that all immutable commits already have a
    // Change-ID.
    let to_upload: Vec<Commit> = workspace_command
        .attach_revset_evaluator(
            workspace_command
                .env()
                .immutable_expression()
                .range(&RevsetExpression::commits(revisions.clone())),
        )
        .evaluate_to_commits()?
        .try_collect()?;

    // Note: This transaction is intentionally never finished. This way, the
    // Change-Id is never part of the commit description in jj.
    // This avoids scenarios where you have many commits with the same
    // Change-Id, or a single commit with many Change-Ids after running
    // jj split / jj squash respectively.
    // If a user doesn't like this behavior, they can add the following to
    // their Cargo.toml.
    // commit_trailers = 'if(!trailers.contains_key("Change-Id"),
    // format_gerrit_change_id_trailer(self))'
    let mut tx = workspace_command.start_transaction();
    let base_repo = tx.base_repo();
    let store = base_repo.store().clone();

    let old_heads = base_repo
        .index()
        .heads(&mut revisions.iter())
        .map_err(internal_error)?;

    let git_settings = command.settings().git_settings()?;
    let remote = calculate_push_remote(&store, command.settings(), args.remote.as_deref())?;
    let remote_branch = calculate_push_ref(command.settings(), args.remote_branch.clone())?;

    // Immediately error and reject any commits that shouldn't be uploaded.
    for commit in &to_upload {
        if commit.is_empty(tx.repo_mut())? {
            return Err(user_error_with_hint(
                format!(
                    "Refusing to upload revision {} because it is empty",
                    short_change_hash(commit.change_id())
                ),
                "Perhaps you squashed then ran upload? Maybe you meant to upload the parent \
                 commit instead (eg. @-)",
            ));
        }
        if commit.description().is_empty() {
            return Err(user_error_with_hint(
                format!(
                    "Refusing to upload revision {} because it is has no description",
                    short_change_hash(commit.change_id())
                ),
                "Maybe you meant to upload the parent commit instead (eg. @-)",
            ));
        }
    }

    let mut old_to_new: HashMap<CommitId, Commit> = HashMap::new();
    for original_commit in to_upload.into_iter().rev() {
        let trailers = parse_description_trailers(original_commit.description());

        let change_id_trailers: Vec<&Trailer> = trailers
            .iter()
            .filter(|trailer| trailer.key == "Change-Id")
            .collect();

        // There shouldn't be multiple change-ID fields. So just error out if
        // there is.
        if change_id_trailers.len() > 1 {
            return Err(user_error(format!(
                "multiple Change-Id footers in revision {}",
                short_change_hash(original_commit.change_id())
            )));
        }

        // The user can choose to explicitly set their own change-ID to
        // override the default change-ID based on the jj change-ID.
        if let Some(trailer) = change_id_trailers.first() {
            // Check the change-id format is correct.
            if trailer.value.len() != 41 || !trailer.value.starts_with('I') {
                // Intentionally leave the invalid change IDs as-is.
                writeln!(
                    ui.warning_default(),
                    "warning: invalid Change-Id footer in revision {}",
                    short_change_hash(original_commit.change_id()),
                )?;
            }

            // map the old commit to itself
            old_to_new.insert(original_commit.id().clone(), original_commit);
            continue;
        }

        // Gerrit change id is 40 chars, jj change id is 32, so we need padding.
        // To be consistent with `format_gerrit_change_id_trailer``, we pad with
        // 6a6a6964 (hex of "jjid").
        let gerrit_change_id = format!("I{}6a6a6964", original_commit.change_id().hex());

        let new_description = format!(
            "{}{}Change-Id: {}\n",
            original_commit.description().trim(),
            if trailers.is_empty() { "\n\n" } else { "\n" },
            gerrit_change_id
        );

        let new_parents = original_commit
            .parents()
            .map(|parent| -> Result<CommitId, BackendError> {
                let p = parent?;
                Ok(old_to_new.get(p.id()).unwrap_or(&p).id().clone())
            })
            .try_collect()?;

        // rewrite the set of parents to point to the commits that were
        // previously rewritten in toposort order
        let new_commit = tx
            .repo_mut()
            .rewrite_commit(&original_commit)
            .set_description(new_description)
            .set_parents(new_parents)
            // Set the timestamp back to the timestamp of the original commit.
            // Otherwise, `jj gerrit upload @ && jj gerrit upload @` will upload
            // two patchsets with the only difference being the timestamp.
            .set_committer(original_commit.committer().clone())
            .set_author(original_commit.author().clone())
            .write()?;

        old_to_new.insert(original_commit.id().clone(), new_commit);
    }
    writeln!(ui.stderr())?;

    let remote_ref = format!("refs/for/{remote_branch}");
    writeln!(
        ui.stderr(),
        "Found {} heads to push to Gerrit (remote '{}'), target branch '{}'",
        old_heads.len(),
        remote,
        remote_branch,
    )?;

    writeln!(ui.stderr())?;

    // NOTE (aseipp): because we are pushing everything to the same remote ref,
    // we have to loop and push each commit one at a time, even though
    // push_updates in theory supports multiple GitRefUpdates at once, because
    // we obviously can't push multiple heads to the same ref.
    for head in &old_heads {
        write!(
            ui.stderr(),
            "{}",
            if args.dry_run {
                "Dry-run: Would push "
            } else {
                "Pushing "
            }
        )?;
        // We have to write the old commit here, because until we finish
        // the transaction (which we don't), the new commit is labeled as
        // "hidden".
        tx.base_workspace_helper().write_commit_summary(
            ui.stderr_formatter().as_mut(),
            &store.get_commit(head).unwrap(),
        )?;
        writeln!(ui.stderr())?;

        if args.dry_run {
            continue;
        }

        let new_commit = old_to_new.get(head).unwrap();

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
                    new_target: Some(new_commit.id().clone()),
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
