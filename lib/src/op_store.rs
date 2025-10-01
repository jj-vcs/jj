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

use std::any::Any;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Debug;
use std::iter;
use std::sync::LazyLock;
use std::time::SystemTime;

use itertools::Itertools as _;
use thiserror::Error;

use crate::backend::CommitId;
use crate::backend::MillisSinceEpoch;
use crate::backend::Timestamp;
use crate::content_hash::ContentHash;
use crate::merge::Merge;
use crate::object_id::HexPrefix;
use crate::object_id::ObjectId as _;
use crate::object_id::PrefixResolution;
use crate::object_id::id_type;
use crate::ref_name::GitRefNameBuf;
use crate::ref_name::RefName;
use crate::ref_name::RefNameBuf;
use crate::ref_name::RemoteName;
use crate::ref_name::RemoteNameBuf;
use crate::ref_name::RemoteRefSymbol;
use crate::ref_name::WorkspaceNameBuf;

id_type!(pub ViewId { hex() });
id_type!(pub OperationId { hex() });

#[derive(ContentHash, PartialEq, Eq, Hash, Clone, Debug, serde::Serialize)]
#[serde(transparent)]
pub struct RefTarget {
    merge: Merge<Option<CommitId>>,
}

impl Default for RefTarget {
    fn default() -> Self {
        Self::absent()
    }
}

impl RefTarget {
    /// Creates non-conflicting target pointing to no commit.
    pub fn absent() -> Self {
        Self::from_merge(Merge::absent())
    }

    /// Returns non-conflicting target pointing to no commit.
    ///
    /// This will typically be used in place of `None` returned by map lookup.
    pub fn absent_ref() -> &'static Self {
        static TARGET: LazyLock<RefTarget> = LazyLock::new(RefTarget::absent);
        &TARGET
    }

    /// Creates non-conflicting target that optionally points to a commit.
    pub fn resolved(maybe_id: Option<CommitId>) -> Self {
        Self::from_merge(Merge::resolved(maybe_id))
    }

    /// Creates non-conflicting target pointing to a commit.
    pub fn normal(id: CommitId) -> Self {
        Self::from_merge(Merge::normal(id))
    }

    /// Creates target from removed/added ids.
    pub fn from_legacy_form(
        removed_ids: impl IntoIterator<Item = CommitId>,
        added_ids: impl IntoIterator<Item = CommitId>,
    ) -> Self {
        Self::from_merge(Merge::from_legacy_form(removed_ids, added_ids))
    }

    pub fn from_merge(merge: Merge<Option<CommitId>>) -> Self {
        Self { merge }
    }

    /// Returns the underlying value if this target is non-conflicting.
    pub fn as_resolved(&self) -> Option<&Option<CommitId>> {
        self.merge.as_resolved()
    }

    /// Returns id if this target is non-conflicting and points to a commit.
    pub fn as_normal(&self) -> Option<&CommitId> {
        self.merge.as_normal()
    }

    /// Returns true if this target points to no commit.
    pub fn is_absent(&self) -> bool {
        self.merge.is_absent()
    }

    /// Returns true if this target points to any commit. Conflicting target is
    /// always "present" as it should have at least one commit id.
    pub fn is_present(&self) -> bool {
        self.merge.is_present()
    }

    /// Whether this target has conflicts.
    pub fn has_conflict(&self) -> bool {
        !self.merge.is_resolved()
    }

    pub fn removed_ids(&self) -> impl Iterator<Item = &CommitId> {
        self.merge.removes().flatten()
    }

    pub fn added_ids(&self) -> impl Iterator<Item = &CommitId> {
        self.merge.adds().flatten()
    }

    pub fn as_merge(&self) -> &Merge<Option<CommitId>> {
        &self.merge
    }
}

/// Remote bookmark or tag.
#[derive(ContentHash, Clone, Debug, Eq, Hash, PartialEq)]
pub struct RemoteRef {
    pub target: RefTarget,
    pub state: RemoteRefState,
}

impl RemoteRef {
    /// Creates remote ref pointing to no commit.
    pub fn absent() -> Self {
        Self {
            target: RefTarget::absent(),
            state: RemoteRefState::New,
        }
    }

