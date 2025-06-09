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

use std::any::Any;
use std::collections::HashMap;
use std::fmt::Debug;
use std::fmt::Error;
use std::fmt::Formatter;
use std::io::Cursor;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::time::SystemTime;

use async_trait::async_trait;
use futures::stream;
use futures::stream::BoxStream;
use jj_lib::backend::make_root_commit;
use jj_lib::backend::Backend;
use jj_lib::backend::BackendError;
use jj_lib::backend::BackendResult;
use jj_lib::backend::ChangeId;
use jj_lib::backend::Commit;
use jj_lib::backend::CommitId;
use jj_lib::backend::Conflict;
use jj_lib::backend::ConflictId;
use jj_lib::backend::CopyHistory;
use jj_lib::backend::CopyId;
use jj_lib::backend::CopyRecord;
use jj_lib::backend::FileId;
use jj_lib::backend::SecureSig;
use jj_lib::backend::SigningFn;
use jj_lib::backend::SymlinkId;
use jj_lib::backend::Tree;
use jj_lib::backend::TreeId;
use jj_lib::dag_walk::topo_order_reverse;
use jj_lib::index::Index;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt as _;

const HASH_LENGTH: usize = 10;
const CHANGE_ID_LENGTH: usize = 16;

// Keyed by canonical store path. Since we just use the path as a key, we can't
// rely on on the file system to resolve two different uncanonicalized paths to
// the same real path (as we would if we just used the path with `std::fs`
// functions).
type TestBackendDataMap = HashMap<PathBuf, Arc<Mutex<TestBackendData>>>;

#[derive(Default)]
pub struct TestBackendData {
    commits: HashMap<CommitId, Commit>,
    trees: HashMap<RepoPathBuf, HashMap<TreeId, Tree>>,
    files: HashMap<RepoPathBuf, HashMap<FileId, Vec<u8>>>,
    symlinks: HashMap<RepoPathBuf, HashMap<SymlinkId, String>>,
    conflicts: HashMap<RepoPathBuf, HashMap<ConflictId, Conflict>>,
    copies: HashMap<CopyId, CopyHistory>,
}

#[derive(Clone, Default)]
pub struct TestBackendFactory {
    backend_data: Arc<Mutex<TestBackendDataMap>>,
}

impl TestBackendFactory {
    pub fn init(&self, store_path: &Path) -> TestBackend {
        let data = Arc::new(Mutex::new(TestBackendData::default()));
        self.backend_data
            .lock()
            .unwrap()
            .insert(store_path.canonicalize().unwrap(), data.clone());
        TestBackend::with_data(data)
    }

    pub fn load(&self, store_path: &Path) -> TestBackend {
        let data = self
            .backend_data
            .lock()
            .unwrap()
            .get(&store_path.canonicalize().unwrap())
            .unwrap()
            .clone();
        TestBackend::with_data(data)
    }
}

impl Debug for TestBackendFactory {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        f.debug_struct("TestBackendFactory").finish_non_exhaustive()
    }
}

fn get_hash(content: &(impl jj_lib::content_hash::ContentHash + ?Sized)) -> Vec<u8> {
    jj_lib::content_hash::blake2b_hash(content).as_slice()[..HASH_LENGTH].to_vec()
}

/// A commit backend for use in tests.
///
/// It's meant to be strict, in order to catch bugs where we make the
/// wrong assumptions. For example, unlike both `GitBackend` and
/// `SimpleBackend`, this backend doesn't share objects written to
/// different paths (writing a file with contents X to path A will not
/// make it possible to read that contents from path B given the same
/// `FileId`).
pub struct TestBackend {
    root_commit_id: CommitId,
    root_change_id: ChangeId,
    empty_tree_id: TreeId,
    data: Arc<Mutex<TestBackendData>>,
}

impl TestBackend {
    pub fn with_data(data: Arc<Mutex<TestBackendData>>) -> Self {
        let root_commit_id = CommitId::from_bytes(&[0; HASH_LENGTH]);
        let root_change_id = ChangeId::from_bytes(&[0; CHANGE_ID_LENGTH]);
        let empty_tree_id = TreeId::new(get_hash(&Tree::default()));
        TestBackend {
            root_commit_id,
            root_change_id,
            empty_tree_id,
            data,
        }
    }

    fn locked_data(&self) -> MutexGuard<'_, TestBackendData> {
        self.data.lock().unwrap()
    }

    pub fn remove_commit_unchecked(&self, id: &CommitId) {
        self.locked_data().commits.remove(id);
    }
}

