// Copyright 2026 The Jujutsu Authors
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

use std::sync::Arc;

use bstr::BStr;
use gix::remote::Direction;
use gix::url::Scheme;
use jj_lib::backend::ChangeId;
use jj_lib::commit::Commit;
use jj_lib::git;
use jj_lib::object_id::ObjectId as _;
use jj_lib::settings::UserSettings;
use jj_lib::store::Store;
use jj_lib::trailer::parse_description_trailers;
use thiserror::Error;

use crate::cli_util::short_change_hash;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum ChangeIdError {
    #[error("Invalid change ID \"{change_id}\" in revision {revision}")]
    InvalidChangeId { revision: String, change_id: String },

    #[error("Invalid link footer {footer} in revision {revision}")]
    InvalidLinkFooter { revision: String, footer: String },

    #[error("Multiple Change-Id or Link footers in revision {revision}")]
    MultipleFooters { revision: String },
}

impl From<ChangeIdError> for CommandError {
    fn from(err: ChangeIdError) -> Self {
        user_error(err)
    }
}

/// Determine the gerrit remote to use.
pub fn calculate_gerrit_remote(
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
        let r: &BStr = remote.as_ref();
        return Ok(r.to_string());
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

/// Determine what Gerrit remote branch to use. The logic is:
///
/// 1. If the user specifies a preferred branch, use that
/// 2. If the user has 'gerrit.default-remote-branch' configured, use that
/// 3. Otherwise, bail out
pub fn calculate_gerrit_remote_branch(
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

pub fn get_gerrit_review_url(settings: &UserSettings) -> Result<String, CommandError> {
    settings
        .get_string("gerrit.review-url")
        .map(|url| url.trim_end_matches('/').to_string())
        .map_err(|_| {
            user_error("No gerrit.review-url configured, which is required for this command")
        })
}

pub fn get_gerrit_repo(
    store: &Arc<Store>,
    settings: &UserSettings,
) -> Result<String, CommandError> {
    let git_repo = git::get_git_repo(store)?;
    let remote_name = calculate_gerrit_remote(store, settings, None)?;
    let remote = match git_repo.try_find_remote(remote_name.as_str()) {
        Some(Ok(remote)) => remote,
        Some(Err(e)) => {
            return Err(user_error_with_message(
                format!("Failed to load configured remote {remote_name}"),
                e,
            ));
        }
        None => return Err(user_error(format!("No remote named {remote_name} found"))),
    };
    let remote_url = remote.url(Direction::Push).ok_or_else(|| {
        user_error(format!(
            "The remote {remote_name} is not configured for pushing"
        ))
    })?;
    match remote_url.scheme {
        Scheme::Http | Scheme::Https => {
            let path = remote_url.path.to_string();
            Ok(path
                .strip_suffix(".git")
                .unwrap_or(&path)
                .trim_start_matches('/')
                .to_string())
        }
        _ => Err(user_error(format!(
            "Unsupported remote for query: {}",
            remote_url
        ))),
    }
}

// Calculates the gerrit change ID for a given change.
pub fn gerrit_change_id(commit: &Commit) -> Result<Option<String>, ChangeIdError> {
    let desc_change_ids = parse_description_trailers(commit.description())
        .into_iter()
        .filter_map(|trailer| {
            if trailer.key == "Change-Id" {
                if trailer.value.len() != 41 || !trailer.value.starts_with('I') {
                    Some(Err(ChangeIdError::InvalidChangeId {
                        revision: short_change_hash(commit.change_id()),
                        change_id: trailer.value,
                    }))
                } else {
                    Some(Ok(trailer.value))
                }
            } else if trailer.key == "Link" {
                match trailer.value.split_once("/id/I") {
                    Some((_, id)) => {
                        let full_id = format!("I{id}");
                        if full_id.len() != 41 {
                            Some(Err(ChangeIdError::InvalidLinkFooter {
                                revision: short_change_hash(commit.change_id()),
                                footer: trailer.value,
                            }))
                        } else {
                            Some(Ok(full_id))
                        }
                    }
                    None => Some(Err(ChangeIdError::InvalidLinkFooter {
                        revision: short_change_hash(commit.change_id()),
                        footer: trailer.value,
                    })),
                }
            } else {
                None
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

    if desc_change_ids.len() > 1 {
        return Err(ChangeIdError::MultipleFooters {
            revision: short_change_hash(commit.change_id()),
        });
    }
    Ok(desc_change_ids.into_iter().next())
}

// Generates a change ID based on the jj change ID.
pub fn generate_gerrit_change_id(commit: &Commit) -> String {
    // Gerrit change id is 40 chars, jj change id is 32, so we need padding.
    // To be consistent with `format_gerrit_change_id_trailer``, we pad with
    // 6a6a6964 (hex of "jjid").
    let make_change_id = |id: &ChangeId| format!("I{}6a6a6964", id.hex());
    make_change_id(commit.change_id())
}

/// Generates a commit description containing the gerrit change id in a footer.
pub fn generate_gerrit_description(review_url: Option<&str>, commit: &Commit) -> String {
    let gerrit_change_id = generate_gerrit_change_id(commit);
    let trailers = parse_description_trailers(commit.description());
    let change_id_trailer = if let Some(review_url) = review_url {
        format!("Link: {review_url}/id/{gerrit_change_id}")
    } else {
        format!("Change-Id: {gerrit_change_id}")
    };

    format!(
        "{}{}{}\n",
        commit.description().trim(),
        if trailers.is_empty() { "\n\n" } else { "\n" },
        change_id_trailer,
    )
}

#[cfg(test)]
mod tests {
    use jj_lib::repo::Repo as _;
    use testutils::CommitBuilderExt as _;
    use testutils::TestRepo;

    use super::*;

    #[test]
    fn test_gerrit_change_id() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();
        let empty_tree = store.empty_merged_tree();

        let mut tx = test_repo.repo.start_transaction();

        // Commit with no change ID
        let commit = tx
            .repo_mut()
            .new_commit(vec![store.root_commit_id().clone()], empty_tree.clone())
            .set_description("test 1")
            .write_unwrap();
        let id = gerrit_change_id(&commit).unwrap();
        assert_eq!(id, None);

        // Commit with valid Change-Id footer
        let commit = tx
            .repo_mut()
            .new_commit(vec![store.root_commit_id().clone()], empty_tree.clone())
            .set_description("test\n\nChange-Id: I1234567890123456789012345678901234567890")
            .write_unwrap();
        let id = gerrit_change_id(&commit).unwrap();
        assert_eq!(
            id,
            Some("I1234567890123456789012345678901234567890".to_string())
        );

        // Commit with invalid Change-Id footer
        let commit = tx
            .repo_mut()
            .new_commit(vec![store.root_commit_id().clone()], empty_tree.clone())
            .set_description("test\n\nChange-Id: I12")
            .write_unwrap();
        assert_eq!(
            gerrit_change_id(&commit).unwrap_err(),
            ChangeIdError::InvalidChangeId {
                revision: short_change_hash(commit.change_id()),
                change_id: "I12".to_string(),
            }
        );

        // Commit with valid link footer
        let commit = tx.repo_mut()
            .new_commit(vec![store.root_commit_id().clone()], empty_tree.clone())
            .set_description("test\n\nLink: https://review.example.com/id/I1234567890123456789012345678901234567890")
            .write_unwrap();
        let id = gerrit_change_id(&commit).unwrap();
        assert_eq!(
            id,
            Some("I1234567890123456789012345678901234567890".to_string())
        );

        // Commit with invalid link footer
        let commit = tx
            .repo_mut()
            .new_commit(vec![store.root_commit_id().clone()], empty_tree.clone())
            .set_description("test\n\nLink: I1234567890123456789012345678901234567890")
            .write_unwrap();
        assert_eq!(
            gerrit_change_id(&commit).unwrap_err(),
            ChangeIdError::InvalidLinkFooter {
                revision: short_change_hash(commit.change_id()),
                footer: "I1234567890123456789012345678901234567890".to_string(),
            }
        );

        // Commit with both link and change-id footer
        let commit = tx.repo_mut()
            .new_commit(vec![store.root_commit_id().clone()], empty_tree.clone())
            .set_description("test\n\nChange-Id: I1234567890123456789012345678901234567890\nLink: https://review.example.com/id/I0987654321098765432109876543210987654321")
            .write_unwrap();
        assert_eq!(
            gerrit_change_id(&commit).unwrap_err(),
            ChangeIdError::MultipleFooters {
                revision: short_change_hash(commit.change_id()),
            }
        );
    }

    #[test]
    fn test_generate_gerrit_change_id() {
        let test_repo = TestRepo::init();
        let store = test_repo.repo.store();
        let empty_tree = store.empty_merged_tree();

        let mut tx = test_repo.repo.start_transaction();

        let commit = tx
            .repo_mut()
            .new_commit(vec![store.root_commit_id().clone()], empty_tree.clone())
            .set_description("test 1")
            .write_unwrap();

        let new_desc = generate_gerrit_description(None, &commit);

        let expected_gerrit_id = "I781199f9d55d18e855a7aa84c5e4b40d6a6a6964";
        assert_eq!(
            new_desc,
            format!("test 1\n\nChange-Id: {expected_gerrit_id}\n")
        );

        let commit = tx
            .repo_mut()
            .new_commit(vec![store.root_commit_id().clone()], empty_tree.clone())
            .set_description("test 1")
            .write_unwrap();

        let new_desc = generate_gerrit_description(Some("https://review.example.com"), &commit);

        let expected_gerrit_id = "Ia2c96fc88f32e487328f04927f20c4b16a6a6964";
        assert_eq!(
            new_desc,
            format!("test 1\n\nLink: https://review.example.com/id/{expected_gerrit_id}\n")
        );
    }
}