    /// Returns remote ref pointing to no commit.
    ///
    /// This will typically be used in place of `None` returned by map lookup.
    pub fn absent_ref() -> &'static Self {
        static TARGET: LazyLock<RemoteRef> = LazyLock::new(RemoteRef::absent);
        &TARGET
    }

    /// Returns true if the target points to no commit.
    pub fn is_absent(&self) -> bool {
        self.target.is_absent()
    }

    /// Returns true if the target points to any commit.
    pub fn is_present(&self) -> bool {
        self.target.is_present()
    }

    /// Returns true if the ref is supposed to be merged in to the local ref.
    pub fn is_tracked(&self) -> bool {
        self.state == RemoteRefState::Tracked
    }

    /// Target that should have been merged in to the local ref.
    ///
    /// Use this as the base or known target when merging new remote ref in to
    /// local or pushing local ref to remote.
    pub fn tracked_target(&self) -> &RefTarget {
        if self.is_tracked() {
            &self.target
        } else {
            RefTarget::absent_ref()
        }
    }
}

/// Whether the ref is tracked or not.
#[derive(ContentHash, Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RemoteRefState {
    /// Remote ref is not merged in to the local ref.
    New,
    /// Remote ref has been merged in to the local ref. Incoming ref will be
    /// merged, too.
    Tracked,
}

/// Helper to strip redundant `Option<T>` from `RefTarget` lookup result.
pub trait RefTargetOptionExt {
    type Value;

    fn flatten(self) -> Self::Value;
}

impl RefTargetOptionExt for Option<RefTarget> {
    type Value = RefTarget;

    fn flatten(self) -> Self::Value {
        self.unwrap_or_else(RefTarget::absent)
    }
}

impl<'a> RefTargetOptionExt for Option<&'a RefTarget> {
    type Value = &'a RefTarget;

    fn flatten(self) -> Self::Value {
        self.unwrap_or_else(|| RefTarget::absent_ref())
    }
}

impl RefTargetOptionExt for Option<RemoteRef> {
    type Value = RemoteRef;

    fn flatten(self) -> Self::Value {
        self.unwrap_or_else(RemoteRef::absent)
    }
}

impl<'a> RefTargetOptionExt for Option<&'a RemoteRef> {
    type Value = &'a RemoteRef;

    fn flatten(self) -> Self::Value {
        self.unwrap_or_else(|| RemoteRef::absent_ref())
    }
}

/// Local and remote refs of the same name.
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct LocalRemoteRefTarget<'a> {
    /// The commit the ref points to locally.
    pub local_target: &'a RefTarget,
    /// `(remote_name, remote_ref)` pairs in lexicographical order.
    pub remote_refs: Vec<(&'a RemoteName, &'a RemoteRef)>,
}

/// Represents the way the repo looks at a given time, just like how a Tree
/// object represents how the file system looks at a given time.
#[derive(ContentHash, PartialEq, Eq, Clone, Debug)]
pub struct View {
    /// All head commits. There should be at least one head commit.
    pub head_ids: HashSet<CommitId>,
    pub local_bookmarks: BTreeMap<RefNameBuf, RefTarget>,
    pub local_tags: BTreeMap<RefNameBuf, RefTarget>,
    pub remote_views: BTreeMap<RemoteNameBuf, RemoteView>,
    pub git_refs: BTreeMap<GitRefNameBuf, RefTarget>,
    /// The commit the Git HEAD points to.
    // TODO: Support multiple Git worktrees?
    // TODO: Do we want to store the current bookmark name too?
    pub git_head: RefTarget,
    // The commit that *should be* checked out in the workspace. Note that the working copy
    // (.jj/working_copy/) has the source of truth about which commit *is* checked out (to be
    // precise: the commit to which we most recently completed an update to).
    pub wc_commit_ids: BTreeMap<WorkspaceNameBuf, CommitId>,
}