impl Debug for TestBackend {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        f.debug_struct("TestBackend").finish_non_exhaustive()
    }
}

#[async_trait]
impl Backend for TestBackend {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn name(&self) -> &str {
        "test"
    }

    fn commit_id_length(&self) -> usize {
        HASH_LENGTH
    }

    fn change_id_length(&self) -> usize {
        CHANGE_ID_LENGTH
    }

    fn root_commit_id(&self) -> &CommitId {
        &self.root_commit_id
    }

    fn root_change_id(&self) -> &ChangeId {
        &self.root_change_id
    }

    fn empty_tree_id(&self) -> &TreeId {
        &self.empty_tree_id
    }

    fn concurrency(&self) -> usize {
        // Not optimal, just for testing the async code more
        10
    }

    async fn read_file(
        &self,
        path: &RepoPath,
        id: &FileId,
    ) -> BackendResult<Pin<Box<dyn AsyncRead + Send>>> {
        match self
            .locked_data()
            .files
            .get(path)
            .and_then(|items| items.get(id))
            .cloned()
        {
            None => Err(BackendError::ObjectNotFound {
                object_type: "file".to_string(),
                hash: id.hex(),
                source: format!("at path {path:?}").into(),
            }),
            Some(contents) => Ok(Box::pin(Cursor::new(contents))),
        }
    }

    async fn write_file(
        &self,
        path: &RepoPath,
        contents: &mut (dyn AsyncRead + Send + Unpin),
    ) -> BackendResult<FileId> {
        let mut bytes = Vec::new();
        contents.read_to_end(&mut bytes).await.unwrap();
        let id = FileId::new(get_hash(&bytes));
        self.locked_data()
            .files
            .entry(path.to_owned())
            .or_default()
            .insert(id.clone(), bytes);
        Ok(id)
    }

    async fn read_symlink(&self, path: &RepoPath, id: &SymlinkId) -> BackendResult<String> {
        match self
            .locked_data()
            .symlinks
            .get(path)
            .and_then(|items| items.get(id))
            .cloned()
        {
            None => Err(BackendError::ObjectNotFound {
                object_type: "symlink".to_string(),
                hash: id.hex(),
                source: format!("at path {path:?}").into(),
            }),
            Some(target) => Ok(target),
        }
    }

    async fn write_symlink(&self, path: &RepoPath, target: &str) -> BackendResult<SymlinkId> {
        let id = SymlinkId::new(get_hash(target.as_bytes()));
        self.locked_data()
            .symlinks
            .entry(path.to_owned())
            .or_default()
            .insert(id.clone(), target.to_string());
        Ok(id)
    }

    async fn read_copy(&self, id: &CopyId) -> BackendResult<CopyHistory> {
        let copy = self.locked_data().copies.get(id).cloned().ok_or_else(|| {
            BackendError::ObjectNotFound {
                object_type: "copy".to_string(),
                hash: id.hex(),
                source: "".into(),
            }
        })?;
        Ok(copy)
    }

    async fn write_copy(&self, contents: &CopyHistory) -> BackendResult<CopyId> {
        let id = CopyId::new(get_hash(contents));
        self.locked_data()
            .copies
            .insert(id.clone(), contents.clone());
        Ok(id)
    }

    async fn get_related_copies(&self, copy_id: &CopyId) -> BackendResult<Vec<CopyHistory>> {
        let copies = &self.locked_data().copies;
        if !copies.contains_key(copy_id) {
            return Err(BackendError::ObjectNotFound {
                object_type: "copy history".to_string(),
                hash: copy_id.hex(),
                source: "".into(),
            });
        }
        // Return all copy histories to test that the caller correctly ignores histories
        // that are not relevant to the trees they're working with.
        let mut histories = vec![];
        for id in topo_order_reverse(
            copies.keys(),
            |id| *id,
            |id| copies.get(*id).unwrap().parents.iter(),
        ) {
            histories.push(copies.get(id).unwrap().clone());
        }
        Ok(histories)
    }

    async fn read_tree(&self, path: &RepoPath, id: &TreeId) -> BackendResult<Tree> {
        if id == &self.empty_tree_id {
            return Ok(Tree::default());
        }
        match self
            .locked_data()
            .trees
            .get(path)
            .and_then(|items| items.get(id))
            .cloned()
        {
            None => Err(BackendError::ObjectNotFound {
                object_type: "tree".to_string(),
                hash: id.hex(),
                source: format!("at path {path:?}").into(),
            }),
            Some(tree) => Ok(tree),
        }
    }

