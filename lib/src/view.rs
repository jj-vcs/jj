// Copyright 2020 The Jujutsu Authors
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

#![expect(missing_docs)]

use std::collections::BTreeMap;
use std::collections::HashSet;

use itertools::Itertools as _;
use thiserror::Error;

use crate::backend::CommitId;
use crate::op_store;
use crate::op_store::LocalRemoteRefTarget;
use crate::op_store::RefTarget;
use crate::op_store::RefTargetOptionExt as _;
use crate::op_store::RemoteRef;
use crate::op_store::RemoteView;
use crate::ref_name::GitRefName;
use crate::ref_name::GitRefNameBuf;
use crate::ref_name::RefName;
use crate::ref_name::RemoteName;
use crate::ref_name::RemoteRefSymbol;
use crate::ref_name::WorkspaceName;
use crate::ref_name::WorkspaceNameBuf;
use crate::refs;
use crate::refs::LocalAndRemoteRef;
use crate::str_util::StringMatcher;

/// A wrapper around [`op_store::View`] that defines additional methods.
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct View {
    data: op_store::View,
}

impl View {
    pub fn new(op_store_view: op_store::View) -> Self {
        Self {
            data: op_store_view,
        }
    }

    pub fn wc_commit_ids(&self) -> &BTreeMap<WorkspaceNameBuf, CommitId> {
        &self.data.wc_commit_ids
    }

    pub fn get_wc_commit_id(&self, name: &WorkspaceName) -> Option<&CommitId> {
        self.data.wc_commit_ids.get(name)
    }

    pub fn workspaces_for_wc_commit_id(&self, commit_id: &CommitId) -> Vec<WorkspaceNameBuf> {
        let mut workspace_names = vec![];
        for (name, wc_commit_id) in &self.data.wc_commit_ids {
            if wc_commit_id == commit_id {
                workspace_names.push(name.clone());
            }
        }
        workspace_names
    }

    pub fn is_wc_commit_id(&self, commit_id: &CommitId) -> bool {
        self.data.wc_commit_ids.values().contains(commit_id)
    }

    pub fn heads(&self) -> &HashSet<CommitId> {
        &self.data.head_ids
    }

    /// Iterates pair of local and remote bookmarks by bookmark name.
    pub fn bookmarks(&self) -> impl Iterator<Item = (&RefName, LocalRemoteRefTarget<'_>)> {
        op_store::merge_join_ref_views(
            &self.data.local_bookmarks,
            &self.data.remote_views,
            |view| &view.bookmarks,
        )
    }

    /// Iterates pair of local and remote tags by tag name.
    pub fn tags(&self) -> impl Iterator<Item = (&RefName, LocalRemoteRefTarget<'_>)> {
        op_store::merge_join_ref_views(&self.data.local_tags, &self.data.remote_views, |view| {
            &view.tags
        })
    }

    pub fn git_refs(&self) -> &BTreeMap<GitRefNameBuf, RefTarget> {
        &self.data.git_refs
    }

    pub fn git_head(&self) -> &RefTarget {
        &self.data.git_head
    }

    pub fn set_wc_commit(&mut self, name: WorkspaceNameBuf, commit_id: CommitId) {
        self.data.wc_commit_ids.insert(name, commit_id);
    }

    pub fn remove_wc_commit(&mut self, name: &WorkspaceName) {
        self.data.wc_commit_ids.remove(name);
    }

    pub fn rename_workspace(
        &mut self,
        old_name: &WorkspaceName,
        new_name: WorkspaceNameBuf,
    ) -> Result<(), RenameWorkspaceError> {
        if self.data.wc_commit_ids.contains_key(&new_name) {
            return Err(RenameWorkspaceError::WorkspaceAlreadyExists {
                name: new_name.clone(),
            });
        }
        let wc_commit_id = self.data.wc_commit_ids.remove(old_name).ok_or_else(|| {
            RenameWorkspaceError::WorkspaceDoesNotExist {
                name: old_name.to_owned(),
            }
        })?;
        self.data.wc_commit_ids.insert(new_name, wc_commit_id);
        Ok(())
    }

    pub fn add_head(&mut self, head_id: &CommitId) {
        self.data.head_ids.insert(head_id.clone());
    }