impl View {
    /// Creates new (mostly empty) view containing the given commit as the head.
    pub fn make_root(root_commit_id: CommitId) -> Self {
        Self {
            head_ids: HashSet::from([root_commit_id]),
            local_bookmarks: BTreeMap::new(),
            local_tags: BTreeMap::new(),
            remote_views: BTreeMap::new(),
            git_refs: BTreeMap::new(),
            git_head: RefTarget::absent(),
            wc_commit_ids: BTreeMap::new(),
        }
    }
}

/// Represents the state of the remote repo.
#[derive(ContentHash, Clone, Debug, Default, Eq, PartialEq)]
pub struct RemoteView {
    // TODO: Do we need to support tombstones for remote bookmarks? For example, if the bookmark
    // has been deleted locally and you pull from a remote, maybe it should make a difference
    // whether the bookmark is known to have existed on the remote. We may not want to resurrect
    // the bookmark if the bookmark's state on the remote was just not known.
    pub bookmarks: BTreeMap<RefNameBuf, RemoteRef>,
    pub tags: BTreeMap<RefNameBuf, RemoteRef>,
}

/// Iterates pair of local and remote refs by name.
pub(crate) fn merge_join_ref_views<'a>(
    local_refs: &'a BTreeMap<RefNameBuf, RefTarget>,
    remote_views: &'a BTreeMap<RemoteNameBuf, RemoteView>,
    get_remote_refs: impl FnMut(&RemoteView) -> &BTreeMap<RefNameBuf, RemoteRef>,
) -> impl Iterator<Item = (&'a RefName, LocalRemoteRefTarget<'a>)> {
    let mut local_refs_iter = local_refs
        .iter()
        .map(|(name, target)| (&**name, target))
        .peekable();
    let mut remote_refs_iter = flatten_remote_refs(remote_views, get_remote_refs).peekable();

    iter::from_fn(move || {
        // Pick earlier bookmark name
        let (name, local_target) = if let Some((symbol, _)) = remote_refs_iter.peek() {
            local_refs_iter
                .next_if(|&(local_name, _)| local_name <= symbol.name)
                .unwrap_or((symbol.name, RefTarget::absent_ref()))
        } else {
            local_refs_iter.next()?
        };
        let remote_refs = remote_refs_iter
            .peeking_take_while(|(symbol, _)| symbol.name == name)
            .map(|(symbol, remote_ref)| (symbol.remote, remote_ref))
            .collect();
        let local_remote_target = LocalRemoteRefTarget {
            local_target,
            remote_refs,
        };
        Some((name, local_remote_target))
    })
}

/// Iterates `(symbol, remote_ref)`s in lexicographical order.
pub(crate) fn flatten_remote_refs(
    remote_views: &BTreeMap<RemoteNameBuf, RemoteView>,
    mut get_remote_refs: impl FnMut(&RemoteView) -> &BTreeMap<RefNameBuf, RemoteRef>,
) -> impl Iterator<Item = (RemoteRefSymbol<'_>, &RemoteRef)> {
    remote_views
        .iter()
        .map(|(remote, remote_view)| {
            get_remote_refs(remote_view)
                .iter()
                .map(move |(name, remote_ref)| (name.to_remote_symbol(remote), remote_ref))
        })
        .kmerge_by(|(symbol1, _), (symbol2, _)| symbol1 < symbol2)
}

#[derive(Clone, ContentHash, Debug, Eq, PartialEq, serde::Serialize)]
pub struct TimestampRange {
    // Could be aliased to Range<Timestamp> if needed.
    pub start: Timestamp,
    pub end: Timestamp,
}

