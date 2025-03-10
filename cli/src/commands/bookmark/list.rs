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
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use clap::ValueEnum;
use clap_complete::ArgValueCandidates;
use itertools::Itertools;
use jj_lib::backend::Timestamp;
use jj_lib::op_store::BookmarkTarget;
use jj_lib::repo::Repo;
use jj_lib::revset::RevsetExpression;
use jj_lib::store::Store;
use jj_lib::str_util::StringPattern;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::command_error::CommandError;
use crate::commit_templater::CommitTemplateLanguage;
use crate::commit_templater::RefName;
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
    /// All 0-argument methods of the [`RefName` type] are available as keywords
    /// in the template expression. See [`jj help -k templates`] for more
    /// information.
    ///
    /// [`RefName` type]:
    ///     https://jj-vcs.github.io/jj/latest/templates/#refname-type
    ///
    /// [`jj help -k templates`]:
    ///     https://jj-vcs.github.io/jj/latest/templates/
    #[arg(long, short = 'T', add = ArgValueCandidates::new(complete::template_aliases))]
    template: Option<String>,

    /// Sort bookmarks based on the given key (or multiple keys). Suffix the key
    /// with `-` to sort in descending order of the value (e.g. `--sort name-`).
    ///
    /// Note that when using multiple keys, the first key is the most significant.
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
        let mut bookmark_names: HashSet<&str> = HashSet::new();
        if let Some(patterns) = &args.names {
            bookmark_names.extend(
                view.bookmarks()
                    .filter(|&(name, _)| patterns.iter().any(|pattern| pattern.matches(name)))
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
            .parse_template(ui, &language, &text, CommitTemplateLanguage::wrap_ref_name)?
            .labeled("bookmark_list")
    };

    let mut bookmarks_to_list = view
        .bookmarks()
        .filter(|(name, target)| {
            bookmark_names_to_list
                .as_ref()
                .is_none_or(|bookmark_names| bookmark_names.contains(name))
                && (!args.conflicted || target.local_target.has_conflict())
        })
        .collect_vec();

    // Multi-pass sorting, the first key is most significant.
    for (i, sort_key) in args.sort.iter().rev().enumerate() {
        let is_reversed = sort_key.is_reversed();
        match SortStrategy::from(*sort_key) {
            SortStrategy::Name => {
                if is_reversed {
                    bookmarks_to_list.sort_by_key(|&(name, _)| cmp::Reverse(name));
                } else {
                    // Bookmarks are already sorted by name.
                    let is_noop = i == 0 && sort_key == &SortKey::Name;
                    if !is_noop {
                        bookmarks_to_list.sort_by_key(|&(name, _)| name);
                    }
                }
            }
            SortStrategy::Commit { key } => {
                let store = repo.store();
                if is_reversed {
                    bookmarks_to_list.sort_by_key(|(_, target)| {
                        cmp::Reverse(ord_value_for_bookmark_target(target, store, key))
                    });
                } else {
                    bookmarks_to_list.sort_by_key(|(_, target)| {
                        ord_value_for_bookmark_target(target, store, key)
                    });
                }
            }
        }
    }

    let mut bookmark_list_items: Vec<RefListItem> = Vec::new();
    for (name, bookmark_target) in bookmarks_to_list {
        let local_target = bookmark_target.local_target;
        let remote_refs = bookmark_target.remote_refs;
        let (mut tracking_remote_refs, untracked_remote_refs) = remote_refs
            .iter()
            .copied()
            .filter(|&(remote_name, _)| {
                args.remotes.as_ref().is_none_or(|patterns| {
                    patterns.iter().any(|pattern| pattern.matches(remote_name))
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
            let primary = RefName::local(
                name,
                local_target.clone(),
                remote_refs.iter().map(|&(_, remote_ref)| remote_ref),
            );
            let tracked = tracking_remote_refs
                .iter()
                .map(|&(remote, remote_ref)| {
                    RefName::remote(name, remote, remote_ref.clone(), local_target)
                })
                .collect();
            bookmark_list_items.push(RefListItem { primary, tracked });
        }

        if !args.tracked && (args.all_remotes || args.remotes.is_some()) {
            bookmark_list_items.extend(untracked_remote_refs.iter().map(
                |&(remote, remote_ref)| RefListItem {
                    primary: RefName::remote_only(name, remote, remote_ref.target.clone()),
                    tracked: vec![],
                },
            ));
        }
    }

    ui.request_pager();
    let mut formatter = ui.stdout_formatter();
    bookmark_list_items
        .iter()
        .flat_map(|item| itertools::chain([&item.primary], &item.tracked))
        .try_for_each(|ref_name| template.format(ref_name, formatter.as_mut()))?;
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
                    !jj_lib::git::is_special_git_remote(remote)
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
    primary: Rc<RefName>,
    /// Remote bookmarks tracked by the primary (or local) bookmark.
    tracked: Vec<Rc<RefName>>,
}

/// Sort key for the `--sort` argument option.
#[derive(Copy, Clone, PartialEq, Debug, ValueEnum)]
enum SortKey {
    Name,
    #[value(name = "name-")]
    NameReversed,
    AuthorName,
    #[value(name = "author-name-")]
    AuthorNameReversed,
    AuthorEmail,
    #[value(name = "author-email-")]
    AuthorEmailReversed,
    AuthorDate,
    #[value(name = "author-date-")]
    AuthorDateReversed,
    CommitterName,
    #[value(name = "committer-name-")]
    CommitterNameReversed,
    CommitterEmail,
    #[value(name = "committer-email-")]
    CommitterEmailReversed,
    CommitterDate,
    #[value(name = "committer-date-")]
    CommitterDateReversed,
}

impl SortKey {
    fn is_reversed(&self) -> bool {
        match self {
            SortKey::Name
            | SortKey::AuthorName
            | SortKey::AuthorEmail
            | SortKey::AuthorDate
            | SortKey::CommitterName
            | SortKey::CommitterEmail
            | SortKey::CommitterDate => false,
            SortKey::NameReversed
            | SortKey::AuthorNameReversed
            | SortKey::AuthorEmailReversed
            | SortKey::AuthorDateReversed
            | SortKey::CommitterNameReversed
            | SortKey::CommitterEmailReversed
            | SortKey::CommitterDateReversed => true,
        }
    }
}

/// Convenience type that helps separate sorting algorithms.
#[derive(Copy, Clone, Debug)]
enum SortStrategy {
    /// Sort strategy based on a bookmark's name.
    Name,
    /// Sort strategy based on a commit that relates to a bookmark.
    Commit { key: CommitSortKey },
}

/// Sort key to sort bookmarks based on a commit.
#[derive(Copy, Clone, Debug)]
enum CommitSortKey {
    AuthorName,
    AuthorEmail,
    AuthorDate,
    CommitterName,
    CommitterEmail,
    CommitterDate,
}

impl From<SortKey> for SortStrategy {
    fn from(value: SortKey) -> Self {
        match value {
            SortKey::Name | SortKey::NameReversed => Self::Name,
            SortKey::AuthorName | SortKey::AuthorNameReversed => Self::Commit {
                key: CommitSortKey::AuthorName,
            },
            SortKey::AuthorEmail | SortKey::AuthorEmailReversed => Self::Commit {
                key: CommitSortKey::AuthorEmail,
            },
            SortKey::AuthorDate | SortKey::AuthorDateReversed => Self::Commit {
                key: CommitSortKey::AuthorDate,
            },
            SortKey::CommitterName | SortKey::CommitterNameReversed => Self::Commit {
                key: CommitSortKey::CommitterName,
            },
            SortKey::CommitterEmail | SortKey::CommitterEmailReversed => Self::Commit {
                key: CommitSortKey::CommitterEmail,
            },
            SortKey::CommitterDate | SortKey::CommitterDateReversed => Self::Commit {
                key: CommitSortKey::CommitterDate,
            },
        }
    }
}

/// Wrapper type to unify types for Ord.
#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
enum CommitOrdValue {
    Str(String),
    Timestamp(Timestamp),
}

fn ord_value_for_bookmark_target(
    bookmark: &BookmarkTarget<'_>,
    store: &Arc<Store>,
    key: CommitSortKey,
) -> Option<CommitOrdValue> {
    bookmark
        .local_target
        .added_ids()
        .next()
        .and_then(|id| store.get_commit(id).ok())
        .map(|commit| match key {
            CommitSortKey::AuthorName => CommitOrdValue::Str(commit.author().name.clone()),
            CommitSortKey::AuthorEmail => CommitOrdValue::Str(commit.author().email.clone()),
            CommitSortKey::AuthorDate => CommitOrdValue::Timestamp(commit.author().timestamp),
            CommitSortKey::CommitterName => CommitOrdValue::Str(commit.committer().name.clone()),
            CommitSortKey::CommitterEmail => CommitOrdValue::Str(commit.committer().email.clone()),
            CommitSortKey::CommitterDate => CommitOrdValue::Timestamp(commit.committer().timestamp),
        })
}