    pub fn remove_head(&mut self, head_id: &CommitId) {
        self.data.head_ids.remove(head_id);
    }

    /// Iterates local bookmark `(name, target)`s in lexicographical order.
    pub fn local_bookmarks(&self) -> impl Iterator<Item = (&RefName, &RefTarget)> {
        self.data
            .local_bookmarks
            .iter()
            .map(|(name, target)| (name.as_ref(), target))
    }

    /// Iterates local bookmarks `(name, target)` in lexicographical order where
    /// the target adds `commit_id`.
    pub fn local_bookmarks_for_commit(
        &self,
        commit_id: &CommitId,
    ) -> impl Iterator<Item = (&RefName, &RefTarget)> {
        self.local_bookmarks()
            .filter(|(_, target)| target.added_ids().contains(commit_id))
    }

    /// Iterates local bookmark `(name, target)`s matching the given pattern.
    /// Entries are sorted by `name`.
    pub fn local_bookmarks_matching(
        &self,
        matcher: &StringMatcher,
    ) -> impl Iterator<Item = (&RefName, &RefTarget)> {
        matcher
            .filter_btree_map_as_deref(&self.data.local_bookmarks)
            .map(|(name, target)| (name.as_ref(), target))
    }

    pub fn get_local_bookmark(&self, name: &RefName) -> &RefTarget {
        self.data.local_bookmarks.get(name).flatten()
    }

    /// Sets local bookmark to point to the given target. If the target is
    /// absent, the local bookmark will be removed. If there are absent remote
    /// bookmarks tracked by the newly-absent local bookmark, they will also be
    /// removed.
    pub fn set_local_bookmark_target(&mut self, name: &RefName, target: RefTarget) {
        if target.is_present() {
            self.data.local_bookmarks.insert(name.to_owned(), target);
        } else {
            self.data.local_bookmarks.remove(name);
            for remote_view in self.data.remote_views.values_mut() {
                let remote_refs = &mut remote_view.bookmarks;
                if remote_refs.get(name).is_some_and(RemoteRef::is_absent) {
                    remote_refs.remove(name);
                }
            }
        }
    }

    /// Iterates over `(symbol, remote_ref)` for all remote bookmarks in
    /// lexicographical order.
    pub fn all_remote_bookmarks(&self) -> impl Iterator<Item = (RemoteRefSymbol<'_>, &RemoteRef)> {
        op_store::flatten_remote_refs(&self.data.remote_views, |view| &view.bookmarks)
    }