/// Represents an operation (transaction) on the repo view, just like how a
/// Commit object represents an operation on the tree.
///
/// Operations and views are not meant to be exchanged between repos or users;
/// they represent local state and history.
///
/// The operation history will almost always be linear. It will only have
/// forks when parallel operations occurred. The parent is determined when
/// the transaction starts. When the transaction commits, a lock will be
/// taken and it will be checked that the current head of the operation
/// graph is unchanged. If the current head has changed, there has been
/// concurrent operation.
#[derive(ContentHash, PartialEq, Eq, Clone, Debug, serde::Serialize)]
pub struct Operation {
    #[serde(skip)] // TODO: should be exposed?
    pub view_id: ViewId,
    pub parents: Vec<OperationId>,
    #[serde(flatten)]
    pub metadata: OperationMetadata,
    /// Mapping from new commit to its predecessors, or `None` if predecessors
    /// weren't recorded when the operation was written.
    ///
    /// * `commit_id: []` if the commit was newly created.
    /// * `commit_id: [predecessor_id, ..]` if the commit was rewritten.
    ///
    /// This mapping preserves all transitive predecessors if a commit was
    /// rewritten multiple times within the same transaction. For example, if
    /// `X` was rewritten as `Y`, then rebased as `Z`, these modifications are
    /// recorded as `{Y: [X], Z: [Y]}`.
    ///
    /// Existing commits (including commits imported from Git) aren't tracked
    /// even if they became visible at this operation.
    // BTreeMap for ease of deterministic serialization. If the deserialization
    // cost matters, maybe this can be changed to sorted Vec.
    #[serde(skip)] // TODO: should be exposed?
    pub commit_predecessors: Option<BTreeMap<CommitId, Vec<CommitId>>>,
}

impl Operation {
    pub fn make_root(root_view_id: ViewId) -> Self {
        let timestamp = Timestamp {
            timestamp: MillisSinceEpoch(0),
            tz_offset: 0,
        };
        let metadata = OperationMetadata {
            time: TimestampRange {
                start: timestamp,
                end: timestamp,
            },
            description: "".to_string(),
            hostname: "".to_string(),
            username: "".to_string(),
            is_snapshot: false,
            tags: HashMap::new(),
        };
        Self {
            view_id: root_view_id,
            parents: vec![],
            metadata,
            // The root operation is guaranteed to have no new commits. The root
            // commit could be considered born at the root operation, but there
            // may be other commits created within the abandoned operations.
            // They don't have any predecessors records as well.
            commit_predecessors: Some(BTreeMap::new()),
        }
    }
}

#[derive(ContentHash, PartialEq, Eq, Clone, Debug, serde::Serialize)]
pub struct OperationMetadata {
    pub time: TimestampRange,
    // Whatever is useful to the user, such as exact command line call
    pub description: String,
    pub hostname: String,
    pub username: String,
    /// Whether this operation represents a pure snapshotting of the working
    /// copy.
    pub is_snapshot: bool,
    pub tags: HashMap<String, String>,
}

/// Data to be loaded into the root operation/view.
#[derive(Clone, Debug)]
pub struct RootOperationData {
    /// The root commit ID, which should exist in the root view.
    pub root_commit_id: CommitId,
}

#[derive(Debug, Error)]
pub enum OpStoreError {
    #[error("Object {hash} of type {object_type} not found")]
    ObjectNotFound {
        object_type: String,
        hash: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("Error when reading object {hash} of type {object_type}")]
    ReadObject {
        object_type: String,
        hash: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("Could not write object of type {object_type}")]
    WriteObject {
        object_type: &'static str,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error(transparent)]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

pub type OpStoreResult<T> = Result<T, OpStoreError>;

pub trait OpStore: Any + Send + Sync + Debug {
    fn name(&self) -> &str;

    fn root_operation_id(&self) -> &OperationId;

    fn read_view(&self, id: &ViewId) -> OpStoreResult<View>;

    fn write_view(&self, contents: &View) -> OpStoreResult<ViewId>;

    fn read_operation(&self, id: &OperationId) -> OpStoreResult<Operation>;

    fn write_operation(&self, contents: &Operation) -> OpStoreResult<OperationId>;

    /// Resolves an unambiguous operation ID prefix.
    fn resolve_operation_id_prefix(
        &self,
        prefix: &HexPrefix,
    ) -> OpStoreResult<PrefixResolution<OperationId>>;

    /// Prunes unreachable operations and views.
    ///
    /// All operations and views reachable from the `head_ids` won't be
    /// removed. In addition to that, objects created after `keep_newer` will be
    /// preserved. This mitigates a risk of deleting new heads created
    /// concurrently by another process.
    // TODO: return stats?
    fn gc(&self, head_ids: &[OperationId], keep_newer: SystemTime) -> OpStoreResult<()>;
}

impl dyn OpStore {
    /// Returns reference of the implementation type.
    pub fn downcast_ref<T: OpStore>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref()
    }
}

#[cfg(test)]
mod tests {
    use maplit::btreemap;

