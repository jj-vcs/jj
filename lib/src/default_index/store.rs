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

#![allow(missing_docs)]

use std::any::Any;
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
use tempfile::NamedTempFile;
use thiserror::Error;

use super::mutable::DefaultMutableIndex;
use super::readonly::DefaultReadonlyIndex;
use super::readonly::FieldLengths;
use super::readonly::ReadonlyIndexLoadError;
use super::readonly::ReadonlyIndexSegment;
use crate::backend::BackendError;
use crate::backend::BackendInitError;
use crate::backend::CommitId;
use crate::commit::CommitByCommitterTimestamp;
use crate::dag_walk;
use crate::file_util;
use crate::file_util::persist_content_addressed_temp_file;
use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;
use crate::index::IndexReadError;
use crate::index::IndexStore;
use crate::index::IndexWriteError;
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
        BackendInitError(err.into())
    }
}

#[derive(Debug, Error)]
pub enum DefaultIndexStoreError {
    #[error("Failed to associate commit index file with an operation {op_id}")]
    AssociateIndex {
        op_id: OperationId,
        source: io::Error,
    },
    #[error("Failed to load associated commit index file name")]
    LoadAssociation(#[source] io::Error),
    #[error(transparent)]
    LoadIndex(ReadonlyIndexLoadError),
    #[error("Failed to write commit index file")]
    SaveIndex(#[source] io::Error),
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
        let store = DefaultIndexStore {
            dir: dir.to_owned(),
        };
        store.ensure_base_dirs()?;
        Ok(store)
    }

    pub fn load(dir: &Path) -> DefaultIndexStore {
        DefaultIndexStore {
            dir: dir.to_owned(),
        }
    }

    pub fn reinit(&self) -> Result<(), DefaultIndexStoreInitError> {
        // Create base directories in case the store was initialized by old jj.
        self.ensure_base_dirs()?;
        // Remove all operation links to trigger rebuilding.
        file_util::remove_dir_contents(&self.operations_dir())?;
        // Remove index segments to save disk space. If raced, new segment file
        // will be created by the other process.
        file_util::remove_dir_contents(&self.segments_dir())?;
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
        for dir in [self.operations_dir(), self.segments_dir()] {
            file_util::create_or_reuse_dir(&dir).context(&dir)?;
        }
        Ok(())
    }

    fn operations_dir(&self) -> PathBuf {
        self.dir.join("operations")
    }

    fn segments_dir(&self) -> PathBuf {
        self.dir.join("segments")
    }

    fn load_index_segments_at_operation(
        &self,
        op_id: &OperationId,
        lengths: FieldLengths,
    ) -> Result<Arc<ReadonlyIndexSegment>, DefaultIndexStoreError> {
        let op_id_file = self.operations_dir().join(op_id.hex());
        let index_file_id_hex =
            fs::read_to_string(op_id_file).map_err(DefaultIndexStoreError::LoadAssociation)?;
        ReadonlyIndexSegment::load(&self.segments_dir(), index_file_id_hex, lengths)
            .map_err(DefaultIndexStoreError::LoadIndex)
    }

    /// Rebuilds index for the given `operation`.
    ///
    /// The index to be built will be calculated from one of the ancestor
    /// operations if exists. Use `reinit()` to rebuild index from scratch.
    pub fn build_index_at_operation(
        &self,
        operation: &Operation,
        store: &Arc<Store>,
    ) -> Result<DefaultReadonlyIndex, DefaultIndexStoreError> {
        let index_segment = self.build_index_segments_at_operation(operation, store)?;
        Ok(DefaultReadonlyIndex::from_segment(index_segment))
    }

    #[tracing::instrument(skip(self, store))]
    fn build_index_segments_at_operation(
        &self,
        operation: &Operation,
        store: &Arc<Store>,
    ) -> Result<Arc<ReadonlyIndexSegment>, DefaultIndexStoreError> {
        tracing::info!("scanning operations to index");
        let operations_dir = self.operations_dir();
        let field_lengths = FieldLengths {
            commit_id: store.commit_id_length(),
            change_id: store.change_id_length(),
        };
        // Pick the latest existing ancestor operation as the parent segment.
        let mut unindexed_ops = Vec::new();
        let mut parent_op = None;
        for op in op_walk::walk_ancestors(slice::from_ref(operation)) {
            let op = op?;
            if operations_dir.join(op.id().hex()).is_file() {
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
        let maybe_parent_file;
        let mut mutable_index;
        match &parent_op {
            None => {
                maybe_parent_file = None;
                mutable_index = DefaultMutableIndex::full(field_lengths);
            }
            Some(op) => {
                let parent_file = self.load_index_segments_at_operation(op.id(), field_lengths)?;
                maybe_parent_file = Some(parent_file.clone());
                mutable_index = DefaultMutableIndex::incremental(parent_file);
            }
        }

        tracing::info!(
            ?maybe_parent_file,
            heads_count = historical_heads.len(),
            "indexing commits reachable from historical heads"
        );
        // Build a list of ancestors of heads where parents come after the
        // commit itself.
        let parent_file_has_id = |id: &CommitId| {
            maybe_parent_file
                .as_ref()
                .is_some_and(|segment| segment.as_composite().has_id(id))
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
                if ancestors.contains(&commit_id) || parent_file_has_id(&commit_id) {
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
                .filter(|&(commit_id, _)| !parent_file_has_id(commit_id))
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
                .filter(|&id| !parent_file_has_id(id))
                .map(|commit_id| get_commit_with_op(commit_id, op_id))
                .collect_vec()
            },
            |_| panic!("graph has cycle"),
        )?;
        for (CommitByCommitterTimestamp(commit), _) in commits.iter().rev() {
            mutable_index.add_commit(commit);
        }

        let index_file = self.save_mutable_index(mutable_index, operation.id())?;
        tracing::info!(
            ?index_file,
            commits_count = commits.len(),
            "saved new index file"
        );

        Ok(index_file)
    }

    fn save_mutable_index(
        &self,
        mutable_index: DefaultMutableIndex,
        op_id: &OperationId,
    ) -> Result<Arc<ReadonlyIndexSegment>, DefaultIndexStoreError> {
        let index_segment = mutable_index
            .squash_and_save_in(&self.segments_dir())
            .map_err(DefaultIndexStoreError::SaveIndex)?;
        self.associate_file_with_operation(&index_segment, op_id)
            .map_err(|source| DefaultIndexStoreError::AssociateIndex {
                op_id: op_id.to_owned(),
                source,
            })?;
        Ok(index_segment)
    }

    /// Records a link from the given operation to the this index version.
    fn associate_file_with_operation(
        &self,
        index: &ReadonlyIndexSegment,
        op_id: &OperationId,
    ) -> io::Result<()> {
        let dir = self.operations_dir();
        let mut temp_file = NamedTempFile::new_in(&dir)?;
        let file = temp_file.as_file_mut();
        file.write_all(index.name().as_bytes())?;
        persist_content_addressed_temp_file(temp_file, dir.join(op_id.hex()))?;
        Ok(())
    }
}

impl IndexStore for DefaultIndexStore {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn name(&self) -> &str {
        Self::name()
    }

    fn get_index_at_op(
        &self,
        op: &Operation,
        store: &Arc<Store>,
    ) -> Result<Box<dyn ReadonlyIndex>, IndexReadError> {
        let field_lengths = FieldLengths {
            commit_id: store.commit_id_length(),
            change_id: store.change_id_length(),
        };
        let index_segment = match self.load_index_segments_at_operation(op.id(), field_lengths) {
            Err(DefaultIndexStoreError::LoadAssociation(err))
                if err.kind() == io::ErrorKind::NotFound =>
            {
                self.build_index_segments_at_operation(op, store)
            }
            Err(DefaultIndexStoreError::LoadIndex(err)) if err.is_corrupt_or_not_found() => {
                // If the index was corrupt (maybe it was written in a different format),
                // we just reindex.
                match &err {
                    ReadonlyIndexLoadError::UnexpectedVersion {
                        found_version,
                        expected_version,
                    } => {
                        eprintln!(
                            "Found index format version {found_version}, expected version \
                             {expected_version}. Reindexing..."
                        );
                    }
                    ReadonlyIndexLoadError::Other { name: _, error } => {
                        eprintln!("{err} (maybe the format has changed): {error}. Reindexing...");
                    }
                }
                self.reinit().map_err(|err| IndexReadError(err.into()))?;
                self.build_index_segments_at_operation(op, store)
            }
            result => result,
        }
        .map_err(|err| IndexReadError(err.into()))?;
        Ok(Box::new(DefaultReadonlyIndex::from_segment(index_segment)))
    }

    fn write_index(
        &self,
        index: Box<dyn MutableIndex>,
        op: &Operation,
    ) -> Result<Box<dyn ReadonlyIndex>, IndexWriteError> {
        let index = index
            .into_any()
            .downcast::<DefaultMutableIndex>()
            .expect("index to merge in must be a DefaultMutableIndex");
        let index_segment = self
            .save_mutable_index(*index, op.id())
            .map_err(|err| IndexWriteError(err.into()))?;
        Ok(Box::new(DefaultReadonlyIndex::from_segment(index_segment)))
    }
}
