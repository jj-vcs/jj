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

use std::cmp;
use std::collections::HashMap;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use clap::ValueEnum;
use clap_complete::ArgValueCandidates;
use itertools::Itertools as _;
use jj_lib::backend;
use jj_lib::backend::CommitId;
use jj_lib::ref_name::RefName;
use jj_lib::repo::Repo as _;
use jj_lib::revset::RevsetExpression;
use jj_lib::str_util::StringPattern;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::commit_templater::CommitRef;
use crate::commit_templater::CommitTemplateLanguage;
use crate::complete;
use crate::ui::Ui;

/// List bookmarks and their targets
///
/// By default, a tracking remote bookmark will be included only if its target
/// is different from the local target. A non-tracking remote bookmark won't be
/// listed. For a conflicted bookmark (both local and remote), old target
/// revisions are preceded by a "-" and new target revisions are preceded by a
/// "+".
///
/// See [`jj help -k bookmarks`] for more information.
///
/// [`jj help -k bookmarks`]:
///     https://jj-vcs.github.io/jj/latest/bookmarks
#[derive(clap::Args, Clone, Debug)]
pub struct BookmarkListArgs {
    /// Show all tracking and non-tracking remote bookmarks including the ones
    /// whose targets are synchronized with the local bookmarks
    #[arg(long, short, alias = "all")]
    all_remotes: bool,

    /// Show all tracking and non-tracking remote bookmarks belonging
    /// to this remote
    ///
    /// Can be combined with `--tracked` or `--conflicted` to filter the
    /// bookmarks shown (can be repeated.)
    ///
    /// By default, the specified remote name matches exactly. Use `glob:`
    /// prefix to select remotes by [wildcard pattern].
    ///
    /// [wildcard pattern]:
    ///     https://jj-vcs.github.io/jj/latest/revsets/#string-patterns
    #[arg(
        long = "remote",
        value_name = "REMOTE",
        conflicts_with_all = ["all_remotes"],
        value_parser = StringPattern::parse,
        add = ArgValueCandidates::new(complete::git_remotes),
    )]
    remotes: Option<Vec<StringPattern>>,

    /// Show remote tracked bookmarks only. Omits local Git-tracking bookmarks
    /// by default
    #[arg(long, short, conflicts_with_all = ["all_remotes"])]
    tracked: bool,

    /// Show conflicted bookmarks only
    #[arg(long, short, conflicts_with_all = ["all_remotes"])]
    conflicted: bool,

    /// Show bookmarks whose local name matches
    ///
    /// By default, the specified name matches exactly. Use `glob:` prefix to
    /// select bookmarks by [wildcard pattern].
    ///
    /// [wildcard pattern]:
    ///     https://jj-vcs.github.io/jj/latest/revsets/#string-patterns
    #[arg(value_parser = StringPattern::parse, add = ArgValueCandidates::new(complete::bookmarks))]
    names: Option<Vec<StringPattern>>,

    /// Show bookmarks whose local targets are in the given revisions
    ///
    /// Note that `-r deleted_bookmark` will not work since `deleted_bookmark`
    /// wouldn't have a local target.
    #[arg(long, short, value_name = "REVSETS")]
    revisions: Option<Vec<RevisionArg>>,

    /// Render each bookmark using the given template
    ///
    /// All 0-argument methods of the [`CommitRef` type] are available as
    /// keywords in the template expression. See [`jj help -k templates`]
    /// for more information.
    ///
    /// [`CommitRef` type]:
    ///     https://jj-vcs.github.io/jj/latest/templates/#commitref-type
    ///
    /// [`jj help -k templates`]:
    ///     https://jj-vcs.github.io/jj/latest/templates/
    #[arg(long, short = 'T', add = ArgValueCandidates::new(complete::template_aliases))]
    template: Option<String>,

    /// Sort bookmarks based on the given key (or multiple keys)
    ///
    /// Suffix the key with `-` to sort in descending order of the value (e.g.
    /// `--sort name-`). Note that when using multiple keys, the first key is
    /// the most significant.
    #[arg(long, value_name = "SORT_KEY", value_enum, value_delimiter = ',')]
    sort: Vec<SortKey>,
}

