// Copyright 2020-2025 The Jujutsu Authors
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

//! Types and functions for listing bookmark and tags.

use std::cmp;
use std::collections::HashMap;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use clap::ValueEnum;
use itertools::Itertools as _;
use jj_lib::backend;
use jj_lib::backend::BackendResult;
use jj_lib::backend::CommitId;
use jj_lib::config::ConfigValue;
use jj_lib::store::Store;
use jj_lib::str_util::StringMatcher;

use crate::commit_templater::CommitRef;

#[derive(Clone, Debug)]
pub struct RefListItem {
    /// Local or untracked remote ref.
    pub primary: Rc<CommitRef>,
    /// Remote refs tracked by the primary (or local) ref.
    pub tracked: Vec<Rc<CommitRef>>,
}

/// Conditions to select local/remote refs.
pub struct RefFilterPredicates {
    /// Matches local names.
    pub name_matcher: StringMatcher,
    /// Matches remote names.
    pub remote_matcher: StringMatcher,
    /// Matches any of the local targets.
    pub matched_local_targets: HashSet<CommitId>,
    /// Selects local refs having conflicted targets.
    pub conflicted: bool,
    /// Includes local-only refs.
    pub include_local_only: bool,
    /// Includes tracked remote refs pointing to the same local targets.
    pub include_synced_remotes: bool,
    /// Includes untracked remote refs.
    pub include_untracked_remotes: bool,
}

/// Sort key for the `--sort` argument option.
#[derive(Copy, Clone, PartialEq, Debug, ValueEnum)]
pub enum SortKey {
    Name,
    #[value(name = "name-")]
    NameDesc,
    AuthorName,
    #[value(name = "author-name-")]
    AuthorNameDesc,
    AuthorEmail,
    #[value(name = "author-email-")]
    AuthorEmailDesc,
    AuthorDate,
    #[value(name = "author-date-")]
    AuthorDateDesc,
    CommitterName,
    #[value(name = "committer-name-")]
    CommitterNameDesc,
    CommitterEmail,
    #[value(name = "committer-email-")]
    CommitterEmailDesc,
    CommitterDate,
    #[value(name = "committer-date-")]
    CommitterDateDesc,
}

impl SortKey {
    fn is_commit_dependant(&self) -> bool {
        match self {
            Self::Name | Self::NameDesc => false,
            Self::AuthorName
            | Self::AuthorNameDesc
            | Self::AuthorEmail
            | Self::AuthorEmailDesc
            | Self::AuthorDate
            | Self::AuthorDateDesc
            | Self::CommitterName
            | Self::CommitterNameDesc
            | Self::CommitterEmail
            | Self::CommitterEmailDesc
            | Self::CommitterDate
            | Self::CommitterDateDesc => true,
        }
    }
}

pub fn parse_sort_keys(value: ConfigValue) -> Result<Vec<SortKey>, String> {
    if let Some(array) = value.as_array() {
        array
            .iter()
            .map(|item| {
                item.as_str()
                    .ok_or("Expected sort key as a string".to_owned())
                    .and_then(|key| SortKey::from_str(key, false))
            })
            .try_collect()
    } else {
        Err("Expected an array of sort keys as strings".to_owned())
    }
}

/// Sorts `items` by multiple `sort_keys`.
///
/// The first key is most significant. The input items should have been sorted
/// by [`SortKey::Name`].
pub fn sort(
    store: &Arc<Store>,
    items: &mut [RefListItem],
    sort_keys: &[SortKey],
) -> BackendResult<()> {
    let mut commits: HashMap<CommitId, Arc<backend::Commit>> = HashMap::new();
    if sort_keys.iter().any(|key| key.is_commit_dependant()) {
        commits = items
            .iter()
            .filter_map(|item| item.primary.target().added_ids().next())
            .map(|commit_id| {
                store
                    .get_commit(commit_id)
                    .map(|commit| (commit_id.clone(), commit.store_commit().clone()))
            })
            .try_collect()?;
    }
    sort_inner(items, sort_keys, &commits);
    Ok(())
}