    async fn write_tree(&self, path: &RepoPath, contents: &Tree) -> BackendResult<TreeId> {
        let id = TreeId::new(get_hash(contents));
        self.locked_data()
            .trees
            .entry(path.to_owned())
            .or_default()
            .insert(id.clone(), contents.clone());
        Ok(id)
    }

    fn read_conflict(&self, path: &RepoPath, id: &ConflictId) -> BackendResult<Conflict> {
        match self
            .locked_data()
            .conflicts
            .get(path)
            .and_then(|items| items.get(id))
            .cloned()
        {
            None => Err(BackendError::ObjectNotFound {
                object_type: "conflict".to_string(),
                hash: id.hex(),
                source: format!("at path {path:?}").into(),
            }),
            Some(conflict) => Ok(conflict),
        }
    }

    fn write_conflict(&self, path: &RepoPath, contents: &Conflict) -> BackendResult<ConflictId> {
        let id = ConflictId::new(get_hash(contents));
        self.locked_data()
            .conflicts
            .entry(path.to_owned())
            .or_default()
            .insert(id.clone(), contents.clone());
        Ok(id)
    }

    async fn read_commit(&self, id: &CommitId) -> BackendResult<Commit> {
        if id == &self.root_commit_id {
            return Ok(make_root_commit(
                self.root_change_id.clone(),
                self.empty_tree_id.clone(),
            ));
        }
        match self.locked_data().commits.get(id).cloned() {
            None => Err(BackendError::ObjectNotFound {
                object_type: "commit".to_string(),
                hash: id.hex(),
                source: "".into(),
            }),
            Some(commit) => Ok(commit),
        }
    }

    async fn write_commit(
        &self,
        mut contents: Commit,
        mut sign_with: Option<&mut SigningFn>,
    ) -> BackendResult<(CommitId, Commit)> {
        assert!(contents.secure_sig.is_none(), "commit.secure_sig was set");

        if let Some(sign) = &mut sign_with {
            let data = format!("{contents:?}").into_bytes();
            let sig = sign(&data).map_err(|err| BackendError::Other(Box::new(err)))?;
            contents.secure_sig = Some(SecureSig { data, sig });
        }

        let id = CommitId::new(get_hash(&contents));
        self.locked_data()
            .commits
            .insert(id.clone(), contents.clone());
        Ok((id, contents))
    }

    fn get_copy_records(
        &self,
        _paths: Option<&[RepoPathBuf]>,
        _root: &CommitId,
        _head: &CommitId,
    ) -> BackendResult<BoxStream<BackendResult<CopyRecord>>> {
        Ok(Box::pin(stream::empty()))
    }

    fn gc(&self, _index: &dyn Index, _keep_newer: SystemTime) -> BackendResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use pollster::FutureExt as _;

    use super::*;
    use crate::repo_path_buf;

    fn copy_history(path: &str, parents: &[CopyId]) -> CopyHistory {
        CopyHistory {
            current_path: repo_path_buf(path),
            parents: parents.to_vec(),
            salt: vec![],
        }
    }

    #[test]
    fn get_related_copies() {
        let backend = TestBackend::with_data(Arc::new(Mutex::new(TestBackendData::default())));

        // Test with a single chain so the resulting order is deterministic
        let copy1 = copy_history("foo1", &[]);
        let copy1_id = backend.write_copy(&copy1).block_on().unwrap();
        let copy2 = copy_history("foo2", &[copy1_id.clone()]);
        let copy2_id = backend.write_copy(&copy2).block_on().unwrap();
        let copy3 = copy_history("foo3", &[copy2_id.clone()]);
        let copy3_id = backend.write_copy(&copy3).block_on().unwrap();

        // Error when looking up by non-existent id
        assert!(backend
            .get_related_copies(&CopyId::from_hex("abcd"))
            .block_on()
            .is_err());

        // Looking up by any id returns the related copies in the same order (children
        // before parents)
        let related = backend.get_related_copies(&copy1_id).block_on().unwrap();
        assert_eq!(related, vec![copy3.clone(), copy2.clone(), copy1.clone()]);
        let related: Vec<CopyHistory> = backend.get_related_copies(&copy3_id).block_on().unwrap();
        assert_eq!(related, vec![copy3.clone(), copy2.clone(), copy1.clone()]);
    }
}