pub fn cmd_bookmark_list(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &BookmarkListArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let repo = workspace_command.repo();
    let view = repo.view();

    // Like cmd_git_push(), names and revisions are OR-ed.
    let bookmark_names_to_list = if args.names.is_some() || args.revisions.is_some() {
        let mut bookmark_names: HashSet<&RefName> = HashSet::new();
        if let Some(patterns) = &args.names {
            bookmark_names.extend(
                view.bookmarks()
                    .filter(|(name, _)| {
                        patterns
                            .iter()
                            .any(|pattern| pattern.matches(name.as_str()))
                    })
                    .map(|(name, _)| name),
            );
        }
        if let Some(revisions) = &args.revisions {
            // Match against local targets only, which is consistent with "jj git push".
            let mut expression = workspace_command.parse_union_revsets(ui, revisions)?;
            // Intersects with the set of local bookmark targets to minimize the lookup
            // space.
            expression.intersect_with(&RevsetExpression::bookmarks(StringPattern::everything()));
            let filtered_targets: HashSet<_> =
                expression.evaluate_to_commit_ids()?.try_collect()?;
            bookmark_names.extend(
                view.local_bookmarks()
                    .filter(|(_, target)| {
                        target.added_ids().any(|id| filtered_targets.contains(id))
                    })
                    .map(|(name, _)| name),
            );
        }
        Some(bookmark_names)
    } else {
        None
    };

    let template = {
        let language = workspace_command.commit_template_language();
        let text = match &args.template {
            Some(value) => value.to_owned(),
            None => workspace_command
                .settings()
                .get("templates.bookmark_list")?,
        };
        workspace_command
            .parse_template(
                ui,
                &language,
                &text,
                CommitTemplateLanguage::wrap_commit_ref,
            )?
            .labeled("bookmark_list")
    };

    let mut bookmark_list_items: Vec<RefListItem> = Vec::new();
    let bookmarks_to_list = view.bookmarks().filter(|(name, target)| {
        bookmark_names_to_list
            .as_ref()
            .is_none_or(|bookmark_names| bookmark_names.contains(name))
            && (!args.conflicted || target.local_target.has_conflict())
    });
    for (name, bookmark_target) in bookmarks_to_list {
        let local_target = bookmark_target.local_target;
        let remote_refs = bookmark_target.remote_refs;
        let (mut tracking_remote_refs, untracked_remote_refs) = remote_refs
            .iter()
            .copied()
            .filter(|(remote_name, _)| {
                args.remotes.as_ref().is_none_or(|patterns| {
                    patterns
                        .iter()
                        .any(|pattern| pattern.matches(remote_name.as_str()))
                })
            })
            .partition::<Vec<_>, _>(|&(_, remote_ref)| remote_ref.is_tracking());

        if args.tracked {
            tracking_remote_refs.retain(|&(remote, _)| !jj_lib::git::is_special_git_remote(remote));
        } else if !args.all_remotes && args.remotes.is_none() {
            tracking_remote_refs.retain(|&(_, remote_ref)| remote_ref.target != *local_target);
        }

        let include_local_only = !args.tracked && args.remotes.is_none();
        if include_local_only && local_target.is_present() || !tracking_remote_refs.is_empty() {
            let primary = CommitRef::local(
                name,
                local_target.clone(),
                remote_refs.iter().map(|&(_, remote_ref)| remote_ref),
            );
            let tracked = tracking_remote_refs
                .iter()
                .map(|&(remote, remote_ref)| {
                    CommitRef::remote(name, remote, remote_ref.clone(), local_target)
                })
                .collect();
            bookmark_list_items.push(RefListItem { primary, tracked });
        }

        if !args.tracked && (args.all_remotes || args.remotes.is_some()) {
            bookmark_list_items.extend(untracked_remote_refs.iter().map(
                |&(remote, remote_ref)| RefListItem {
                    primary: CommitRef::remote_only(name, remote, remote_ref.target.clone()),
                    tracked: vec![],
                },
            ));
        }
    }

    let store = repo.store();
    let mut commits: HashMap<CommitId, Arc<backend::Commit>> = HashMap::new();
    if args.sort.iter().any(|key| key.is_commit_dependant()) {
        commits = bookmark_list_items
            .iter()
            .filter_map(|item| item.primary.target().added_ids().next())
            .cloned()
            .map(|commit_id| {
                store
                    .get_commit(&commit_id)
                    .map(|commit| (commit_id, commit.store_commit().clone()))
            })
            .try_collect()?;
    }
    sort(&mut bookmark_list_items, &args.sort, &commits);

    ui.request_pager();
    let mut formatter = ui.stdout_formatter();
    bookmark_list_items
        .iter()
        .flat_map(|item| itertools::chain([&item.primary], &item.tracked))
        .try_for_each(|commit_ref| template.format(commit_ref, formatter.as_mut()))?;
    drop(formatter);

    #[cfg(feature = "git")]
    if jj_lib::git::get_git_backend(repo.store()).is_ok() {
        // Print only one of these hints. It's not important to mention unexported
        // bookmarks, but user might wonder why deleted bookmarks are still listed.
        let deleted_tracking = bookmark_list_items
            .iter()
            .filter(|item| item.primary.is_local() && item.primary.is_absent())
            .map(|item| {
                item.tracked.iter().any(|r| {
                    let remote = r.remote_name().expect("tracked ref should be remote");
                    !jj_lib::git::is_special_git_remote(remote.as_ref())
                })
            })
            .max();
        match deleted_tracking {
            Some(true) => {
                writeln!(
                    ui.hint_default(),
                    "Bookmarks marked as deleted will be *deleted permanently* on the remote on \
                     the next `jj git push`. Use `jj bookmark forget` to prevent this."
                )?;
            }
            Some(false) => {
                writeln!(
                    ui.hint_default(),
                    "Bookmarks marked as deleted will be deleted from the underlying Git repo on \
                     the next `jj git export`."
                )?;
            }
            None => {}
        }
    }

    Ok(())
}

