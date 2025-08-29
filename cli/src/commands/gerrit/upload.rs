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
use indexmap::IndexMap;
use itertools::Itertools as _;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::commit::CommitIteratorExt as _;
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
use crate::cli_util::short_commit_hash;
use crate::command_error::CommandError;
use crate::command_error::internal_error;
use crate::command_error::user_error;
use crate::command_error::user_error_with_hint;
use crate::command_error::user_error_with_message;
use crate::git_util::with_remote_git_callbacks;
use crate::ui::Ui;

#[derive(clap::Args, Clone, Debug)]
pub struct UploadArgs {
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

pub fn cmd_upload(
    ui: &mut Ui,
    command: &CommandHelper,
    upload: &UploadArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;

    let revisions: Vec<_> = workspace_command
        .parse_union_revsets(ui, &upload.revisions)?
        .evaluate_to_commits()?
        .try_collect()?;
    if revisions.is_empty() {
        writeln!(ui.status(), "No revisions to upload.")?;
        return Ok(());
    }

    if revisions
        .iter()
        .any(|commit| commit.id() == workspace_command.repo().store().root_commit_id())
    {
        return Err(user_error("Cannot upload the virtual 'root()' commit"));
    }

    workspace_command.check_rewritable(revisions.iter().ids())?;

    // If you have the changes main -> A -> B, and then run `jj gerrit upload B`,
    // then that uploads both A and B. Thus, we need to ensure that A also
    // has a Change-ID.
    // We make an assumption here that all immutable commits already have a
    // Change-ID.
    let to_upload: Vec<Commit> = workspace_command
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

    let old_heads = base_repo
        .index()
        .heads(&mut revisions.iter().ids())
        .map_err(internal_error)?;

    let git_settings = command.settings().git_settings()?;
    let remote = calculate_push_remote(store, command.settings(), upload.remote.clone())?;
    let remote_branch = calculate_push_ref(command.settings(), upload.remote_branch.clone())?;

    // immediately error and reject any discardable commits, i.e. the
    // the empty wcc
    for commit in &to_upload {
        if commit.is_discardable(tx.repo_mut())? {
            return Err(user_error_with_hint(
                format!(
                    "Refusing to upload commit {} because it is an empty commit with no \
                     description",
                    short_commit_hash(commit.id())
                ),
                "Perhaps you squashed then ran upload? Maybe you meant to upload the parent \
                 commit instead (eg. @-)",
            ));
        }
    }

    let mut old_to_new: IndexMap<CommitId, Commit> = IndexMap::new();
    for commit_id in to_upload.iter().map(|c| c.id()).rev() {
        let original_commit = store.get_commit(commit_id).unwrap();
        let description = original_commit.description().to_owned();
        let trailers = parse_description_trailers(&description);

        let change_id_trailers: Vec<&Trailer> = trailers
            .iter()
            .filter(|trailer| trailer.key == "Change-Id")
            .collect();

        // There shouldn't be multiple change-ID fields. So just error out if
        // there is.
        if change_id_trailers.len() > 1 {
            return Err(user_error(format!(
                "multiple Change-Id footers in commit {}",
                short_commit_hash(commit_id)
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
                    "warning: invalid Change-Id footer in commit {}",
                    short_commit_hash(original_commit.id()),
                )?;
            }

            // map the old commit to itself
            old_to_new.insert(original_commit.id().clone(), original_commit.clone());
            continue;
        }

        // Gerrit change id is 40 chars, jj change id is 32, so we need padding.
        // To be consistent with `format_gerrit_change_id_trailer``, we pad with
        // 6a6a6964 (hex of "jjid").
        let gerrit_change_id = format!("I6a6a6964{}", original_commit.change_id().hex());

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
                if let Some(rewritten_parent) = old_to_new.get(p.id()) {
                    rewritten_parent
                } else {
                    &p
                }
                .id()
                .clone()
            })
            .collect();

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
            // Set the timestamp back to the timestamp of the original commit.
            // Otherwise, `jj gerrit upload @ && jj gerrit upload @` will upload
            // two patchsets with the only difference being the timestamp.
            .set_committer(original_commit.committer().clone())
            .set_author(original_commit.author().clone())
            .write()?;

        old_to_new.insert(original_commit.id().clone(), new_commit.clone());
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
            if upload.dry_run {
                "Dry-run: Would push "
            } else {
                "Pushing "
            }
        )?;
        // We have to write the old commit here, because the until we finish
        // the transaction (which we don't), the new commit is labelled as
        // "hidden".
        tx.base_workspace_helper().write_commit_summary(
            ui.stderr_formatter().as_mut(),
            &store.get_commit(head).unwrap(),
        )?;
        writeln!(ui.stderr())?;

        if upload.dry_run {
            continue;
        }

        let new_commit = store
            .get_commit(old_to_new.get(head).unwrap().id())
            .unwrap();

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