    /// Iterates over `(name, remote_ref)`s for all remote bookmarks of the
    /// specified remote in lexicographical order.
    pub fn remote_bookmarks(
        &self,
        remote_name: &RemoteName,
    ) -> impl Iterator<Item = (&RefName, &RemoteRef)> + use<'_> {
        let maybe_remote_view = self.data.remote_views.get(remote_name);
        maybe_remote_view
            .map(|remote_view| {
                remote_view
                    .bookmarks
                    .iter()
                    .map(|(name, remote_ref)| (name.as_ref(), remote_ref))
            })
            .into_iter()
            .flatten()
    }

    /// Iterates over `(symbol, remote_ref)`s for all remote bookmarks of the
    /// specified remote that match the given pattern.
    ///
    /// Entries are sorted by `symbol`, which is `(name, remote)`.
    pub fn remote_bookmarks_matching(
        &self,
        bookmark_matcher: &StringMatcher,
        remote_matcher: &StringMatcher,
    ) -> impl Iterator<Item = (RemoteRefSymbol<'_>, &RemoteRef)> {
        // Use kmerge instead of flat_map for consistency with all_remote_bookmarks().
        remote_matcher
            .filter_btree_map_as_deref(&self.data.remote_views)
            .map(|(remote, remote_view)| {
                bookmark_matcher
                    .filter_btree_map_as_deref(&remote_view.bookmarks)
                    .map(|(name, remote_ref)| (name.to_remote_symbol(remote), remote_ref))
            })
            .kmerge_by(|(symbol1, _), (symbol2, _)| symbol1 < symbol2)
    }

    pub fn get_remote_bookmark(&self, symbol: RemoteRefSymbol<'_>) -> &RemoteRef {
        if let Some(remote_view) = self.data.remote_views.get(symbol.remote) {
            remote_view.bookmarks.get(symbol.name).flatten()
        } else {
            RemoteRef::absent_ref()
        }
    }

    /// Sets remote-tracking bookmark to the given target and state. If the
    /// target is absent and if no tracking local bookmark exists, the bookmark
    /// will be removed.
    pub fn set_remote_bookmark(&mut self, symbol: RemoteRefSymbol<'_>, remote_ref: RemoteRef) {
        if remote_ref.is_present()
            || (remote_ref.is_tracked() && self.get_local_bookmark(symbol.name).is_present())
        {
            let remote_view = self
                .data
                .remote_views
                .entry(symbol.remote.to_owned())
                .or_default();
            remote_view
                .bookmarks
                .insert(symbol.name.to_owned(), remote_ref);
        } else if let Some(remote_view) = self.data.remote_views.get_mut(symbol.remote) {
            remote_view.bookmarks.remove(symbol.name);
        }
    }

    /// Iterates over `(name, {local_ref, remote_ref})`s for every bookmark
    /// present locally and/or on the specified remote, in lexicographical
    /// order.
    ///
    /// Note that this does *not* take into account whether the local bookmark
    /// tracks the remote bookmark or not. Missing values are represented as
    /// RefTarget::absent_ref() or RemoteRef::absent_ref().
    pub fn local_remote_bookmarks(
        &self,
        remote_name: &RemoteName,
    ) -> impl Iterator<Item = (&RefName, LocalAndRemoteRef<'_>)> + use<'_> {
        refs::iter_named_local_remote_refs(
            self.local_bookmarks(),
            self.remote_bookmarks(remote_name),
        )
        .map(|(name, (local_target, remote_ref))| {
            let targets = LocalAndRemoteRef {
                local_target,
                remote_ref,
            };
            (name, targets)
        })
    }

    /// Iterates over `(name, TrackingRefPair {local_ref, remote_ref})`s for
    /// every bookmark with a name that matches the given pattern, and that is
    /// present locally and/or on the specified remote.
    ///
    /// Entries are sorted by `name`.
    ///
    /// Note that this does *not* take into account whether the local bookmark
    /// tracks the remote bookmark or not. Missing values are represented as
    /// RefTarget::absent_ref() or RemoteRef::absent_ref().
    pub fn local_remote_bookmarks_matching<'a, 'b>(
        &'a self,
        bookmark_matcher: &'b StringMatcher,
        remote_name: &RemoteName,
    ) -> impl Iterator<Item = (&'a RefName, LocalAndRemoteRef<'a>)> + use<'a, 'b> {
        // Change remote_name to StringMatcher if needed, but merge-join adapter won't
        // be usable.
        let maybe_remote_view = self.data.remote_views.get(remote_name);
        refs::iter_named_local_remote_refs(
            bookmark_matcher.filter_btree_map_as_deref(&self.data.local_bookmarks),
            maybe_remote_view
                .map(|remote_view| {
                    bookmark_matcher.filter_btree_map_as_deref(&remote_view.bookmarks)
                })
                .into_iter()
                .flatten(),
        )
        .map(|(name, (local_target, remote_ref))| {
            let targets = LocalAndRemoteRef {
                local_target,
                remote_ref,
            };
            (name.as_ref(), targets)
        })
    }

    /// Iterates remote `(name, view)`s in lexicographical order.
    pub fn remote_views(&self) -> impl Iterator<Item = (&RemoteName, &RemoteView)> {
        self.data
            .remote_views
            .iter()
            .map(|(name, view)| (name.as_ref(), view))
    }

    /// Iterates matching remote `(name, view)`s in lexicographical order.
    pub fn remote_views_matching(
        &self,
        matcher: &StringMatcher,
    ) -> impl Iterator<Item = (&RemoteName, &RemoteView)> {
        matcher
            .filter_btree_map_as_deref(&self.data.remote_views)
            .map(|(name, view)| (name.as_ref(), view))
    }

    /// Returns the remote view for `name`.
    pub fn get_remote_view(&self, name: &RemoteName) -> Option<&RemoteView> {
        self.data.remote_views.get(name)
    }

    /// Adds remote view if it doesn't exist.
    pub fn ensure_remote(&mut self, remote_name: &RemoteName) {
        if self.data.remote_views.contains_key(remote_name) {
            return;
        }
        self.data
            .remote_views
            .insert(remote_name.to_owned(), RemoteView::default());
    }

    pub fn remove_remote(&mut self, remote_name: &RemoteName) {
        self.data.remote_views.remove(remote_name);
    }

    pub fn rename_remote(&mut self, old: &RemoteName, new: &RemoteName) {
        if let Some(remote_view) = self.data.remote_views.remove(old) {
            self.data.remote_views.insert(new.to_owned(), remote_view);
        }
    }

    /// Iterates local tag `(name, target)`s in lexicographical order.
    pub fn local_tags(&self) -> impl Iterator<Item = (&RefName, &RefTarget)> {
        self.data
            .local_tags
            .iter()
            .map(|(name, target)| (name.as_ref(), target))
    }

    pub fn get_local_tag(&self, name: &RefName) -> &RefTarget {
        self.data.local_tags.get(name).flatten()
    }

    /// Iterates local tag `(name, target)`s matching the given pattern. Entries
    /// are sorted by `name`.
    pub fn local_tags_matching(
        &self,
        matcher: &StringMatcher,
    ) -> impl Iterator<Item = (&RefName, &RefTarget)> {
        matcher
            .filter_btree_map_as_deref(&self.data.local_tags)
            .map(|(name, target)| (name.as_ref(), target))
    }

    /// Sets local tag to point to the given target. If the target is absent,
    /// the local tag will be removed. If there are absent remote tags tracked
    /// by the newly-absent local tag, they will also be removed.
    pub fn set_local_tag_target(&mut self, name: &RefName, target: RefTarget) {
        if target.is_present() {
            self.data.local_tags.insert(name.to_owned(), target);
        } else {
            self.data.local_tags.remove(name);
            for remote_view in self.data.remote_views.values_mut() {
                let remote_refs = &mut remote_view.tags;
                if remote_refs.get(name).is_some_and(RemoteRef::is_absent) {
                    remote_refs.remove(name);
                }
            }
        }
    }

    /// Iterates over `(symbol, remote_ref)` for all remote tags in
    /// lexicographical order.
    pub fn all_remote_tags(&self) -> impl Iterator<Item = (RemoteRefSymbol<'_>, &RemoteRef)> {
        op_store::flatten_remote_refs(&self.data.remote_views, |view| &view.tags)
    }

    /// Iterates over `(name, remote_ref)`s for all remote tags of the specified
    /// remote in lexicographical order.
    pub fn remote_tags(
        &self,
        remote_name: &RemoteName,
    ) -> impl Iterator<Item = (&RefName, &RemoteRef)> + use<'_> {
        let maybe_remote_view = self.data.remote_views.get(remote_name);
        maybe_remote_view
            .map(|remote_view| {
                remote_view
                    .tags
                    .iter()
                    .map(|(name, remote_ref)| (name.as_ref(), remote_ref))
            })
            .into_iter()
            .flatten()
    }

    /// Iterates over `(symbol, remote_ref)`s for all remote tags of the
    /// specified remote that match the given pattern.
    ///
    /// Entries are sorted by `symbol`, which is `(name, remote)`.
    pub fn remote_tags_matching(
        &self,
        tag_matcher: &StringMatcher,
        remote_matcher: &StringMatcher,
    ) -> impl Iterator<Item = (RemoteRefSymbol<'_>, &RemoteRef)> {
        // Use kmerge instead of flat_map for consistency with all_remote_tags().
        remote_matcher
            .filter_btree_map_as_deref(&self.data.remote_views)
            .map(|(remote, remote_view)| {
                tag_matcher
                    .filter_btree_map_as_deref(&remote_view.tags)
                    .map(|(name, remote_ref)| (name.to_remote_symbol(remote), remote_ref))
            })
            .kmerge_by(|(symbol1, _), (symbol2, _)| symbol1 < symbol2)
    }

    /// Returns remote-tracking tag target and state specified by `symbol`.
    pub fn get_remote_tag(&self, symbol: RemoteRefSymbol<'_>) -> &RemoteRef {
        if let Some(remote_view) = self.data.remote_views.get(symbol.remote) {
            remote_view.tags.get(symbol.name).flatten()
        } else {
            RemoteRef::absent_ref()
        }
    }

    /// Sets remote-tracking tag to the given target and state. If the target is
    /// absent and if no tracking local tag exists, the tag will be removed.
    pub fn set_remote_tag(&mut self, symbol: RemoteRefSymbol<'_>, remote_ref: RemoteRef) {
        if remote_ref.is_present()
            || (remote_ref.is_tracked() && self.get_local_tag(symbol.name).is_present())
        {
            let remote_view = self
                .data
                .remote_views
                .entry(symbol.remote.to_owned())
                .or_default();
            remote_view.tags.insert(symbol.name.to_owned(), remote_ref);
        } else if let Some(remote_view) = self.data.remote_views.get_mut(symbol.remote) {
            remote_view.tags.remove(symbol.name);
        }
    }

    /// Iterates over `(name, {local_ref, remote_ref})`s for every tag present
    /// locally and/or on the specified remote, in lexicographical order.
    ///
    /// Note that this does *not* take into account whether the local tag tracks
    /// the remote tag or not. Missing values are represented as
    /// [`RefTarget::absent_ref()`] or [`RemoteRef::absent_ref()`].
    pub fn local_remote_tags(
        &self,
        remote_name: &RemoteName,
    ) -> impl Iterator<Item = (&RefName, LocalAndRemoteRef<'_>)> + use<'_> {
        refs::iter_named_local_remote_refs(self.local_tags(), self.remote_tags(remote_name)).map(
            |(name, (local_target, remote_ref))| {
                let targets = LocalAndRemoteRef {
                    local_target,
                    remote_ref,
                };
                (name, targets)
            },
        )
    }

    pub fn get_git_ref(&self, name: &GitRefName) -> &RefTarget {
        self.data.git_refs.get(name).flatten()
    }

    /// Sets the last imported Git ref to point to the given target. If the
    /// target is absent, the reference will be removed.
    pub fn set_git_ref_target(&mut self, name: &GitRefName, target: RefTarget) {
        if target.is_present() {
            self.data.git_refs.insert(name.to_owned(), target);
        } else {
            self.data.git_refs.remove(name);
        }
    }

    /// Sets Git HEAD to point to the given target. If the target is absent, the
    /// reference will be cleared.
    pub fn set_git_head_target(&mut self, target: RefTarget) {
        self.data.git_head = target;
    }

    /// Iterates all commit ids referenced by this view.
    ///
    /// This can include hidden commits referenced by remote bookmarks, previous
    /// positions of conflicted bookmarks, etc. The ancestors of the returned
    /// commits should be considered reachable from the view. Use this to build
    /// commit index from scratch.
    ///
    /// The iteration order is unspecified, and may include duplicated entries.
    pub fn all_referenced_commit_ids(&self) -> impl Iterator<Item = &CommitId> {
        // Include both added/removed ids since ancestry information of old
        // references will be needed while merging views.
        fn ref_target_ids(target: &RefTarget) -> impl Iterator<Item = &CommitId> {
            target.as_merge().iter().flatten()
        }

        // Some of the fields (e.g. wc_commit_ids) would be redundant, but let's
        // not be smart here. Callers will build a larger set of commits anyway.
        let op_store::View {
            head_ids,
            local_bookmarks,
            local_tags,
            remote_views,
            git_refs,
            git_head,
            wc_commit_ids,
        } = &self.data;
        itertools::chain!(
            head_ids,
            local_bookmarks.values().flat_map(ref_target_ids),
            local_tags.values().flat_map(ref_target_ids),
            remote_views.values().flat_map(|remote_view| {
                let op_store::RemoteView { bookmarks, tags } = remote_view;
                itertools::chain(bookmarks.values(), tags.values())
                    .flat_map(|remote_ref| ref_target_ids(&remote_ref.target))
            }),
            git_refs.values().flat_map(ref_target_ids),
            ref_target_ids(git_head),
            wc_commit_ids.values()
        )
    }

    pub fn set_view(&mut self, data: op_store::View) {
        self.data = data;
    }

    pub fn store_view(&self) -> &op_store::View {
        &self.data
    }

    pub fn store_view_mut(&mut self) -> &mut op_store::View {
        &mut self.data
    }
}

