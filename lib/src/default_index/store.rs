// Copyright 2023 The Jujutsu Authors
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

use std::collections::HashMap;
use std::error::Error as StdError;
use std::fmt::Debug;
use std::fs;
use std::future;
use std::io;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::pin::pin;
use std::slice;
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::stream;
use itertools::Itertools as _;
use prost::Message as _;
use tempfile::NamedTempFile;
use thiserror::Error;

use super::changed_path::ChangedPathIndexSegmentId;
use super::changed_path::CompositeChangedPathIndex;
use super::changed_path::collect_changed_paths;
use super::composite::AsCompositeIndex as _;
use super::composite::CommitIndexSegmentId;
use super::entry::GlobalCommitPosition;
use super::mutable::DefaultMutableIndex;
use super::readonly::DefaultReadonlyIndex;
use super::readonly::FieldLengths;
use super::readonly::ReadonlyCommitIndexSegment;
use super::readonly::ReadonlyIndexLoadError;
use crate::backend::BackendError;
use crate::backend::BackendInitError;
use crate::backend::CommitId;
use crate::commit::CommitByCommitterTimestamp;
use crate::dag_walk_async;
use crate::file_util;
use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;
use crate::file_util::persist_content_addressed_temp_file;
use crate::file_util::persist_temp_file;
use crate::index::IndexStore;
use crate::index::IndexStoreError;
use crate::index::IndexStoreResult;
use crate::index::MutableIndex;
use crate::index::ReadonlyIndex;
use crate::object_id::ObjectId as _;
use crate::op_store::OpStoreError;
use crate::op_store::OperationId;
use crate::op_walk;
use crate::operation::Operation;
use crate::store::Store;

// BLAKE2b-512 hash length in hex string
const SEGMENT_FILE_NAME_LENGTH: usize = 64 * 2;

