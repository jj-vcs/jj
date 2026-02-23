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

use jj_lib::backend::ChangeId;
use jj_lib::commit::Commit;
use jj_lib::object_id::ObjectId as _;
use jj_lib::ref_name::RefNameBuf;
use jj_lib::repo::Repo;
use jj_lib::settings::UserSettings;
use jj_lib::trailer::parse_description_trailers;
use thiserror::Error;

use crate::cli_util::short_change_hash;
use crate::command_error::CommandError;
use crate::command_error::user_error;

#[derive(Clone, Debug, PartialEq, Eq, strum_macros::Display)]
pub enum ChangeIdSource {
    #[strum(serialize = "a Link footer")]
    LinkFooter,
    #[strum(serialize = "a Change-Id footer")]
    ChangeIdFooter,
    #[strum(serialize = "a gerrit-* bookmark")]
    Bookmark,
    #[strum(serialize = "the JJ Change ID")]
    Generated,
}

#[derive(Error, Debug, PartialEq, Eq)]
pub enum ChangeIdError {
    #[error("Invalid change ID \"{change_id}\" for revision {revision} from {id_source}")]
    InvalidChangeId {
        revision: String,
        change_id: String,
        id_source: ChangeIdSource,
    },

    #[error("Invalid link footer {footer} for revision {revision}")]
    InvalidLinkFooter { revision: String, footer: String },

    #[error("Multiple Change-Id / Link footers for revision {revision}")]
    MultipleFooters { revision: String },

    #[error("Multiple gerrit-* bookmarks for revision {revision}")]
    MultipleBookmarks { revision: String },

    #[error("Multiple gerrit Change-Ids for revision {revision}")]
    MultipleChangeIds { revision: String },

    #[error(
        "Revision {revision} has both a bookmark gerrit-{bookmark_id} and {footer_source} \
         {footer_id}"
    )]
    MultipleChangeIdsAndFooters {
        revision: String,
        bookmark_id: String,
        footer_source: ChangeIdSource,
        footer_id: String,
    },
}

impl From<ChangeIdError> for CommandError {
    fn from(err: ChangeIdError) -> Self {
        match err {
            ChangeIdError::MultipleBookmarks { .. } => user_error(err).hinted(
                "You will need to run `jj bookmark forget/move` on one of them to resolve the \
                 ambiguity",
            ),
            _ => user_error(err),
        }
    }
}

pub fn get_review_url(settings: &UserSettings) -> Result<String, CommandError> {
    settings
        .get_string("gerrit.review-url")
        .map(|url| url.trim_end_matches('/').to_string())
        .map_err(|_| {
            user_error("No gerrit.review-url configured, which is required for this command")
        })
}