/// Error from attempts to rename a workspace
#[derive(Debug, Error)]
pub enum RenameWorkspaceError {
    #[error("Workspace {} not found", name.as_symbol())]
    WorkspaceDoesNotExist { name: WorkspaceNameBuf },

    #[error("Workspace {} already exists", name.as_symbol())]
    WorkspaceAlreadyExists { name: WorkspaceNameBuf },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op_store::RemoteRefState;

    fn remote_symbol<'a, N, M>(name: &'a N, remote: &'a M) -> RemoteRefSymbol<'a>
    where
        N: AsRef<RefName> + ?Sized,
        M: AsRef<RemoteName> + ?Sized,
    {
        RemoteRefSymbol {
            name: name.as_ref(),
            remote: remote.as_ref(),
        }
    }

    #[test]
    fn test_absent_tracked_bookmarks() {
        let mut view = View {
            data: op_store::View::make_root(CommitId::from_hex("000000")),
        };
        let absent_tracked_ref = RemoteRef {
            target: RefTarget::absent(),
            state: RemoteRefState::Tracked,
        };
        let present_tracked_ref = RemoteRef {
            target: RefTarget::normal(CommitId::from_hex("111111")),
            state: RemoteRefState::Tracked,
        };

        // Absent remote ref cannot be tracked by absent local ref
        view.set_remote_bookmark(remote_symbol("foo", "new"), absent_tracked_ref.clone());
        assert_eq!(
            view.get_remote_bookmark(remote_symbol("foo", "new")),
            RemoteRef::absent_ref()
        );

        // Present remote ref can be tracked by absent local ref
        view.set_remote_bookmark(remote_symbol("foo", "present"), present_tracked_ref.clone());
        assert_eq!(
            view.get_remote_bookmark(remote_symbol("foo", "present")),
            &present_tracked_ref
        );

        // Absent remote ref can be tracked by present local ref
        view.set_local_bookmark_target(
            "foo".as_ref(),
            RefTarget::normal(CommitId::from_hex("222222")),
        );
        view.set_remote_bookmark(remote_symbol("foo", "new"), absent_tracked_ref.clone());
        assert_eq!(
            view.get_remote_bookmark(remote_symbol("foo", "new")),
            &absent_tracked_ref
        );

        // Absent remote ref should be removed if local ref becomes absent
        view.set_local_bookmark_target("foo".as_ref(), RefTarget::absent());
        assert_eq!(
            view.get_remote_bookmark(remote_symbol("foo", "new")),
            RemoteRef::absent_ref()
        );
        assert_eq!(
            view.get_remote_bookmark(remote_symbol("foo", "present")),
            &present_tracked_ref
        );
    }