    use super::*;

    #[test]
    fn test_merge_join_bookmark_views() {
        let remote_ref = |target: &RefTarget| RemoteRef {
            target: target.clone(),
            state: RemoteRefState::Tracked, // doesn't matter
        };
        let local_bookmark1_target = RefTarget::normal(CommitId::from_hex("111111"));
        let local_bookmark2_target = RefTarget::normal(CommitId::from_hex("222222"));
        let git_bookmark1_remote_ref = remote_ref(&RefTarget::normal(CommitId::from_hex("333333")));
        let git_bookmark2_remote_ref = remote_ref(&RefTarget::normal(CommitId::from_hex("444444")));
        let remote1_bookmark1_remote_ref =
            remote_ref(&RefTarget::normal(CommitId::from_hex("555555")));
        let remote2_bookmark2_remote_ref =
            remote_ref(&RefTarget::normal(CommitId::from_hex("666666")));

        let local_bookmarks = btreemap! {
            "bookmark1".into() => local_bookmark1_target.clone(),
            "bookmark2".into() => local_bookmark2_target.clone(),
        };
        let remote_views = btreemap! {
            "git".into() => RemoteView {
                bookmarks: btreemap! {
                    "bookmark1".into() => git_bookmark1_remote_ref.clone(),
                    "bookmark2".into() => git_bookmark2_remote_ref.clone(),
                },
                tags: btreemap! {},
            },
            "remote1".into() => RemoteView {
                bookmarks: btreemap! {
                    "bookmark1".into() => remote1_bookmark1_remote_ref.clone(),
                },
                tags: btreemap! {},
            },
            "remote2".into() => RemoteView {
                bookmarks: btreemap! {
                    "bookmark2".into() => remote2_bookmark2_remote_ref.clone(),
                },
                tags: btreemap! {},
            },
        };
        assert_eq!(
            merge_join_ref_views(&local_bookmarks, &remote_views, |view| &view.bookmarks)
                .collect_vec(),
            vec![
                (
                    "bookmark1".as_ref(),
                    LocalRemoteRefTarget {
                        local_target: &local_bookmark1_target,
                        remote_refs: vec![
                            ("git".as_ref(), &git_bookmark1_remote_ref),
                            ("remote1".as_ref(), &remote1_bookmark1_remote_ref),
                        ],
                    },
                ),
                (
                    "bookmark2".as_ref(),
                    LocalRemoteRefTarget {
                        local_target: &local_bookmark2_target.clone(),
                        remote_refs: vec![
                            ("git".as_ref(), &git_bookmark2_remote_ref),
                            ("remote2".as_ref(), &remote2_bookmark2_remote_ref),
                        ],
                    },
                ),
            ],
        );

        // Local only
        let local_bookmarks = btreemap! {
            "bookmark1".into() => local_bookmark1_target.clone(),
        };
        let remote_views = btreemap! {};
        assert_eq!(
            merge_join_ref_views(&local_bookmarks, &remote_views, |view| &view.bookmarks)
                .collect_vec(),
            vec![(
                "bookmark1".as_ref(),
                LocalRemoteRefTarget {
                    local_target: &local_bookmark1_target,
                    remote_refs: vec![]
                },
            )],
        );

        // Remote only
        let local_bookmarks = btreemap! {};
        let remote_views = btreemap! {
            "remote1".into() => RemoteView {
                bookmarks: btreemap! {
                    "bookmark1".into() => remote1_bookmark1_remote_ref.clone(),
                },
                tags: btreemap! {},
            },
        };
        assert_eq!(
            merge_join_ref_views(&local_bookmarks, &remote_views, |view| &view.bookmarks)
                .collect_vec(),
            vec![(
                "bookmark1".as_ref(),
                LocalRemoteRefTarget {
                    local_target: RefTarget::absent_ref(),
                    remote_refs: vec![("remote1".as_ref(), &remote1_bookmark1_remote_ref)],
                },
            )],
        );
    }
}