#[derive(Clone, Debug)]
struct RefListItem {
    /// Local bookmark or untracked remote bookmark.
    primary: Rc<CommitRef>,
    /// Remote bookmarks tracked by the primary (or local) bookmark.
    tracked: Vec<Rc<CommitRef>>,
}

/// Sort key for the `--sort` argument option.
#[derive(Copy, Clone, PartialEq, Debug, ValueEnum)]
enum SortKey {
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
            SortKey::Name | SortKey::NameDesc => false,
            SortKey::AuthorName
            | SortKey::AuthorNameDesc
            | SortKey::AuthorEmail
            | SortKey::AuthorEmailDesc
            | SortKey::AuthorDate
            | SortKey::AuthorDateDesc
            | SortKey::CommitterName
            | SortKey::CommitterNameDesc
            | SortKey::CommitterEmail
            | SortKey::CommitterEmailDesc
            | SortKey::CommitterDate
            | SortKey::CommitterDateDesc => true,
        }
    }
}

fn sort(
    bookmark_items: &mut [RefListItem],
    sort_keys: &[SortKey],
    commits: &HashMap<CommitId, Arc<backend::Commit>>,
) {
    let to_commit = |item: &RefListItem| {
        let id = item.primary.target().added_ids().next()?;
        commits.get(id)
    };

    // Multi-pass sorting, the first key is most significant.
    // Skip first iteration if sort key is `Name`, since bookmarks are already
    // sorted by name.
    for sort_key in sort_keys
        .iter()
        .rev()
        .skip_while(|key| *key == &SortKey::Name)
    {
        match sort_key {
            SortKey::Name => {
                bookmark_items.sort_by_key(|item| {
                    (
                        item.primary.name().to_owned(),
                        item.primary.remote_name().map(|name| name.to_owned()),
                    )
                });
            }
            SortKey::NameDesc => {
                bookmark_items.sort_by_key(|item| {
                    cmp::Reverse((
                        item.primary.name().to_owned(),
                        item.primary.remote_name().map(|name| name.to_owned()),
                    ))
                });
            }
            SortKey::AuthorName => bookmark_items
                .sort_by_key(|item| to_commit(item).map(|commit| commit.author.name.as_str())),
            SortKey::AuthorNameDesc => bookmark_items.sort_by_key(|item| {
                cmp::Reverse(to_commit(item).map(|commit| commit.author.name.as_str()))
            }),
            SortKey::AuthorEmail => bookmark_items
                .sort_by_key(|item| to_commit(item).map(|commit| commit.author.email.as_str())),
            SortKey::AuthorEmailDesc => bookmark_items.sort_by_key(|item| {
                cmp::Reverse(to_commit(item).map(|commit| commit.author.email.as_str()))
            }),
            SortKey::AuthorDate => bookmark_items
                .sort_by_key(|item| to_commit(item).map(|commit| commit.author.timestamp)),
            SortKey::AuthorDateDesc => bookmark_items.sort_by_key(|item| {
                cmp::Reverse(to_commit(item).map(|commit| commit.author.timestamp))
            }),
            SortKey::CommitterName => bookmark_items
                .sort_by_key(|item| to_commit(item).map(|commit| commit.committer.name.as_str())),
            SortKey::CommitterNameDesc => bookmark_items.sort_by_key(|item| {
                cmp::Reverse(to_commit(item).map(|commit| commit.committer.name.as_str()))
            }),
            SortKey::CommitterEmail => bookmark_items
                .sort_by_key(|item| to_commit(item).map(|commit| commit.committer.email.as_str())),
            SortKey::CommitterEmailDesc => bookmark_items.sort_by_key(|item| {
                cmp::Reverse(to_commit(item).map(|commit| commit.committer.email.as_str()))
            }),
            SortKey::CommitterDate => bookmark_items
                .sort_by_key(|item| to_commit(item).map(|commit| commit.committer.timestamp)),
            SortKey::CommitterDateDesc => bookmark_items.sort_by_key(|item| {
                cmp::Reverse(to_commit(item).map(|commit| commit.committer.timestamp))
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use backend::ChangeId;
    use backend::MergedTreeId;
    use backend::TreeId;
    use jj_lib::backend::MillisSinceEpoch;
    use jj_lib::backend::Signature;
    use jj_lib::backend::Timestamp;
    use jj_lib::op_store::RefTarget;
    use jj_lib::op_store::RemoteRef;
    use jj_lib::op_store::RemoteRefState;

    use super::*;

    fn make_backend_commit(author: Signature, committer: Signature) -> Arc<backend::Commit> {
        Arc::new(backend::Commit {
            parents: vec![],
            predecessors: vec![],
            root_tree: MergedTreeId::Legacy(TreeId::new(vec![])),
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
            email: "test.user@example.com".to_owned(),
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
        let names = ["bob", "alice", "eve", "bob"];
        let emails = ["bob@g.com", "alice@g.com", "eve@g.com", "bob@g.com"];
        let bookmark_names = ["feature", "bug-fix", "chore", "bug-fix"];
        let remote_names = [None, Some("upstream"), None, Some("origin")];
        let mut bookmark_items: Vec<RefListItem> = Vec::new();
        let mut commits: HashMap<CommitId, Arc<backend::Commit>> = HashMap::new();
        for (((&name, &email), bookmark_name), remote_name) in names
            .iter()
            .zip(emails.iter())
            .zip(bookmark_names.iter())
            .zip(remote_names.iter())
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
                let local_target = RefTarget::normal(commit_id.clone());
                let remote_ref = RemoteRef {
                    target: local_target.clone(),
                    state: RemoteRefState::New,
                };
                bookmark_items.push(RefListItem {
                    primary: CommitRef::remote(b_name, *remote_name, remote_ref, &local_target),
                    tracked: vec![],
                });
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
        sort(items, sort_keys, commits);

        let to_commit = |item: &RefListItem| {
            let id = item.primary.target().added_ids().next()?;
            commits.get(id)
        };
        items
            .iter()
            .map(|item| {
                sort_keys
                    .iter()
                    .map(|key| match key {
                        SortKey::Name | SortKey::NameDesc => {
                            [Some(item.primary.name()), item.primary.remote_name()]
                                .iter()
                                .flatten()
                                .join("@")
                        }
                        SortKey::AuthorName | SortKey::AuthorNameDesc => to_commit(item)
                            .map(|commit| format!("author: {}", commit.author.name))
                            .unwrap_or_default(),
                        SortKey::AuthorEmail | SortKey::AuthorEmailDesc => to_commit(item)
                            .map(|commit| format!("author.email: {}", commit.author.email))
                            .unwrap_or_default(),
                        SortKey::AuthorDate | SortKey::AuthorDateDesc => to_commit(item)
                            .map(|commit| {
                                format!("author.timestamp: {}", commit.author.timestamp.timestamp.0)
                            })
                            .unwrap_or_default(),
                        SortKey::CommitterName | SortKey::CommitterNameDesc => to_commit(item)
                            .map(|commit| format!("committer: {}", commit.committer.name))
                            .unwrap_or_default(),
                        SortKey::CommitterEmail | SortKey::CommitterEmailDesc => to_commit(item)
                            .map(|commit| format!("committer.email: {}", commit.committer.email))
                            .unwrap_or_default(),
                        SortKey::CommitterDate | SortKey::CommitterDateDesc => to_commit(item)
                            .map(|commit| {
                                format!(
                                    "committer.timestamp: {}",
                                    commit.committer.timestamp.timestamp.0
                                )
                            })
                            .unwrap_or_default(),
                    })
                    .join(", ")
            })
            .join("\n")
    }

    #[test]
    fn test_sort_by_name() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::Name]), @r"
        bug-fix@origin
        bug-fix@upstream
        chore
        feature
        ");
    }

    #[test]
    fn test_sort_by_name_desc() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::NameDesc]), @r"
        feature
        chore
        bug-fix@upstream
        bug-fix@origin
        ");
    }

    #[test]
    fn test_sort_by_author_name() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::AuthorName]), @r"
        author: alice
        author: bob
        author: bob
        author: eve
        ");
    }

    #[test]
    fn test_sort_by_author_name_desc() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::AuthorNameDesc]), @r"
        author: eve
        author: bob
        author: bob
        author: alice
        ");
    }

    #[test]
    fn test_sort_by_author_email() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::AuthorEmail]), @r"
        author.email: alice@g.com
        author.email: bob@g.com
        author.email: bob@g.com
        author.email: eve@g.com
        ");
    }

    #[test]
    fn test_sort_by_author_email_desc() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::AuthorEmailDesc]), @r"
        author.email: eve@g.com
        author.email: bob@g.com
        author.email: bob@g.com
        author.email: alice@g.com
        ");
    }

    #[test]
    fn test_sort_by_author_date() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::AuthorDate]), @r"
        author.timestamp: 1
        author.timestamp: 1
        author.timestamp: 2
        author.timestamp: 3
        ");
    }

    #[test]
    fn test_sort_by_author_date_desc() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::AuthorDateDesc]), @r"
        author.timestamp: 3
        author.timestamp: 2
        author.timestamp: 1
        author.timestamp: 1
        ");
    }

    #[test]
    fn test_sort_by_committer_name() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::CommitterName]), @r"
        committer: alice
        committer: bob
        committer: bob
        committer: eve
        ");
    }

    #[test]
    fn test_sort_by_committer_name_desc() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::CommitterNameDesc]), @r"
        committer: eve
        committer: bob
        committer: bob
        committer: alice
        ");
    }

    #[test]
    fn test_sort_by_committer_email() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::CommitterEmail]), @r"
        committer.email: alice@g.com
        committer.email: bob@g.com
        committer.email: bob@g.com
        committer.email: eve@g.com
        ");
    }

    #[test]
    fn test_sort_by_committer_email_desc() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::CommitterEmailDesc]), @r"
        committer.email: eve@g.com
        committer.email: bob@g.com
        committer.email: bob@g.com
        committer.email: alice@g.com
        ");
    }

    #[test]
    fn test_sort_by_committer_date() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::CommitterDate]), @r"
        committer.timestamp: 1
        committer.timestamp: 1
        committer.timestamp: 2
        committer.timestamp: 3
        ");
    }

    #[test]
    fn test_sort_by_committer_date_desc() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::CommitterDateDesc]), @r"
        committer.timestamp: 3
        committer.timestamp: 2
        committer.timestamp: 1
        committer.timestamp: 1
        ");
    }

    #[test]
    fn test_sort_by_author_date_desc_and_name() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::AuthorDateDesc, SortKey::Name]), @r"
        author.timestamp: 3, bug-fix@origin
        author.timestamp: 2, chore
        author.timestamp: 1, bug-fix@upstream
        author.timestamp: 1, feature
        ");
    }

    #[test]
    fn test_sort_by_committer_name_and_name_desc() {
        insta::assert_snapshot!(
            prepare_data_sort_and_snapshot(&[SortKey::CommitterName, SortKey::NameDesc]), @r"
        committer: alice, bug-fix@upstream
        committer: bob, feature
        committer: bob, bug-fix@origin
        committer: eve, chore
        ");
    }
}