    #[test]
    fn test_absent_tracked_tags() {
        let mut view = View {
            data: op_store::View::make_root(CommitId::from_hex("000000")),
        };
        let absent_tracked_ref = RemoteRef {
            target: RefTarget::absent(),
            state: RemoteRefState::Tracked,
        };
        let present_tracked_ref = RemoteRef {
            target: RefTarget::normal(CommitId::from_hex("111111")),
            state: RemoteRefState::Tracked,
        };

        // Absent remote ref cannot be tracked by absent local ref
        view.set_remote_tag(remote_symbol("foo", "new"), absent_tracked_ref.clone());
        assert_eq!(
            view.get_remote_tag(remote_symbol("foo", "new")),
            RemoteRef::absent_ref()
        );

        // Present remote ref can be tracked by absent local ref
        view.set_remote_tag(remote_symbol("foo", "present"), present_tracked_ref.clone());
        assert_eq!(
            view.get_remote_tag(remote_symbol("foo", "present")),
            &present_tracked_ref
        );

        // Absent remote ref can be tracked by present local ref
        view.set_local_tag_target(
            "foo".as_ref(),
            RefTarget::normal(CommitId::from_hex("222222")),
        );
        view.set_remote_tag(remote_symbol("foo", "new"), absent_tracked_ref.clone());
        assert_eq!(
            view.get_remote_tag(remote_symbol("foo", "new")),
            &absent_tracked_ref
        );

        // Absent remote ref should be removed if local ref becomes absent
        view.set_local_tag_target("foo".as_ref(), RefTarget::absent());
        assert_eq!(
            view.get_remote_tag(remote_symbol("foo", "new")),
            RemoteRef::absent_ref()
        );
        assert_eq!(
            view.get_remote_tag(remote_symbol("foo", "present")),
            &present_tracked_ref
        );
    }
}
