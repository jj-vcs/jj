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
use std::collections::HashSet;
use std::fs;
use std::io;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::slice;
use std::sync::Arc;

use itertools::Itertools as _;
use pollster::FutureExt as _;
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
use crate::dag_walk;
use crate::file_util;
use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;
use crate::file_util::persist_temp_file;
use crate::index::Index as _;
use crate::index::IndexStore;
use crate::index::IndexStoreError;
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

#[derive(Debug, Error)]
pub enum DefaultIndexStoreError {
    #[error("Failed to associate index files with an operation {op_id}")]
    AssociateIndex {
        op_id: OperationId,
        source: PathError,
    },
    #[error("Failed to load associated index file names")]
    LoadAssociation(#[source] PathError),
    #[error(transparent)]
    LoadIndex(ReadonlyIndexLoadError),
    #[error("Failed to write index file")]
    SaveIndex(#[source] PathError),
    #[error("Failed to index commits at operation {op_id}")]
    IndexCommits {
        op_id: OperationId,
        source: BackendError,
    },
    #[error(transparent)]
    OpStore(#[from] OpStoreError),
}

#[derive(Debug)]
pub struct DefaultIndexStore {
    dir: PathBuf,
}

impl DefaultIndexStore {
    pub fn name() -> &'static str {
        "default"
    }

    pub fn init(dir: &Path) -> Result<Self, DefaultIndexStoreInitError> {
        let store = Self {
            dir: dir.to_owned(),
        };
        store.ensure_base_dirs()?;
        Ok(store)
    }

    pub fn load(dir: &Path) -> Self {
        Self {
            dir: dir.to_owned(),
        }
    }

    pub fn reinit(&self) -> Result<(), DefaultIndexStoreInitError> {
        // Create base directories in case the store was initialized by old jj.
        self.ensure_base_dirs()?;
        // Remove all operation links to trigger rebuilding.
        file_util::remove_dir_contents(&self.op_links_dir())?;
        file_util::remove_dir_contents(&self.legacy_operations_dir())?;
        // Remove index segments to save disk space. If raced, new segment file
        // will be created by the other process.
        file_util::remove_dir_contents(&self.commit_segments_dir())?;
        file_util::remove_dir_contents(&self.changed_path_segments_dir())?;
        // jj <= 0.14 created segment files in the top directory
        for entry in self.dir.read_dir().context(&self.dir)? {
            let entry = entry.context(&self.dir)?;
            let path = entry.path();
            if path.file_name().unwrap().len() != SEGMENT_FILE_NAME_LENGTH {
                // Skip "type" file, "operations" directory, etc.
                continue;
            }
            fs::remove_file(&path).context(&path)?;
        }
        Ok(())
    }

    fn ensure_base_dirs(&self) -> Result<(), PathError> {
        for dir in [
            self.op_links_dir(),
            self.legacy_operations_dir(),
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

    /// Directory for mapping from operations to commit segments. (jj < 0.33)
    fn legacy_operations_dir(&self) -> PathBuf {
        self.dir.join("operations")
    }

    /// Directory for commit segment files.
    fn commit_segments_dir(&self) -> PathBuf {
        self.dir.join("segments")
    }

    /// Directory for changed-path segment files.
    fn changed_path_segments_dir(&self) -> PathBuf {
        self.dir.join("changed_paths")
    }

    fn load_index_at_operation(
        &self,
        op_id: &OperationId,
        lengths: FieldLengths,
    ) -> Result<DefaultReadonlyIndex, DefaultIndexStoreError> {
        let commit_segment_id;
        let changed_path_start_commit_pos;
        let changed_path_segment_ids;
        let op_link_file = self.op_links_dir().join(op_id.hex());
        match fs::read(&op_link_file).context(&op_link_file) {
            Ok(data) => {
                let proto = crate::protos::default_index::SegmentControl::decode(&*data)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
                    .context(&op_link_file)
                    .map_err(DefaultIndexStoreError::LoadAssociation)?;
                commit_segment_id = CommitIndexSegmentId::new(proto.commit_segment_id);
                changed_path_start_commit_pos = proto
                    .changed_path_start_commit_pos
                    .map(GlobalCommitPosition);
                changed_path_segment_ids = proto
                    .changed_path_segment_ids
                    .into_iter()
                    .map(ChangedPathIndexSegmentId::new)
                    .collect_vec();
            }
            // TODO: drop support for legacy operation link file in jj 0.39 or so
            Err(PathError { source: error, .. }) if error.kind() == io::ErrorKind::NotFound => {
                let op_id_file = self.legacy_operations_dir().join(op_id.hex());
                let data = fs::read(&op_id_file)
                    .context(&op_id_file)
                    .map_err(DefaultIndexStoreError::LoadAssociation)?;
                commit_segment_id = CommitIndexSegmentId::try_from_hex(&data)
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "file name is not valid hex")
                    })
                    .context(&op_id_file)
                    .map_err(DefaultIndexStoreError::LoadAssociation)?;
                changed_path_start_commit_pos = None;
                changed_path_segment_ids = vec![];
            }
            Err(err) => return Err(DefaultIndexStoreError::LoadAssociation(err)),
        };

        let commits = ReadonlyCommitIndexSegment::load(
            &self.commit_segments_dir(),
            commit_segment_id,
            lengths,
        )
        .map_err(DefaultIndexStoreError::LoadIndex)?;
        // TODO: lazy load or mmap?
        let changed_paths = if let Some(start_commit_pos) = changed_path_start_commit_pos {
            CompositeChangedPathIndex::load(
                &self.changed_path_segments_dir(),
                start_commit_pos,
                &changed_path_segment_ids,
            )
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
        let op_links_dir = self.op_links_dir();
        let legacy_operations_dir = self.legacy_operations_dir();
        let field_lengths = FieldLengths {
            commit_id: store.commit_id_length(),
            change_id: store.change_id_length(),
        };
        // Pick the latest existing ancestor operation as the parent segment.
        let mut unindexed_ops = Vec::new();
        let mut parent_op = None;
        for op in op_walk::walk_ancestors(slice::from_ref(operation)) {
            let op = op?;
            if op_links_dir.join(op.id().hex()).is_file()
                || legacy_operations_dir.join(op.id().hex()).is_file()
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
                .try_collect()?
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
                op.view()?.all_referenced_commit_ids(),
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
                let parent_index = self.load_index_at_operation(op.id(), field_lengths)?;
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
                .is_some_and(|index| index.has_id(id))
        };
        let get_commit_with_op = |commit_id: &CommitId, op_id: &OperationId| {
            let op_id = op_id.clone();
            match store.get_commit(commit_id) {
                // Propagate head's op_id to report possible source of an error.
                // The op_id doesn't have to be included in the sort key, but
                // that wouldn't matter since the commit should be unique.
                Ok(commit) => Ok((CommitByCommitterTimestamp(commit), op_id)),
                Err(source) => Err(DefaultIndexStoreError::IndexCommits { op_id, source }),
            }
        };
        // Retain immediate predecessors if legacy operation exists. Some
        // commands (e.g. squash into grandparent) may leave transitive
        // predecessors, which aren't visible to any views.
        // TODO: delete this workaround with commit.predecessors.
        let commits_to_keep_immediate_predecessors = if ops_to_visit
            .iter()
            .any(|op| !op.stores_commit_predecessors())
        {
            let mut ancestors = HashSet::new();
            let mut work = historical_heads.keys().cloned().collect_vec();
            while let Some(commit_id) = work.pop() {
                if ancestors.contains(&commit_id) || parent_index_has_id(&commit_id) {
                    continue;
                }
                if let Ok(commit) = store.get_commit(&commit_id) {
                    work.extend(commit.parent_ids().iter().cloned());
                }
                ancestors.insert(commit_id);
            }
            ancestors
        } else {
            HashSet::new()
        };
        let commits = dag_walk::topo_order_reverse_ord_ok(
            historical_heads
                .iter()
                .filter(|&(commit_id, _)| !parent_index_has_id(commit_id))
                .map(|(commit_id, op_id)| get_commit_with_op(commit_id, op_id)),
            |(CommitByCommitterTimestamp(commit), _)| commit.id().clone(),
            |(CommitByCommitterTimestamp(commit), op_id)| {
                let keep_predecessors =
                    commits_to_keep_immediate_predecessors.contains(commit.id());
                itertools::chain(
                    commit.parent_ids(),
                    keep_predecessors
                        .then_some(&commit.store_commit().predecessors)
                        .into_iter()
                        .flatten(),
                )
                .filter(|&id| !parent_index_has_id(id))
                .map(|commit_id| get_commit_with_op(commit_id, op_id))
                .collect_vec()
            },
            |_| panic!("graph has cycle"),
        )?;
        for (CommitByCommitterTimestamp(commit), op_id) in commits.iter().rev() {
            mutable_index.add_commit(commit).await.map_err(|source| {
                DefaultIndexStoreError::IndexCommits {
                    op_id: op_id.clone(),
                    source,
                }
            })?;
        }

        let index = self.save_mutable_index(mutable_index, operation.id())?;
        tracing::info!(?index, commits_count = commits.len(), "saved new index");

        Ok(index)
    }

    /// Builds changed-path index for the specified operation.
    ///
    /// At most `max_commits` number of commits will be scanned from the latest
    /// unindexed commit.
    #[tracing::instrument(skip(self, store))]
    pub async fn build_changed_path_index_at_operation(
        &self,
        op_id: &OperationId,
        store: &Arc<Store>,
        max_commits: u32,
        // TODO: add progress callback?
    ) -> Result<DefaultReadonlyIndex, DefaultIndexStoreError> {
        // Create directories in case the store was initialized by jj < 0.33.
        self.ensure_base_dirs()
            .map_err(DefaultIndexStoreError::SaveIndex)?;
        let field_lengths = FieldLengths {
            commit_id: store.commit_id_length(),
            change_id: store.change_id_length(),
        };
        let index = self.load_index_at_operation(op_id, field_lengths)?;
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
            index_commit(&mut new_changed_paths, pos)
                .await
                .map_err(to_index_err)?;
        }
        new_changed_paths
            .save_in(&self.changed_path_segments_dir())
            .map_err(DefaultIndexStoreError::SaveIndex)?;

        // Copy previously-indexed segments
        new_changed_paths.append_segments(old_changed_paths);

        // Index post range, which is usually empty
        new_changed_paths.make_mutable();
        tracing::info!(?post_start, ?post_end, "indexing changed paths in commits");
        for pos in (post_start..post_end).map(GlobalCommitPosition) {
            index_commit(&mut new_changed_paths, pos)
                .await
                .map_err(to_index_err)?;
        }
        new_changed_paths.maybe_squash_with_ancestors();
        new_changed_paths
            .save_in(&self.changed_path_segments_dir())
            .map_err(DefaultIndexStoreError::SaveIndex)?;

        // Update the operation link to point to the new segments
        let commits = index.readonly_commits().clone();
        let index = DefaultReadonlyIndex::from_segment(commits, new_changed_paths);
        self.associate_index_with_operation(&index, op_id)
            .map_err(|source| DefaultIndexStoreError::AssociateIndex {
                op_id: op_id.to_owned(),
                source,
            })?;
        Ok(index)
    }

    fn save_mutable_index(
        &self,
        index: DefaultMutableIndex,
        op_id: &OperationId,
    ) -> Result<DefaultReadonlyIndex, DefaultIndexStoreError> {
        // Create directories in case the store was initialized by jj < 0.33.
        self.ensure_base_dirs()
            .map_err(DefaultIndexStoreError::SaveIndex)?;
        let (commits, mut changed_paths) = index.into_segment();
        let commits = commits
            .maybe_squash_with_ancestors()
            .save_in(&self.commit_segments_dir())
            .map_err(DefaultIndexStoreError::SaveIndex)?;
        changed_paths.maybe_squash_with_ancestors();
        changed_paths
            .save_in(&self.changed_path_segments_dir())
            .map_err(DefaultIndexStoreError::SaveIndex)?;
        let index = DefaultReadonlyIndex::from_segment(commits, changed_paths);
        self.associate_index_with_operation(&index, op_id)
            .map_err(|source| DefaultIndexStoreError::AssociateIndex {
                op_id: op_id.to_owned(),
                source,
            })?;
        Ok(index)
    }

    /// Records a link from the given operation to the this index version.
    fn associate_index_with_operation(
        &self,
        index: &DefaultReadonlyIndex,
        op_id: &OperationId,
    ) -> Result<(), PathError> {
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
        let dir = self.op_links_dir();
        let mut temp_file = NamedTempFile::new_in(&dir).context(&dir)?;
        let file = temp_file.as_file_mut();
        file.write_all(&proto.encode_to_vec())
            .context(temp_file.path())?;
        let path = dir.join(op_id.hex());
        persist_temp_file(temp_file, &path).context(&path)?;

        // TODO: drop support for legacy operation link file in jj 0.39 or so
        let dir = self.legacy_operations_dir();
        let mut temp_file = NamedTempFile::new_in(&dir).context(&dir)?;
        let file = temp_file.as_file_mut();
        file.write_all(index.readonly_commits().id().hex().as_bytes())
            .context(temp_file.path())?;
        let path = dir.join(op_id.hex());
        persist_temp_file(temp_file, &path).context(&path)?;
        Ok(())
    }
}

impl IndexStore for DefaultIndexStore {
    fn name(&self) -> &str {
        Self::name()
    }