/// Error that may occur during `DefaultIndexStore` initialization.
#[derive(Debug, Error)]
#[error("Failed to initialize index store")]
pub struct DefaultIndexStoreInitError(#[from] pub PathError);

impl From<DefaultIndexStoreInitError> for BackendInitError {
    fn from(err: DefaultIndexStoreInitError) -> Self {
        Self(err.into())
    }
}

/// Error returned by default-index storage backends.
pub type DefaultIndexStorageError = Box<dyn StdError + Send + Sync + 'static>;

/// Result returned by default-index storage backends.
pub type DefaultIndexStorageResult<T> = Result<T, DefaultIndexStorageError>;

#[derive(Debug, Error)]
pub enum DefaultIndexStoreError {
    #[error("Failed to associate index records with an operation {op_id}")]
    AssociateIndex {
        op_id: OperationId,
        #[source]
        source: DefaultIndexStorageError,
    },
    #[error("No index records are associated with operation {op_id}")]
    IndexNotAssociated { op_id: OperationId },
    #[error("Failed to load associated index records for operation {op_id}")]
    LoadAssociation {
        op_id: OperationId,
        #[source]
        source: DefaultIndexStorageError,
    },
    #[error(transparent)]
    LoadIndex(ReadonlyIndexLoadError),
    #[error("Failed to save index data")]
    SaveIndex(#[source] DefaultIndexStorageError),
    #[error("Failed to reinitialize index store")]
    Reinit(#[source] DefaultIndexStorageError),
    #[error("Failed to index commits at operation {op_id}")]
    IndexCommits {
        op_id: OperationId,
        source: BackendError,
    },
    #[error(transparent)]
    OpStore(#[from] OpStoreError),
}

/// Provides persistent storage for default index records.
#[async_trait(?Send)]
pub trait DefaultIndexStorage: Send + Sync + Debug {
    /// Reads the segment-control record associated with `op_id`.
    ///
    /// Returns `Ok(None)` if no record exists. Backend failures should be
    /// returned as errors, not absence.
    async fn read_operation_link(
        &self,
        op_id: &OperationId,
    ) -> DefaultIndexStorageResult<Option<Vec<u8>>>;

    /// Writes the segment-control record associated with `op_id`.
    ///
    /// This is the publication point for an index version. Implementations must
    /// make referenced segments durable and visible before this record becomes
    /// visible to readers.
    async fn write_operation_link(
        &self,
        op_id: &OperationId,
        data: &[u8],
    ) -> DefaultIndexStorageResult<()>;

    /// Returns true if a segment-control record exists for `op_id`.
    ///
    /// Implementations should avoid reading the whole record if the backend has
    /// an efficient existence check.
    async fn has_operation_link(&self, op_id: &OperationId) -> DefaultIndexStorageResult<bool>;

    /// Reads a commit index segment by content-addressed name.
    ///
    /// Returns `Ok(None)` if no segment exists. Backend failures should be
    /// returned as errors, not absence.
    async fn read_commit_segment(&self, name: &str) -> DefaultIndexStorageResult<Option<Vec<u8>>>;

    /// Writes a commit index segment by content-addressed name.
    ///
    /// Segment names are derived from their content, so implementations should
    /// treat writes of identical data to an existing name as idempotent.
    async fn write_commit_segment(&self, name: &str, data: &[u8]) -> DefaultIndexStorageResult<()>;

    /// Reads a changed-path index segment by content-addressed name.
    ///
    /// Returns `Ok(None)` if no segment exists. Backend failures should be
    /// returned as errors, not absence.
    async fn read_changed_path_segment(
        &self,
        name: &str,
    ) -> DefaultIndexStorageResult<Option<Vec<u8>>>;

    /// Writes a changed-path index segment by content-addressed name.
    ///
    /// Segment names are derived from their content, so implementations should
    /// treat writes of identical data to an existing name as idempotent.
    async fn write_changed_path_segment(
        &self,
        name: &str,
        data: &[u8],
    ) -> DefaultIndexStorageResult<()>;

    /// Clears operation links and segment records so the index can be rebuilt.
    async fn reinit(&self) -> DefaultIndexStorageResult<()>;
}

/// Stores default index records in local filesystem directories.
#[derive(Debug)]
pub struct FileDefaultIndexStore {
    dir: PathBuf,
}

impl FileDefaultIndexStore {
    /// Creates filesystem-backed index storage in `dir`.
    pub fn init(dir: &Path) -> Result<Self, DefaultIndexStoreInitError> {
        let storage = Self::load(dir);
        storage.ensure_base_dirs()?;
        Ok(storage)
    }

    /// Loads filesystem-backed index storage from `dir`.
    pub fn load(dir: &Path) -> Self {
        Self {
            dir: dir.to_owned(),
        }
    }

    fn read_file(path: PathBuf) -> DefaultIndexStorageResult<Option<Vec<u8>>> {
        match fs::read(&path) {
            Ok(data) => Ok(Some(data)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(PathError { path, source: err }.into()),
        }
    }

    fn ensure_base_dirs(&self) -> Result<(), PathError> {
        for dir in [
            self.op_links_dir(),
            self.commit_segments_dir(),
            self.changed_path_segments_dir(),
        ] {
            file_util::create_or_reuse_dir(&dir).context(&dir)?;
        }
        Ok(())
    }

    /// Directory for mapping from operations to segments. (jj >= 0.33)
    fn op_links_dir(&self) -> PathBuf {
        self.dir.join("op_links")
    }

    /// Directory for commit segment files.
    fn commit_segments_dir(&self) -> PathBuf {
        self.dir.join("segments")
    }

    /// Directory for changed-path segment files.
    fn changed_path_segments_dir(&self) -> PathBuf {
        self.dir.join("changed_paths")
    }
}

#[async_trait(?Send)]
impl DefaultIndexStorage for FileDefaultIndexStore {
    async fn read_operation_link(
        &self,
        op_id: &OperationId,
    ) -> DefaultIndexStorageResult<Option<Vec<u8>>> {
        Self::read_file(self.op_links_dir().join(op_id.hex()))
    }

    async fn write_operation_link(
        &self,
        op_id: &OperationId,
        data: &[u8],
    ) -> DefaultIndexStorageResult<()> {
        self.ensure_base_dirs()?;
        let dir = self.op_links_dir();
        let path = dir.join(op_id.hex());
        let mut temp_file = NamedTempFile::new_in(&dir).context(&dir)?;
        temp_file
            .as_file_mut()
            .write_all(data)
            .context(temp_file.path())?;
        persist_temp_file(temp_file, &path).context(&path)?;
        Ok(())
    }

    async fn has_operation_link(&self, op_id: &OperationId) -> DefaultIndexStorageResult<bool> {
        let path = self.op_links_dir().join(op_id.hex());
        Ok(path.try_exists().context(&path)?)
    }

    async fn read_commit_segment(&self, name: &str) -> DefaultIndexStorageResult<Option<Vec<u8>>> {
        Self::read_file(self.commit_segments_dir().join(name))
    }

    async fn write_commit_segment(&self, name: &str, data: &[u8]) -> DefaultIndexStorageResult<()> {
        self.ensure_base_dirs()?;
        let dir = self.commit_segments_dir();
        let path = dir.join(name);
        let mut temp_file = NamedTempFile::new_in(&dir).context(&dir)?;
        temp_file
            .as_file_mut()
            .write_all(data)
            .context(temp_file.path())?;
        persist_content_addressed_temp_file(temp_file, &path).context(&path)?;
        Ok(())
    }

    async fn read_changed_path_segment(
        &self,
        name: &str,
    ) -> DefaultIndexStorageResult<Option<Vec<u8>>> {
        Self::read_file(self.changed_path_segments_dir().join(name))
    }

    async fn write_changed_path_segment(
        &self,
        name: &str,
        data: &[u8],
    ) -> DefaultIndexStorageResult<()> {
        self.ensure_base_dirs()?;
        let dir = self.changed_path_segments_dir();
        let path = dir.join(name);
        let mut temp_file = NamedTempFile::new_in(&dir).context(&dir)?;
        temp_file
            .as_file_mut()
            .write_all(data)
            .context(temp_file.path())?;
        persist_content_addressed_temp_file(temp_file, &path).context(&path)?;
        Ok(())
    }

    async fn reinit(&self) -> DefaultIndexStorageResult<()> {
        // Create base directories in case the store was initialized by old jj.
        self.ensure_base_dirs()?;
        // Remove all operation links to trigger rebuilding.
        let op_links_dir = self.op_links_dir();
        file_util::remove_dir_contents(&op_links_dir)?;
        let legacy_operations_dir = self.dir.join("operations"); // jj < 0.33
        if legacy_operations_dir.exists() {
            file_util::remove_dir_contents(&legacy_operations_dir)?;
            fs::remove_dir(&legacy_operations_dir).context(&legacy_operations_dir)?;
        }
        // Remove index segments to save disk space. If raced, new segment file
        // will be created by the other process.
        let commit_segments_dir = self.commit_segments_dir();
        file_util::remove_dir_contents(&commit_segments_dir)?;
        let changed_path_segments_dir = self.changed_path_segments_dir();
        file_util::remove_dir_contents(&changed_path_segments_dir)?;
        // jj <= 0.14 created segment files in the top directory
        for entry in self.dir.read_dir().context(&self.dir)? {
            let entry = entry.context(&self.dir)?;
            let path = entry.path();
            let Some(file_name) = path.file_name() else {
                continue;
            };
            if file_name.len() != SEGMENT_FILE_NAME_LENGTH {
                // Skip "type" file, "operations" directory, etc.
                continue;
            }
            fs::remove_file(&path).context(&path)?;
        }
        Ok(())
    }
}

/// Stores, builds, and loads the default commit index.
#[derive(Debug)]
pub struct DefaultIndexStore {
    storage: Arc<dyn DefaultIndexStorage>,
}

impl DefaultIndexStore {
    /// Returns the stable index-store backend name.
    pub fn name() -> &'static str {
        "default"
    }

    /// Creates a default index store backed by `storage`.
    pub fn with_storage(storage: Arc<dyn DefaultIndexStorage>) -> Self {
        Self { storage }
    }

    /// Creates a filesystem-backed default index store in `dir`.
    pub fn init(dir: &Path) -> Result<Self, DefaultIndexStoreInitError> {
        Ok(Self::with_storage(Arc::new(FileDefaultIndexStore::init(
            dir,
        )?)))
    }

    /// Loads a filesystem-backed default index store from `dir`.
    pub fn load(dir: &Path) -> Self {
        Self::with_storage(Arc::new(FileDefaultIndexStore::load(dir)))
    }

    /// Clears stored index records so the index can be rebuilt.
    pub async fn reinit(&self) -> Result<(), DefaultIndexStoreError> {
        self.storage
            .reinit()
            .await
            .map_err(DefaultIndexStoreError::Reinit)
    }

    async fn load_index_at_operation(
        &self,
        op_id: &OperationId,
        lengths: FieldLengths,
    ) -> Result<DefaultReadonlyIndex, DefaultIndexStoreError> {
        let Some(data) = self
            .storage
            .read_operation_link(op_id)
            .await
            .map_err(|source| DefaultIndexStoreError::LoadAssociation {
                op_id: op_id.clone(),
                source,
            })?
        else {
            return Err(DefaultIndexStoreError::IndexNotAssociated {
                op_id: op_id.clone(),
            });
        };
        let proto =
            crate::protos::default_index::SegmentControl::decode(&*data).map_err(|err| {
                DefaultIndexStoreError::LoadAssociation {
                    op_id: op_id.clone(),
                    source: io::Error::new(io::ErrorKind::InvalidData, err).into(),
                }
            })?;
        let commit_segment_id = CommitIndexSegmentId::new(proto.commit_segment_id);
        let changed_path_start_commit_pos = proto
            .changed_path_start_commit_pos
            .map(GlobalCommitPosition);
        let changed_path_segment_ids = proto
            .changed_path_segment_ids
            .into_iter()
            .map(ChangedPathIndexSegmentId::new)
            .collect_vec();

        let commits = ReadonlyCommitIndexSegment::load_from_storage(
            self.storage.as_ref(),
            commit_segment_id,
            lengths,
        )
        .await
        .map_err(DefaultIndexStoreError::LoadIndex)?;
        // TODO: lazy load or mmap?
        let changed_paths = if let Some(start_commit_pos) = changed_path_start_commit_pos {
            CompositeChangedPathIndex::load_segments_from_storage(
                self.storage.as_ref(),
                start_commit_pos,
                &changed_path_segment_ids,
            )
            .await
            .map_err(DefaultIndexStoreError::LoadIndex)?
        } else {
            CompositeChangedPathIndex::null()
        };
        Ok(DefaultReadonlyIndex::from_segment(commits, changed_paths))
    }

    /// Rebuilds index for the given `operation`.
    ///
    /// The index to be built will be calculated from one of the ancestor
    /// operations if exists. Use `reinit()` to rebuild index from scratch.
    #[tracing::instrument(skip(self, store))]
    pub async fn build_index_at_operation(
        &self,
        operation: &Operation,
        store: &Arc<Store>,
    ) -> Result<DefaultReadonlyIndex, DefaultIndexStoreError> {
        tracing::info!("scanning operations to index");
        let field_lengths = FieldLengths {
            commit_id: store.commit_id_length(),
            change_id: store.change_id_length(),
        };
        // Pick the latest existing ancestor operation as the parent segment.
        let mut unindexed_ops = Vec::new();
        let mut parent_op = None;
        let mut ancestors = pin!(op_walk::walk_ancestors(slice::from_ref(operation)));
        while let Some(op) = ancestors.next().await {
            let op = op?;
            if self
                .storage
                .has_operation_link(op.id())
                .await
                .map_err(|source| DefaultIndexStoreError::LoadAssociation {
                    op_id: op.id().clone(),
                    source,
                })?
            {
                parent_op = Some(op);
                break;
            } else {
                unindexed_ops.push(op);
            }
        }
        let ops_to_visit = if let Some(op) = &parent_op {
            // There may be concurrent ops, so revisit from the head. The parent
            // op is usually shallow if existed.
            op_walk::walk_ancestors_range(slice::from_ref(operation), slice::from_ref(op))
                .try_collect()
                .await?
        } else {
            unindexed_ops
        };
        tracing::info!(
            ops_count = ops_to_visit.len(),
            "collecting head commits to index"
        );
        let mut historical_heads: HashMap<CommitId, OperationId> = HashMap::new();
        for op in &ops_to_visit {
            for commit_id in itertools::chain(
                op.all_referenced_commit_ids(),
                op.view().await?.all_referenced_commit_ids(),
            ) {
                if !historical_heads.contains_key(commit_id) {
                    historical_heads.insert(commit_id.clone(), op.id().clone());
                }
            }
        }
        let mut mutable_index;
        let maybe_parent_index;
        match &parent_op {
            None => {
                mutable_index = DefaultMutableIndex::full(field_lengths);
                maybe_parent_index = None;
            }
            Some(op) => {
                let parent_index = self.load_index_at_operation(op.id(), field_lengths).await?;
                mutable_index = parent_index.start_modification();
                maybe_parent_index = Some(parent_index);
            }
        }

        tracing::info!(
            ?maybe_parent_index,
            heads_count = historical_heads.len(),
            "indexing commits reachable from historical heads"
        );
        // Build a list of ancestors of heads where parents come after the
        // commit itself.
        let parent_index_has_id = |id: &CommitId| {
            maybe_parent_index
                .as_ref()
                .is_some_and(|index| index.has_id_impl(id))
        };
        let get_commit_with_op = async |commit_id: &CommitId, op_id: &OperationId| {
            let op_id = op_id.clone();
            match store.get_commit_async(commit_id).await {
                // Propagate head's op_id to report possible source of an error.
                // The op_id doesn't have to be included in the sort key, but
                // that wouldn't matter since the commit should be unique.
                Ok(commit) => Ok((CommitByCommitterTimestamp(commit), op_id)),
                Err(source) => Err(DefaultIndexStoreError::IndexCommits { op_id, source }),
            }
        };
        let commits = dag_walk_async::topo_order_reverse_ord(
            stream::iter(&historical_heads)
                .filter(|&(commit_id, _)| future::ready(!parent_index_has_id(commit_id)))
                .map(|(commit_id, op_id)| get_commit_with_op(commit_id, op_id))
                .buffered(store.concurrency())
                .collect::<Vec<_>>()
                .await,
            |(CommitByCommitterTimestamp(commit), _)| commit.id().clone(),
            async |(CommitByCommitterTimestamp(commit), op_id)| {
                stream::iter(commit.parent_ids())
                    .filter(|&id| future::ready(!parent_index_has_id(id)))
                    .map(|commit_id| get_commit_with_op(commit_id, op_id))
                    .buffered(store.concurrency())
                    .collect::<Vec<_>>()
                    .await
            },
            |_| panic!("graph has cycle"),
        )
        .await?;
        for (CommitByCommitterTimestamp(commit), op_id) in commits.iter().rev() {
            mutable_index.add_commit(commit).await.map_err(|source| {
                DefaultIndexStoreError::IndexCommits {
                    op_id: op_id.clone(),
                    source,
                }
            })?;
        }

        let index = self
            .save_mutable_index(mutable_index, operation.id())
            .await?;
        tracing::info!(?index, commits_count = commits.len(), "saved new index");

        Ok(index)
    }

    /// Builds changed-path index for the specified operation.
    ///
    /// At most `max_commits` number of commits will be scanned from the latest
    /// unindexed commit.
    #[tracing::instrument(skip(self, store, progress_callback))]
    pub async fn build_changed_path_index_at_operation(
        &self,
        op_id: &OperationId,
        store: &Arc<Store>,
        max_commits: u32,
        mut progress_callback: impl FnMut(&DefaultChangedPathIndexProgress),
    ) -> Result<DefaultReadonlyIndex, DefaultIndexStoreError> {
        let field_lengths = FieldLengths {
            commit_id: store.commit_id_length(),
            change_id: store.change_id_length(),
        };
        let index = self.load_index_at_operation(op_id, field_lengths).await?;
        let old_changed_paths = index.changed_paths();

        // Distribute max_commits to contiguous pre/post ranges:
        //   ..|pre|old_changed_paths|post|
        //   (where pre.len() + post.len() <= max_commits)
        let pre_start;
        let pre_end;
        let post_start;
        let post_end;
        if let Some(GlobalCommitPosition(pos)) = old_changed_paths.start_commit_pos() {
            post_start = pos + old_changed_paths.num_commits();
            assert!(post_start <= index.num_commits());
            post_end = u32::saturating_add(post_start, max_commits).min(index.num_commits());
            pre_start = u32::saturating_sub(pos, max_commits - (post_end - post_start));
            pre_end = pos;
        } else {
            pre_start = u32::saturating_sub(index.num_commits(), max_commits);
            pre_end = index.num_commits();
            post_start = pre_end;
            post_end = pre_end;
        }

        let mut progress = DefaultChangedPathIndexProgress {
            current: 0,
            total: (pre_end - pre_start) + (post_end - post_start),
        };
        let mut emit_progress = || {
            progress_callback(&progress);
            progress.current += 1;
        };

        let to_index_err = |source| DefaultIndexStoreError::IndexCommits {
            op_id: op_id.clone(),
            source,
        };
        let index_commit = async |changed_paths: &mut CompositeChangedPathIndex,
                                  pos: GlobalCommitPosition| {
            assert_eq!(changed_paths.next_mutable_commit_pos(), Some(pos));
            let commit_id = index.as_composite().commits().entry_by_pos(pos).commit_id();
            let commit = store.get_commit_async(&commit_id).await?;
            let paths = collect_changed_paths(&index, &commit).await?;
            changed_paths.add_changed_paths(paths);
            Ok(())
        };

        // Index pre range
        let mut new_changed_paths =
            CompositeChangedPathIndex::empty(GlobalCommitPosition(pre_start));
        new_changed_paths.make_mutable();
        tracing::info!(?pre_start, ?pre_end, "indexing changed paths in commits");
        for pos in (pre_start..pre_end).map(GlobalCommitPosition) {
            emit_progress();
            index_commit(&mut new_changed_paths, pos)
                .await
                .map_err(to_index_err)?;
        }
        new_changed_paths
            .save_to_storage(self.storage.as_ref())
            .await
            .map_err(DefaultIndexStoreError::SaveIndex)?;

        // Copy previously-indexed segments
        new_changed_paths.append_segments(old_changed_paths);

        // Index post range, which is usually empty
        new_changed_paths.make_mutable();
        tracing::info!(?post_start, ?post_end, "indexing changed paths in commits");
        for pos in (post_start..post_end).map(GlobalCommitPosition) {
            emit_progress();
            index_commit(&mut new_changed_paths, pos)
                .await
                .map_err(to_index_err)?;
        }
        new_changed_paths.maybe_squash_with_ancestors();
        new_changed_paths
            .save_to_storage(self.storage.as_ref())
            .await
            .map_err(DefaultIndexStoreError::SaveIndex)?;

        // Update the operation link to point to the new segments
        let commits = index.readonly_commits().clone();
        let index = DefaultReadonlyIndex::from_segment(commits, new_changed_paths);
        self.associate_index_with_operation(&index, op_id).await?;
        emit_progress();
        Ok(index)
    }

    async fn save_mutable_index(
        &self,
        index: DefaultMutableIndex,
        op_id: &OperationId,
    ) -> Result<DefaultReadonlyIndex, DefaultIndexStoreError> {
        let (commits, mut changed_paths) = index.into_segment();
        let commits = commits
            .maybe_squash_with_ancestors()
            .save_to_storage(self.storage.as_ref())
            .await
            .map_err(DefaultIndexStoreError::SaveIndex)?;
        changed_paths.maybe_squash_with_ancestors();
        changed_paths
            .save_to_storage(self.storage.as_ref())
            .await
            .map_err(DefaultIndexStoreError::SaveIndex)?;
        let index = DefaultReadonlyIndex::from_segment(commits, changed_paths);
        self.associate_index_with_operation(&index, op_id).await?;
        Ok(index)
    }

    /// Records a link from the given operation to the this index version.
    async fn associate_index_with_operation(
        &self,
        index: &DefaultReadonlyIndex,
        op_id: &OperationId,
    ) -> Result<(), DefaultIndexStoreError> {
        let proto = crate::protos::default_index::SegmentControl {
            commit_segment_id: index.readonly_commits().id().to_bytes(),
            changed_path_start_commit_pos: index
                .changed_paths()
                .start_commit_pos()
                .map(|GlobalCommitPosition(start)| start),
            changed_path_segment_ids: index
                .changed_paths()
                .readonly_segments()
                .iter()
                .map(|segment| segment.id().to_bytes())
                .collect(),
        };
        self.storage
            .write_operation_link(op_id, &proto.encode_to_vec())
            .await
            .map_err(|source| DefaultIndexStoreError::AssociateIndex {
                op_id: op_id.to_owned(),
                source,
            })?;
        Ok(())
    }
}

#[async_trait(?Send)]
impl IndexStore for DefaultIndexStore {
    fn name(&self) -> &str {
        Self::name()
    }

    async fn get_index_at_op(
        &self,
        op: &Operation,
        store: &Arc<Store>,
    ) -> IndexStoreResult<Box<dyn ReadonlyIndex>> {
        let field_lengths = FieldLengths {
            commit_id: store.commit_id_length(),
            change_id: store.change_id_length(),
        };
        let index = match self.load_index_at_operation(op.id(), field_lengths).await {
            Err(DefaultIndexStoreError::IndexNotAssociated { .. }) => {
                self.build_index_at_operation(op, store).await
            }
            Err(DefaultIndexStoreError::LoadIndex(err)) if err.is_corrupt_or_not_found() => {
                // If the index was corrupt (maybe it was written in a different format),
                // we just reindex.
                match &err {
                    ReadonlyIndexLoadError::UnexpectedVersion {
                        kind,
                        found_version,
                        expected_version,
                    } => {
                        eprintln!(
                            "Found {kind} index format version {found_version}, expected version \
                             {expected_version}. Reindexing..."
                        );
                    }
                    ReadonlyIndexLoadError::Missing { .. } => {
                        eprintln!("{err}. Reindexing...");
                    }
                    ReadonlyIndexLoadError::Storage { error, .. } => {
                        eprintln!("{err}: {error}. Reindexing...");
                    }
                    ReadonlyIndexLoadError::Other { error, .. } => {
                        eprintln!("{err} (maybe the format has changed): {error}. Reindexing...");
                    }
                }
                self.reinit()
                    .await
                    .map_err(|err| IndexStoreError::Read(err.into()))?;
                self.build_index_at_operation(op, store).await
            }
            result => result,
        }
        .map_err(|err| IndexStoreError::Read(err.into()))?;
        Ok(Box::new(index))
    }

    async fn write_index(
        &self,
        index: Box<dyn MutableIndex>,
        op: &Operation,
    ) -> IndexStoreResult<Box<dyn ReadonlyIndex>> {
        let index: Box<DefaultMutableIndex> = index
            .downcast()
            .expect("index to merge in must be a DefaultMutableIndex");
        let index = self
            .save_mutable_index(*index, op.id())
            .await
            .map_err(|err| IndexStoreError::Write(err.into()))?;
        Ok(Box::new(index))
    }
}

/// Progress of [`DefaultIndexStore::build_changed_path_index_at_operation()`].
#[derive(Clone, Debug)]
pub struct DefaultChangedPathIndexProgress {
    pub current: u32,
    pub total: u32,
}