// Calculates the gerrit change ID for a given change.
pub fn gerrit_change_id(
    repo: &dyn Repo,
    commit: &Commit,
) -> Result<Option<(String, ChangeIdSource)>, ChangeIdError> {
    let desc_change_ids = parse_description_trailers(commit.description())
        .into_iter()
        .filter_map(|trailer| {
            if trailer.key == "Change-Id" {
                Some(Ok((trailer.value, ChangeIdSource::ChangeIdFooter)))
            } else if trailer.key == "Link" {
                match trailer.value.split_once("/id/I") {
                    Some((_, id)) => Some(Ok((format!("I{id}"), ChangeIdSource::LinkFooter))),
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

    let bookmark_change_ids = repo
        .view()
        .local_bookmarks_for_commit(commit.id())
        .filter_map(|(name, _)| name.as_str().strip_prefix("gerrit-"))
        .map(|id| (id.to_string(), ChangeIdSource::Bookmark))
        .collect::<Vec<_>>();

    if bookmark_change_ids.len() > 1 {
        return Err(ChangeIdError::MultipleBookmarks {
            revision: short_change_hash(commit.change_id()),
        });
    }

    if let Some((book_id, _)) = bookmark_change_ids.first()
        && let Some((desc_id, desc_source)) = desc_change_ids.first()
        && book_id != desc_id
    {
        return Err(ChangeIdError::MultipleChangeIdsAndFooters {
            revision: short_change_hash(commit.change_id()),
            bookmark_id: book_id.clone(),
            footer_source: desc_source.clone(),
            footer_id: desc_id.clone(),
        });
    }

    bookmark_change_ids
        .first()
        .or(desc_change_ids.first())
        .map(|(id, source)| {
            if !id.starts_with('I') || id.len() != 41 {
                Err(ChangeIdError::InvalidChangeId {
                    revision: short_change_hash(commit.change_id()),
                    change_id: id.clone(),
                    id_source: source.clone(),
                })
            } else {
                Ok((id.clone(), source.clone()))
            }
        })
        .transpose()
}

// Generates a change ID based on the jj change ID.
// Returns the new commit description, the gerrit change ID, and where the
// change ID came from.
pub fn generate_gerrit_change_id(
    repo: &dyn Repo,
    settings: &UserSettings,
    commit: &Commit,
) -> Result<(String, String, ChangeIdSource), ChangeIdError> {
    match gerrit_change_id(repo, commit)? {
        Some((change_id, source)) => Ok((commit.description().to_string(), change_id, source)),
        None => {
            let trailers = parse_description_trailers(commit.description());
            // Gerrit change id is 40 chars, jj change id is 32, so we need padding.
            // To be consistent with `format_gerrit_change_id_trailer``, we pad with
            // 6a6a6964 (hex of "jjid").
            let make_change_id = |id: &ChangeId| format!("I{}6a6a6964", id.hex());
            let mut gerrit_change_id = make_change_id(commit.change_id());
            // The bookmark may have been moved from this change to another.
            // In this case, we assume they intend to disassociate this change
            // with the Change-Id, so we create a new Change-Id to avoid conflicts.
            if repo
                .view()
                .get_local_bookmark(&RefNameBuf::from(format!("gerrit-{gerrit_change_id}")))
                .is_present()
            {
                gerrit_change_id = make_change_id(
                    &settings
                        .get_rng()
                        .new_change_id(repo.store().change_id_length()),
                );
            }
            let change_id_trailer = if let Ok(review_url) = get_review_url(settings) {
                format!("Link: {review_url}/id/{gerrit_change_id}")
            } else {
                format!("Change-Id: {gerrit_change_id}")
            };

            Ok((
                format!(
                    "{}{}{}\n",
                    commit.description().trim(),
                    if trailers.is_empty() { "\n\n" } else { "\n" },
                    change_id_trailer,
                ),
                gerrit_change_id,
                ChangeIdSource::Generated,
            ))
        }
    }
}

pub fn review_url_for_commit(
    repo: &dyn Repo,
    commit: &Commit,
    review_url: &str,
) -> Result<Option<String>, ChangeIdError> {
    Ok(gerrit_change_id(repo, commit)?.map(|(id, _)| format!("{review_url}/id/{id}")))
}

#[cfg(test)]
mod tests {
    use jj_lib::op_store::RefTarget;
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
        let commit1 = tx
            .repo_mut()
            .new_commit(vec![store.root_commit_id().clone()], empty_tree.clone())
            .set_description("test 1")
            .write_unwrap();
        let id1 = gerrit_change_id(tx.repo(), &commit1).unwrap();
        assert_eq!(id1, None);

        // Commit with valid Change-Id footer
        let commit = tx
            .repo_mut()
            .new_commit(vec![store.root_commit_id().clone()], empty_tree.clone())
            .set_description("test\n\nChange-Id: I1234567890123456789012345678901234567890")
            .write_unwrap();
        let id2 = gerrit_change_id(tx.repo(), &commit).unwrap();
        assert_eq!(
            id2,
            Some((
                "I1234567890123456789012345678901234567890".to_string(),
                ChangeIdSource::ChangeIdFooter
            ))
        );

        // Commit with invalid Change-Id footer
        let commit = tx
            .repo_mut()
            .new_commit(vec![store.root_commit_id().clone()], empty_tree.clone())
            .set_description("test\n\nChange-Id: I12")
            .write_unwrap();
        assert_eq!(
            gerrit_change_id(tx.repo(), &commit).unwrap_err(),
            ChangeIdError::InvalidChangeId {
                revision: short_change_hash(commit.change_id()),
                change_id: "I12".to_string(),
                id_source: ChangeIdSource::ChangeIdFooter,
            }
        );

        // Commit with valid link footer
        let commit = tx.repo_mut()
            .new_commit(vec![store.root_commit_id().clone()], empty_tree.clone())
            .set_description("test\n\nLink: https://review.example.com/id/I1234567890123456789012345678901234567890")
            .write_unwrap();
        let id = gerrit_change_id(tx.repo(), &commit).unwrap();
        assert_eq!(
            id,
            Some((
                "I1234567890123456789012345678901234567890".to_string(),
                ChangeIdSource::LinkFooter
            ))
        );

        // Commit with invalid link footer
        let commit = tx
            .repo_mut()
            .new_commit(vec![store.root_commit_id().clone()], empty_tree.clone())
            .set_description("test\n\nLink: I1234567890123456789012345678901234567890")
            .write_unwrap();
        assert_eq!(
            gerrit_change_id(tx.repo(), &commit).unwrap_err(),
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
            gerrit_change_id(tx.repo(), &commit).unwrap_err(),
            ChangeIdError::MultipleFooters {
                revision: short_change_hash(commit.change_id()),
            }
        );

        // Commit with multiple bookmarks
        let commit = tx
            .repo_mut()
            .new_commit(vec![store.root_commit_id().clone()], empty_tree.clone())
            .set_description("test")
            .write_unwrap();
        tx.repo_mut().set_local_bookmark_target(
            &RefNameBuf::from("gerrit-Iaaaaabbbbbcccccdddddeeeeefffffggggghhhhh"),
            RefTarget::normal(commit.id().clone()),
        );
        tx.repo_mut().set_local_bookmark_target(
            &RefNameBuf::from("gerrit-Ibbbbbaaaaacccccdddddeeeeefffffggggghhhhh"),
            RefTarget::normal(commit.id().clone()),
        );
        assert_eq!(
            gerrit_change_id(tx.repo(), &commit).unwrap_err(),
            ChangeIdError::MultipleBookmarks {
                revision: short_change_hash(commit.change_id()),
            }
        );

        // Commit with matching change-Id and bookmark footer
        let commit = tx
            .repo_mut()
            .new_commit(vec![store.root_commit_id().clone()], empty_tree.clone())
            .set_description("test\n\nChange-Id: Iaaaaabbbbbcccccdddddeeeeefffffggggghhhhh")
            .write_unwrap();
        tx.repo_mut().set_local_bookmark_target(
            &RefNameBuf::from("gerrit-Iaaaaabbbbbcccccdddddeeeeefffffggggghhhhh"),
            RefTarget::normal(commit.id().clone()),
        );
        let id = gerrit_change_id(tx.repo(), &commit).unwrap();
        assert_eq!(
            id,
            Some((
                "Iaaaaabbbbbcccccdddddeeeeefffffggggghhhhh".to_string(),
                ChangeIdSource::Bookmark
            ))
        );

        // Commit with non-matching change-Id and bookmark footer
        let commit = tx
            .repo_mut()
            .new_commit(vec![store.root_commit_id().clone()], empty_tree.clone())
            .set_description("test\n\nChange-Id: Ibbbbbaaaaacccccdddddeeeeefffffggggghhhhh")
            .write_unwrap();
        tx.repo_mut().set_local_bookmark_target(
            &RefNameBuf::from("gerrit-Iaaaaabbbbbcccccdddddeeeeefffffggggghhhhh"),
            RefTarget::normal(commit.id().clone()),
        );
        assert_eq!(
            gerrit_change_id(tx.repo(), &commit).unwrap_err(),
            ChangeIdError::MultipleChangeIdsAndFooters {
                revision: short_change_hash(commit.change_id()),
                bookmark_id: "Iaaaaabbbbbcccccdddddeeeeefffffggggghhhhh".to_string(),
                footer_source: ChangeIdSource::ChangeIdFooter,
                footer_id: "Ibbbbbaaaaacccccdddddeeeeefffffggggghhhhh".to_string(),
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

        let settings = testutils::user_settings();
        let (new_desc, gerrit_id, source) =
            generate_gerrit_change_id(tx.repo(), &settings, &commit).unwrap();

        let expected_gerrit_id = "I781199f9d55d18e855a7aa84c5e4b40d6a6a6964";
        assert_eq!(source, ChangeIdSource::Generated);
        assert_eq!(gerrit_id, expected_gerrit_id);
        assert_eq!(
            new_desc,
            format!("test 1\n\nChange-Id: {expected_gerrit_id}\n")
        );

        let mut config = testutils::base_user_config();
        config.add_layer(
            jj_lib::config::ConfigLayer::parse(
                jj_lib::config::ConfigSource::User,
                "gerrit.review-url = \"https://review.example.com/\"",
            )
            .unwrap(),
        );
        let settings = jj_lib::settings::UserSettings::from_config(config).unwrap();

        let commit = tx
            .repo_mut()
            .new_commit(vec![store.root_commit_id().clone()], empty_tree.clone())
            .set_description("test 1")
            .write_unwrap();

        let (new_desc, gerrit_id, source) =
            generate_gerrit_change_id(tx.repo(), &settings, &commit).unwrap();

        let expected_gerrit_id = "Ia2c96fc88f32e487328f04927f20c4b16a6a6964";
        assert_eq!(source, ChangeIdSource::Generated);
        assert_eq!(gerrit_id, expected_gerrit_id);
        assert_eq!(
            new_desc,
            format!("test 1\n\nLink: https://review.example.com/id/{expected_gerrit_id}\n")
        );
    }
}
