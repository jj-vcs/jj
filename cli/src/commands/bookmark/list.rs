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
use jj_lib::config::ConfigValue;
use jj_lib::ref_name::RefName;
use jj_lib::repo::Repo as _;
use jj_lib::revset::RevsetExpression;
use jj_lib::str_util::StringExpression;
use jj_lib::str_util::StringPattern;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::default_ignored_remote_name;
use crate::command_error::CommandError;
use crate::commit_templater::CommitRef;
use crate::complete;
use crate::templater::TemplateRenderer;
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
    ///
    /// This defaults to the `ui.bookmark-list-sort-keys` setting.
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
                            .any(|pattern| pattern.is_match(name.as_str()))
                    })
                    .map(|(name, _)| name),
            );
        }
        if let Some(revisions) = &args.revisions {
            // Match against local targets only, which is consistent with "jj git push".
            let mut expression = workspace_command.parse_union_revsets(ui, revisions)?;
            // Intersects with the set of local bookmark targets to minimize the lookup
            // space.
            expression.intersect_with(&RevsetExpression::bookmarks(StringExpression::all()));
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

    let template: TemplateRenderer<Rc<CommitRef>> = {
        let language = workspace_command.commit_template_language();
        let text = match &args.template {
            Some(value) => value.to_owned(),
            None => workspace_command
                .settings()
                .get("templates.bookmark_list")?,
        };
        workspace_command
            .parse_template(ui, &language, &text)?
            .labeled(["bookmark_list"])
    };

    let ignored_tracked_remote = default_ignored_remote_name(repo.store());
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
        let (mut tracked_remote_refs, untracked_remote_refs) = remote_refs
            .iter()
            .copied()
            .filter(|(remote_name, _)| {
                args.remotes.as_ref().is_none_or(|patterns| {
                    patterns
                        .iter()
                        .any(|pattern| pattern.is_match(remote_name.as_str()))
                })
            })
            .partition::<Vec<_>, _>(|&(_, remote_ref)| remote_ref.is_tracked());

        if args.tracked {
            tracked_remote_refs.retain(|&(remote, _)| {
                ignored_tracked_remote.is_none_or(|ignored| remote != ignored)
            });
        } else if !args.all_remotes && args.remotes.is_none() {
            tracked_remote_refs.retain(|&(_, remote_ref)| remote_ref.target != *local_target);
        }

        let include_local_only = !args.tracked && args.remotes.is_none();
        if include_local_only && local_target.is_present() || !tracked_remote_refs.is_empty() {
            let primary = CommitRef::local(
                name,
                local_target.clone(),
                remote_refs.iter().map(|&(_, remote_ref)| remote_ref),
            );
            let tracked = tracked_remote_refs
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

    let sort_keys = if args.sort.is_empty() {
        workspace_command
            .settings()
            .get_value_with("ui.bookmark-list-sort-keys", parse_sort_keys)?
    } else {
        args.sort.clone()
    };
    let store = repo.store();
    let mut commits: HashMap<CommitId, Arc<backend::Commit>> = HashMap::new();
    if sort_keys.iter().any(|key| key.is_commit_dependant()) {
        commits = bookmark_list_items
            .iter()
            .filter_map(|item| item.primary.target().added_ids().next())
            .map(|commit_id| {
                store
                    .get_commit(commit_id)
                    .map(|commit| (commit_id.clone(), commit.store_commit().clone()))
            })
            .try_collect()?;
    }
    sort(&mut bookmark_list_items, &sort_keys, &commits);

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
                    ignored_tracked_remote.is_none_or(|ignored| remote != ignored)
                })
            })
            .max();
        match deleted_tracking {
            Some(true) => {
                writeln!(
                    ui.hint_default(),
                    "Bookmarks marked as deleted can be *deleted permanently* on the remote by \
                     running `jj git push --deleted`. Use `jj bookmark forget` if you don't want \
                     that."
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

fn parse_sort_keys(value: ConfigValue) -> Result<Vec<SortKey>, String> {
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
    use jj_lib::backend::ChangeId;
    use jj_lib::backend::MergedTreeId;
    use jj_lib::backend::MillisSinceEpoch;
    use jj_lib::backend::Signature;
    use jj_lib::backend::Timestamp;
    use jj_lib::backend::TreeId;
    use jj_lib::op_store::RefTarget;

    use super::*;

    fn make_backend_commit(author: Signature, committer: Signature) -> Arc<backend::Commit> {
        Arc::new(backend::Commit {
            parents: vec![],
            predecessors: vec![],
            root_tree: MergedTreeId::resolved(TreeId::new(vec![])),
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
        sort(items, sort_keys, commits);

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
}