    fn get_index_at_op(
        &self,
        op: &Operation,
        store: &Arc<Store>,
    ) -> Result<Box<dyn ReadonlyIndex>, IndexStoreError> {
        let field_lengths = FieldLengths {
            commit_id: store.commit_id_length(),
            change_id: store.change_id_length(),
        };
        let index = match self.load_index_at_operation(op.id(), field_lengths) {
            Err(DefaultIndexStoreError::LoadAssociation(PathError { source: error, .. }))
                if error.kind() == io::ErrorKind::NotFound =>
            {
                self.build_index_at_operation(op, store).block_on()
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
                    ReadonlyIndexLoadError::Other { error, .. } => {
                        eprintln!("{err} (maybe the format has changed): {error}. Reindexing...");
                    }
                }
                self.reinit()
                    .map_err(|err| IndexStoreError::Read(err.into()))?;
                self.build_index_at_operation(op, store).block_on()
            }
            result => result,
        }
        .map_err(|err| IndexStoreError::Read(err.into()))?;
        Ok(Box::new(index))
    }

    fn write_index(
        &self,
        index: Box<dyn MutableIndex>,
        op: &Operation,
    ) -> Result<Box<dyn ReadonlyIndex>, IndexStoreError> {
        let index: Box<DefaultMutableIndex> = index
            .downcast()
            .expect("index to merge in must be a DefaultMutableIndex");
        let index = self
            .save_mutable_index(*index, op.id())
            .map_err(|err| IndexStoreError::Write(err.into()))?;
        Ok(Box::new(index))
    }
}