fn sort_inner(
    items: &mut [RefListItem],
    sort_keys: &[SortKey],
    commits: &HashMap<CommitId, Arc<backend::Commit>>,
) {
    let to_commit = |item: &RefListItem| {
        let id = item.primary.target().added_ids().next()?;
        commits.get(id)
    };

    // Multi-pass sorting, the first key is most significant. Skip first
    // iteration if sort key is `Name`, since items are already sorted by name.
    for sort_key in sort_keys
        .iter()
        .rev()
        .skip_while(|key| *key == &SortKey::Name)
    {
        match sort_key {
            SortKey::Name => {
                items.sort_by_key(|item| {
                    (
                        item.primary.name().to_owned(),
                        item.primary.remote_name().map(|name| name.to_owned()),
                    )
                });
            }
            SortKey::NameDesc => {
                items.sort_by_key(|item| {
                    cmp::Reverse((
                        item.primary.name().to_owned(),
                        item.primary.remote_name().map(|name| name.to_owned()),
                    ))
                });
            }
            SortKey::AuthorName => {
                items.sort_by_key(|item| to_commit(item).map(|commit| commit.author.name.as_str()));
            }
            SortKey::AuthorNameDesc => {
                items.sort_by_key(|item| {
                    cmp::Reverse(to_commit(item).map(|commit| commit.author.name.as_str()))
                });
            }
            SortKey::AuthorEmail => {
                items
                    .sort_by_key(|item| to_commit(item).map(|commit| commit.author.email.as_str()));
            }
            SortKey::AuthorEmailDesc => {
                items.sort_by_key(|item| {
                    cmp::Reverse(to_commit(item).map(|commit| commit.author.email.as_str()))
                });
            }
            SortKey::AuthorDate => {
                items.sort_by_key(|item| to_commit(item).map(|commit| commit.author.timestamp));
            }
            SortKey::AuthorDateDesc => {
                items.sort_by_key(|item| {
                    cmp::Reverse(to_commit(item).map(|commit| commit.author.timestamp))
                });
            }
            SortKey::CommitterName => {
                items.sort_by_key(|item| {
                    to_commit(item).map(|commit| commit.committer.name.as_str())
                });
            }
            SortKey::CommitterNameDesc => {
                items.sort_by_key(|item| {
                    cmp::Reverse(to_commit(item).map(|commit| commit.committer.name.as_str()))
                });
            }
            SortKey::CommitterEmail => {
                items.sort_by_key(|item| {
                    to_commit(item).map(|commit| commit.committer.email.as_str())
                });
            }
            SortKey::CommitterEmailDesc => {
                items.sort_by_key(|item| {
                    cmp::Reverse(to_commit(item).map(|commit| commit.committer.email.as_str()))
                });
            }
            SortKey::CommitterDate => {
                items.sort_by_key(|item| to_commit(item).map(|commit| commit.committer.timestamp));
            }
            SortKey::CommitterDateDesc => {
                items.sort_by_key(|item| {
                    cmp::Reverse(to_commit(item).map(|commit| commit.committer.timestamp))
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use jj_lib::backend::ChangeId;
    use jj_lib::backend::MillisSinceEpoch;
    use jj_lib::backend::Signature;
    use jj_lib::backend::Timestamp;
    use jj_lib::backend::TreeId;
    use jj_lib::merge::Merge;
    use jj_lib::op_store::RefTarget;

    use super::*;

    fn make_backend_commit(author: Signature, committer: Signature) -> Arc<backend::Commit> {
        Arc::new(backend::Commit {
            parents: vec![],
            predecessors: vec![],
            root_tree: Merge::resolved(TreeId::new(vec![])),
            conflict_labels: Merge::resolved(String::new()),
            change_id: ChangeId::new(vec![]),
            description: String::new(),
            author,
            committer,
            secure_sig: None,
        })
    }

    fn make_default_signature() -> Signature {
        Signature {
            name: "Test User".to_owned(),
            email: "test.user@g.com".to_owned(),
            timestamp: Timestamp {
                timestamp: MillisSinceEpoch(0),
                tz_offset: 0,
            },
        }
    }

    fn commit_id_generator() -> impl FnMut() -> CommitId {
        let mut iter = (1_u128..).map(|n| CommitId::new(n.to_le_bytes().into()));
        move || iter.next().unwrap()
    }

    fn commit_ts_generator() -> impl FnMut() -> Timestamp {
        // iter starts as 1, 1, 2, ... for test purposes
        let mut iter = Some(1_i64).into_iter().chain(1_i64..).map(|ms| Timestamp {
            timestamp: MillisSinceEpoch(ms),
            tz_offset: 0,
        });
        move || iter.next().unwrap()
    }

    // Helper function to prepare test data, sort and prepare snapshot with relevant
    // information.
    fn prepare_data_sort_and_snapshot(sort_keys: &[SortKey]) -> String {
        let mut new_commit_id = commit_id_generator();
        let mut new_timestamp = commit_ts_generator();
        let names = ["bob", "alice", "eve", "bob", "bob"];
        let emails = [
            "bob@g.com",
            "alice@g.com",
            "eve@g.com",
            "bob@g.com",
            "bob@g.com",
        ];
        let bookmark_names = ["feature", "bug-fix", "chore", "bug-fix", "feature"];
        let remote_names = [None, Some("upstream"), None, Some("origin"), Some("origin")];
        let deleted = [false, false, false, false, true];
        let mut bookmark_items: Vec<RefListItem> = Vec::new();
        let mut commits: HashMap<CommitId, Arc<backend::Commit>> = HashMap::new();
        for (&name, &email, bookmark_name, remote_name, &is_deleted) in
            itertools::izip!(&names, &emails, &bookmark_names, &remote_names, &deleted)
        {
            let commit_id = new_commit_id();
            let mut b_name = "foo";
            let mut author = make_default_signature();
            let mut committer = make_default_signature();

            if sort_keys.contains(&SortKey::Name) || sort_keys.contains(&SortKey::NameDesc) {
                b_name = bookmark_name;
            }
            if sort_keys.contains(&SortKey::AuthorName)
                || sort_keys.contains(&SortKey::AuthorNameDesc)
            {
                author.name = String::from(name);
            }
            if sort_keys.contains(&SortKey::AuthorEmail)
                || sort_keys.contains(&SortKey::AuthorEmailDesc)
            {
                author.email = String::from(email);
            }
            if sort_keys.contains(&SortKey::AuthorDate)
                || sort_keys.contains(&SortKey::AuthorDateDesc)
            {
                author.timestamp = new_timestamp();
            }
            if sort_keys.contains(&SortKey::CommitterName)
                || sort_keys.contains(&SortKey::CommitterNameDesc)
            {
                committer.name = String::from(name);
            }
            if sort_keys.contains(&SortKey::CommitterEmail)
                || sort_keys.contains(&SortKey::CommitterEmailDesc)
            {
                committer.email = String::from(email);
            }
            if sort_keys.contains(&SortKey::CommitterDate)
                || sort_keys.contains(&SortKey::CommitterDateDesc)
            {
                committer.timestamp = new_timestamp();
            }

            if let Some(remote_name) = remote_name {
                if is_deleted {
                    bookmark_items.push(RefListItem {
                        primary: CommitRef::remote_only(b_name, *remote_name, RefTarget::absent()),
                        tracked: vec![CommitRef::local_only(
                            b_name,
                            RefTarget::normal(commit_id.clone()),
                        )],
                    });
                } else {
                    bookmark_items.push(RefListItem {
                        primary: CommitRef::remote_only(
                            b_name,
                            *remote_name,
                            RefTarget::normal(commit_id.clone()),
                        ),
                        tracked: vec![],
                    });
                }
            } else {
                bookmark_items.push(RefListItem {
                    primary: CommitRef::local_only(b_name, RefTarget::normal(commit_id.clone())),
                    tracked: vec![],
                });
            }

            commits.insert(commit_id, make_backend_commit(author, committer));
        }

        // The sort function has an assumption that refs are sorted by name.
        // Here we support this assumption.
        bookmark_items.sort_by_key(|item| {
            (
                item.primary.name().to_owned(),
                item.primary.remote_name().map(|name| name.to_owned()),
            )
        });

        sort_and_snapshot(&mut bookmark_items, sort_keys, &commits)
    }

    // Helper function to sort refs and prepare snapshot with relevant information.
    fn sort_and_snapshot(
        items: &mut [RefListItem],
        sort_keys: &[SortKey],
        commits: &HashMap<CommitId, Arc<backend::Commit>>,
    ) -> String {
        sort_inner(items, sort_keys, commits);

        let to_commit = |item: &RefListItem| {
            let id = item.primary.target().added_ids().next()?;
            commits.get(id)
        };

        macro_rules! row_format {
            ($($args:tt)*) => {
                format!("{:<20}{:<16}{:<17}{:<14}{:<16}{:<17}{}", $($args)*)
            }
        }

        let header = row_format!(
            "Name",
            "AuthorName",
            "AuthorEmail",
            "AuthorDate",
            "CommitterName",
            "CommitterEmail",
            "CommitterDate"
        );

        let rows: Vec<String> = items
            .iter()
            .map(|item| {
                let name = [Some(item.primary.name()), item.primary.remote_name()]
                    .iter()
                    .flatten()
                    .join("@");

                let commit = to_commit(item);

                let author_name = commit
                    .map(|c| c.author.name.clone())
                    .unwrap_or_else(|| String::from("-"));
                let author_email = commit
                    .map(|c| c.author.email.clone())
                    .unwrap_or_else(|| String::from("-"));
                let author_date = commit
                    .map(|c| c.author.timestamp.timestamp.0.to_string())
                    .unwrap_or_else(|| String::from("-"));

                let committer_name = commit
                    .map(|c| c.committer.name.clone())
                    .unwrap_or_else(|| String::from("-"));
                let committer_email = commit
                    .map(|c| c.committer.email.clone())
                    .unwrap_or_else(|| String::from("-"));
                let committer_date = commit
                    .map(|c| c.committer.timestamp.timestamp.0.to_string())
                    .unwrap_or_else(|| String::from("-"));

                row_format!(
                    name,
                    author_name,
                    author_email,
                    author_date,
                    committer_name,
                    committer_email,
                    committer_date
                )
            })
            .collect();

        let mut result = vec![header];
        result.extend(rows);
        result.join("\n")
    }

    #[test]
    fn test_sort_by_name() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::Name]), @r"
        Name                AuthorName      AuthorEmail      AuthorDate    CommitterName   CommitterEmail   CommitterDate
        bug-fix@origin      Test User       test.user@g.com  0             Test User       test.user@g.com  0
        bug-fix@upstream    Test User       test.user@g.com  0             Test User       test.user@g.com  0
        chore               Test User       test.user@g.com  0             Test User       test.user@g.com  0
        feature             Test User       test.user@g.com  0             Test User       test.user@g.com  0
        feature@origin      -               -                -             -               -                -
        ");
    }

    #[test]
    fn test_sort_by_name_desc() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::NameDesc]), @r"
        Name                AuthorName      AuthorEmail      AuthorDate    CommitterName   CommitterEmail   CommitterDate
        feature@origin      -               -                -             -               -                -
        feature             Test User       test.user@g.com  0             Test User       test.user@g.com  0
        chore               Test User       test.user@g.com  0             Test User       test.user@g.com  0
        bug-fix@upstream    Test User       test.user@g.com  0             Test User       test.user@g.com  0
        bug-fix@origin      Test User       test.user@g.com  0             Test User       test.user@g.com  0
        ");
    }

    #[test]
    fn test_sort_by_author_name() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::AuthorName]), @r"
        Name                AuthorName      AuthorEmail      AuthorDate    CommitterName   CommitterEmail   CommitterDate
        foo@origin          -               -                -             -               -                -
        foo@upstream        alice           test.user@g.com  0             Test User       test.user@g.com  0
        foo                 bob             test.user@g.com  0             Test User       test.user@g.com  0
        foo@origin          bob             test.user@g.com  0             Test User       test.user@g.com  0
        foo                 eve             test.user@g.com  0             Test User       test.user@g.com  0
        ");
    }

    #[test]
    fn test_sort_by_author_name_desc() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::AuthorNameDesc]), @r"
        Name                AuthorName      AuthorEmail      AuthorDate    CommitterName   CommitterEmail   CommitterDate
        foo                 eve             test.user@g.com  0             Test User       test.user@g.com  0
        foo                 bob             test.user@g.com  0             Test User       test.user@g.com  0
        foo@origin          bob             test.user@g.com  0             Test User       test.user@g.com  0
        foo@upstream        alice           test.user@g.com  0             Test User       test.user@g.com  0
        foo@origin          -               -                -             -               -                -
        ");
    }

    #[test]
    fn test_sort_by_author_email() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::AuthorEmail]), @r"
        Name                AuthorName      AuthorEmail      AuthorDate    CommitterName   CommitterEmail   CommitterDate
        foo@origin          -               -                -             -               -                -
        foo@upstream        Test User       alice@g.com      0             Test User       test.user@g.com  0
        foo                 Test User       bob@g.com        0             Test User       test.user@g.com  0
        foo@origin          Test User       bob@g.com        0             Test User       test.user@g.com  0
        foo                 Test User       eve@g.com        0             Test User       test.user@g.com  0
        ");
    }

    #[test]
    fn test_sort_by_author_email_desc() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::AuthorEmailDesc]), @r"
        Name                AuthorName      AuthorEmail      AuthorDate    CommitterName   CommitterEmail   CommitterDate
        foo                 Test User       eve@g.com        0             Test User       test.user@g.com  0
        foo                 Test User       bob@g.com        0             Test User       test.user@g.com  0
        foo@origin          Test User       bob@g.com        0             Test User       test.user@g.com  0
        foo@upstream        Test User       alice@g.com      0             Test User       test.user@g.com  0
        foo@origin          -               -                -             -               -                -
        ");
    }

    #[test]
    fn test_sort_by_author_date() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::AuthorDate]), @r"
        Name                AuthorName      AuthorEmail      AuthorDate    CommitterName   CommitterEmail   CommitterDate
        foo@origin          -               -                -             -               -                -
        foo                 Test User       test.user@g.com  1             Test User       test.user@g.com  0
        foo@upstream        Test User       test.user@g.com  1             Test User       test.user@g.com  0
        foo                 Test User       test.user@g.com  2             Test User       test.user@g.com  0
        foo@origin          Test User       test.user@g.com  3             Test User       test.user@g.com  0
        ");
    }

    #[test]
    fn test_sort_by_author_date_desc() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::AuthorDateDesc]), @r"
        Name                AuthorName      AuthorEmail      AuthorDate    CommitterName   CommitterEmail   CommitterDate
        foo@origin          Test User       test.user@g.com  3             Test User       test.user@g.com  0
        foo                 Test User       test.user@g.com  2             Test User       test.user@g.com  0
        foo                 Test User       test.user@g.com  1             Test User       test.user@g.com  0
        foo@upstream        Test User       test.user@g.com  1             Test User       test.user@g.com  0
        foo@origin          -               -                -             -               -                -
        ");
    }

    #[test]
    fn test_sort_by_committer_name() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::CommitterName]), @r"
        Name                AuthorName      AuthorEmail      AuthorDate    CommitterName   CommitterEmail   CommitterDate
        foo@origin          -               -                -             -               -                -
        foo@upstream        Test User       test.user@g.com  0             alice           test.user@g.com  0
        foo                 Test User       test.user@g.com  0             bob             test.user@g.com  0
        foo@origin          Test User       test.user@g.com  0             bob             test.user@g.com  0
        foo                 Test User       test.user@g.com  0             eve             test.user@g.com  0
        ");
    }

    #[test]
    fn test_sort_by_committer_name_desc() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::CommitterNameDesc]), @r"
        Name                AuthorName      AuthorEmail      AuthorDate    CommitterName   CommitterEmail   CommitterDate
        foo                 Test User       test.user@g.com  0             eve             test.user@g.com  0
        foo                 Test User       test.user@g.com  0             bob             test.user@g.com  0
        foo@origin          Test User       test.user@g.com  0             bob             test.user@g.com  0
        foo@upstream        Test User       test.user@g.com  0             alice           test.user@g.com  0
        foo@origin          -               -                -             -               -                -
        ");
    }

    #[test]
    fn test_sort_by_committer_email() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::CommitterEmail]), @r"
        Name                AuthorName      AuthorEmail      AuthorDate    CommitterName   CommitterEmail   CommitterDate
        foo@origin          -               -                -             -               -                -
        foo@upstream        Test User       test.user@g.com  0             Test User       alice@g.com      0
        foo                 Test User       test.user@g.com  0             Test User       bob@g.com        0
        foo@origin          Test User       test.user@g.com  0             Test User       bob@g.com        0
        foo                 Test User       test.user@g.com  0             Test User       eve@g.com        0
        ");
    }

    #[test]
    fn test_sort_by_committer_email_desc() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::CommitterEmailDesc]), @r"
        Name                AuthorName      AuthorEmail      AuthorDate    CommitterName   CommitterEmail   CommitterDate
        foo                 Test User       test.user@g.com  0             Test User       eve@g.com        0
        foo                 Test User       test.user@g.com  0             Test User       bob@g.com        0
        foo@origin          Test User       test.user@g.com  0             Test User       bob@g.com        0
        foo@upstream        Test User       test.user@g.com  0             Test User       alice@g.com      0
        foo@origin          -               -                -             -               -                -
        ");
    }

    #[test]
    fn test_sort_by_committer_date() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::CommitterDate]), @r"
        Name                AuthorName      AuthorEmail      AuthorDate    CommitterName   CommitterEmail   CommitterDate
        foo@origin          -               -                -             -               -                -
        foo                 Test User       test.user@g.com  0             Test User       test.user@g.com  1
        foo@upstream        Test User       test.user@g.com  0             Test User       test.user@g.com  1
        foo                 Test User       test.user@g.com  0             Test User       test.user@g.com  2
        foo@origin          Test User       test.user@g.com  0             Test User       test.user@g.com  3
        ");
    }

    #[test]
    fn test_sort_by_committer_date_desc() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::CommitterDateDesc]), @r"
        Name                AuthorName      AuthorEmail      AuthorDate    CommitterName   CommitterEmail   CommitterDate
        foo@origin          Test User       test.user@g.com  0             Test User       test.user@g.com  3
        foo                 Test User       test.user@g.com  0             Test User       test.user@g.com  2
        foo                 Test User       test.user@g.com  0             Test User       test.user@g.com  1
        foo@upstream        Test User       test.user@g.com  0             Test User       test.user@g.com  1
        foo@origin          -               -                -             -               -                -
        ");
    }

    #[test]
    fn test_sort_by_author_date_desc_and_name() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::AuthorDateDesc, SortKey::Name]), @r"
        Name                AuthorName      AuthorEmail      AuthorDate    CommitterName   CommitterEmail   CommitterDate
        bug-fix@origin      Test User       test.user@g.com  3             Test User       test.user@g.com  0
        chore               Test User       test.user@g.com  2             Test User       test.user@g.com  0
        bug-fix@upstream    Test User       test.user@g.com  1             Test User       test.user@g.com  0
        feature             Test User       test.user@g.com  1             Test User       test.user@g.com  0
        feature@origin      -               -                -             -               -                -
        ");
    }

    #[test]
    fn test_sort_by_committer_name_and_name_desc() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::CommitterName, SortKey::NameDesc]), @r"
        Name                AuthorName      AuthorEmail      AuthorDate    CommitterName   CommitterEmail   CommitterDate
        feature@origin      -               -                -             -               -                -
        bug-fix@upstream    Test User       test.user@g.com  0             alice           test.user@g.com  0
        feature             Test User       test.user@g.com  0             bob             test.user@g.com  0
        bug-fix@origin      Test User       test.user@g.com  0             bob             test.user@g.com  0
        chore               Test User       test.user@g.com  0             eve             test.user@g.com  0
        ");
    }

    // Bookmarks are already sorted by name
    // Test when sorting by name is not the only/last criteria
    #[test]
    fn test_sort_by_name_and_committer_date() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::Name, SortKey::AuthorDate]), @r"
        Name                AuthorName      AuthorEmail      AuthorDate    CommitterName   CommitterEmail   CommitterDate
        bug-fix@origin      Test User       test.user@g.com  3             Test User       test.user@g.com  0
        bug-fix@upstream    Test User       test.user@g.com  1             Test User       test.user@g.com  0
        chore               Test User       test.user@g.com  2             Test User       test.user@g.com  0
        feature             Test User       test.user@g.com  1             Test User       test.user@g.com  0
        feature@origin      -               -                -             -               -                -
        ");
    }
}
