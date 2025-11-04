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

use std::fmt::Debug;
use std::fs;
use std::fs::File;
use std::io::Cursor;
use std::io::Read as _;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::SystemTime;

use async_trait::async_trait;
use blake2::Blake2b512;
use blake2::Digest as _;
use futures::stream;
use futures::stream::BoxStream;
use pollster::FutureExt as _;
use prost::Message as _;
use tempfile::NamedTempFile;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt as _;

use crate::backend::Backend;
use crate::backend::BackendError;
use crate::backend::BackendResult;
use crate::backend::ChangeId;
use crate::backend::Commit;
use crate::backend::CommitId;
use crate::backend::CopyHistory;
use crate::backend::CopyId;
use crate::backend::CopyRecord;
use crate::backend::FileId;
use crate::backend::MillisSinceEpoch;
use crate::backend::SecureSig;
use crate::backend::Signature;
use crate::backend::SigningFn;
use crate::backend::SymlinkId;
use crate::backend::Timestamp;
use crate::backend::Tree;
use crate::backend::TreeId;
use crate::backend::TreeValue;
use crate::backend::make_root_commit;
use crate::content_hash::blake2b_hash;
use crate::file_util::persist_content_addressed_temp_file;
use crate::index::Index;
use crate::merge::Merge;
use crate::merge::MergeBuilder;
use crate::object_id::ObjectId;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;
use crate::repo_path::RepoPathComponentBuf;

const COMMIT_ID_LENGTH: usize = 64;
const CHANGE_ID_LENGTH: usize = 16;

fn map_not_found_err(err: std::io::Error, id: &impl ObjectId) -> BackendError {
    if err.kind() == std::io::ErrorKind::NotFound {
        BackendError::ObjectNotFound {
            object_type: id.object_type(),
            hash: id.hex(),
            source: Box::new(err),
        }
    } else {
        BackendError::ReadObject {
            object_type: id.object_type(),
            hash: id.hex(),
            source: Box::new(err),
        }
    }
}

fn to_other_err(err: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> BackendError {
    BackendError::Other(err.into())
}

#[derive(Debug)]
pub struct SimpleBackend {
    path: PathBuf,
    root_commit_id: CommitId,
    root_change_id: ChangeId,
    empty_tree_id: TreeId,
}

impl SimpleBackend {
    pub fn name() -> &'static str {
        "Simple"
    }

    pub fn init(store_path: &Path) -> Self {
        fs::create_dir(store_path.join("commits")).unwrap();
        fs::create_dir(store_path.join("trees")).unwrap();
        fs::create_dir(store_path.join("files")).unwrap();
        fs::create_dir(store_path.join("symlinks")).unwrap();
        fs::create_dir(store_path.join("conflicts")).unwrap();
        let backend = Self::load(store_path);
        let empty_tree_id = backend
            .write_tree(RepoPath::root(), &Tree::default())
            .block_on()
            .unwrap();
        assert_eq!(empty_tree_id, backend.empty_tree_id);
        backend
    }

    pub fn load(store_path: &Path) -> Self {
        let root_commit_id = CommitId::from_bytes(&[0; COMMIT_ID_LENGTH]);
        let root_change_id = ChangeId::from_bytes(&[0; CHANGE_ID_LENGTH]);
        let empty_tree_id = TreeId::from_hex(
            "482ae5a29fbe856c7272f2071b8b0f0359ee2d89ff392b8a900643fbd0836eccd067b8bf41909e206c90d45d6e7d8b6686b93ecaee5fe1a9060d87b672101310",
        );
        Self {
            path: store_path.to_path_buf(),
            root_commit_id,
            root_change_id,
            empty_tree_id,
        }
    }

    fn file_path(&self, id: &FileId) -> PathBuf {
        self.path.join("files").join(id.hex())
    }

    fn symlink_path(&self, id: &SymlinkId) -> PathBuf {
        self.path.join("symlinks").join(id.hex())
    }

    fn tree_path(&self, id: &TreeId) -> PathBuf {
        self.path.join("trees").join(id.hex())
    }

    fn commit_path(&self, id: &CommitId) -> PathBuf {
        self.path.join("commits").join(id.hex())
    }
}

#[async_trait]
impl Backend for SimpleBackend {
    fn name(&self) -> &str {
        Self::name()
    }

    fn commit_id_length(&self) -> usize {
        COMMIT_ID_LENGTH
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
        1
    }

    async fn read_file(
        &self,
        path: &RepoPath,
        id: &FileId,
    ) -> BackendResult<Pin<Box<dyn AsyncRead + Send>>> {
        let disk_path = self.file_path(id);
        let mut file = File::open(disk_path).map_err(|err| map_not_found_err(err, id))?;
        let mut buf = vec![];
        file.read_to_end(&mut buf)
            .map_err(|err| BackendError::ReadFile {
                path: path.to_owned(),
                id: id.clone(),
                source: err.into(),
            })?;
        Ok(Box::pin(Cursor::new(buf)))
    }

    async fn write_file(
        &self,
        _path: &RepoPath,
        contents: &mut (dyn AsyncRead + Send + Unpin),
    ) -> BackendResult<FileId> {
        // TODO: Write temporary file in the destination directory (#5712)
        let temp_file = NamedTempFile::new_in(&self.path).map_err(to_other_err)?;
        let mut file = temp_file.as_file();
        let mut hasher = Blake2b512::new();
        let mut buff: Vec<u8> = vec![0; 1 << 14];
        loop {
            let bytes_read = contents.read(&mut buff).await.map_err(to_other_err)?;
            if bytes_read == 0 {
                break;
            }
            let bytes = &buff[..bytes_read];
            file.write_all(bytes).map_err(to_other_err)?;
            hasher.update(bytes);
        }
        file.flush().map_err(to_other_err)?;
        let id = FileId::new(hasher.finalize().to_vec());

        persist_content_addressed_temp_file(temp_file, self.file_path(&id))
            .map_err(to_other_err)?;
        Ok(id)
    }

    async fn read_symlink(&self, _path: &RepoPath, id: &SymlinkId) -> BackendResult<String> {
        let path = self.symlink_path(id);
        let target = fs::read_to_string(path).map_err(|err| map_not_found_err(err, id))?;
        Ok(target)
    }

    async fn write_symlink(&self, _path: &RepoPath, target: &str) -> BackendResult<SymlinkId> {
        // TODO: Write temporary file in the destination directory (#5712)
        let mut temp_file = NamedTempFile::new_in(&self.path).map_err(to_other_err)?;
        temp_file
            .write_all(target.as_bytes())
            .map_err(to_other_err)?;
        let mut hasher = Blake2b512::new();
        hasher.update(target.as_bytes());
        let id = SymlinkId::new(hasher.finalize().to_vec());

        persist_content_addressed_temp_file(temp_file, self.symlink_path(&id))
            .map_err(to_other_err)?;
        Ok(id)
    }

    async fn read_copy(&self, _id: &CopyId) -> BackendResult<CopyHistory> {
        Err(BackendError::Unsupported(
            "The simple backend doesn't support copies".to_string(),
        ))
    }

    async fn write_copy(&self, _contents: &CopyHistory) -> BackendResult<CopyId> {
        Err(BackendError::Unsupported(
            "The simple backend doesn't support copies".to_string(),
        ))
    }

    async fn get_related_copies(&self, _copy_id: &CopyId) -> BackendResult<Vec<CopyHistory>> {
        Err(BackendError::Unsupported(
            "The simple backend doesn't support copies".to_string(),
        ))
    }

    async fn read_tree(&self, _path: &RepoPath, id: &TreeId) -> BackendResult<Tree> {
        let path = self.tree_path(id);
        let buf = fs::read(path).map_err(|err| map_not_found_err(err, id))?;

        let proto = crate::protos::simple_store::Tree::decode(&*buf).map_err(to_other_err)?;
        Ok(tree_from_proto(proto))
    }

    async fn write_tree(&self, _path: &RepoPath, tree: &Tree) -> BackendResult<TreeId> {
        // TODO: Write temporary file in the destination directory (#5712)
        let temp_file = NamedTempFile::new_in(&self.path).map_err(to_other_err)?;

        let proto = tree_to_proto(tree);
        temp_file
            .as_file()
            .write_all(&proto.encode_to_vec())
            .map_err(to_other_err)?;

        let id = TreeId::new(blake2b_hash(tree).to_vec());

        persist_content_addressed_temp_file(temp_file, self.tree_path(&id))
            .map_err(to_other_err)?;
        Ok(id)
    }

    async fn read_commit(&self, id: &CommitId) -> BackendResult<Commit> {
        if *id == self.root_commit_id {
            return Ok(make_root_commit(
                self.root_change_id().clone(),
                self.empty_tree_id.clone(),
            ));
        }

        let path = self.commit_path(id);
        let buf = fs::read(path).map_err(|err| map_not_found_err(err, id))?;

        let proto = crate::protos::simple_store::Commit::decode(&*buf).map_err(to_other_err)?;
        Ok(commit_from_proto(proto))
    }

    async fn write_commit(
        &self,
        mut commit: Commit,
        sign_with: Option<&mut SigningFn>,
    ) -> BackendResult<(CommitId, Commit)> {
        assert!(commit.secure_sig.is_none(), "commit.secure_sig was set");

        if commit.parents.is_empty() {
            return Err(BackendError::Other(
                "Cannot write a commit with no parents".into(),
            ));
        }
        // TODO: Write temporary file in the destination directory (#5712)
        let temp_file = NamedTempFile::new_in(&self.path).map_err(to_other_err)?;

        let mut proto = commit_to_proto(&commit);
        if let Some(sign) = sign_with {
            let data = proto.encode_to_vec();
            let sig = sign(&data).map_err(to_other_err)?;
            proto.secure_sig = Some(sig.clone());
            commit.secure_sig = Some(SecureSig { data, sig });
        }

        temp_file
            .as_file()
            .write_all(&proto.encode_to_vec())
            .map_err(to_other_err)?;

        let id = CommitId::new(blake2b_hash(&commit).to_vec());

        persist_content_addressed_temp_file(temp_file, self.commit_path(&id))
            .map_err(to_other_err)?;
        Ok((id, commit))
    }

    fn get_copy_records(
        &self,
        _paths: Option<&[RepoPathBuf]>,
        _root: &CommitId,
        _head: &CommitId,
    ) -> BackendResult<BoxStream<'_, BackendResult<CopyRecord>>> {
        Ok(Box::pin(stream::empty()))
    }

    fn gc(&self, _index: &dyn Index, _keep_newer: SystemTime) -> BackendResult<()> {
        Ok(())
    }
}

#[expect(clippy::assigning_clones)]
pub fn commit_to_proto(commit: &Commit) -> crate::protos::simple_store::Commit {
    let mut proto = crate::protos::simple_store::Commit::default();
    for parent in &commit.parents {
        proto.parents.push(parent.to_bytes());
    }
    for predecessor in &commit.predecessors {
        proto.predecessors.push(predecessor.to_bytes());
    }
    proto.root_tree = commit.root_tree.iter().map(|id| id.to_bytes()).collect();
    proto.change_id = commit.change_id.to_bytes();
    proto.description = commit.description.clone();
    proto.author = Some(signature_to_proto(&commit.author));
    proto.committer = Some(signature_to_proto(&commit.committer));
    proto
}

fn commit_from_proto(mut proto: crate::protos::simple_store::Commit) -> Commit {
    // Note how .take() sets the secure_sig field to None before we encode the data.
    // Needs to be done first since proto is partially moved a bunch below
    let secure_sig = proto.secure_sig.take().map(|sig| SecureSig {
        data: proto.encode_to_vec(),
        sig,
    });

    let parents = proto.parents.into_iter().map(CommitId::new).collect();
    let predecessors = proto.predecessors.into_iter().map(CommitId::new).collect();
    let merge_builder: MergeBuilder<_> = proto.root_tree.into_iter().map(TreeId::new).collect();
    let root_tree = merge_builder.build();
    let change_id = ChangeId::new(proto.change_id);
    Commit {
        parents,
        predecessors,
        root_tree,
        // TODO: store conflict labels
        conflict_labels: Merge::resolved(String::new()),
        change_id,
        description: proto.description,
        author: signature_from_proto(proto.author.unwrap_or_default()),
        committer: signature_from_proto(proto.committer.unwrap_or_default()),
        secure_sig,
    }
}

fn tree_to_proto(tree: &Tree) -> crate::protos::simple_store::Tree {
    let mut proto = crate::protos::simple_store::Tree::default();
    for entry in tree.entries() {
        proto
            .entries
            .push(crate::protos::simple_store::tree::Entry {
                name: entry.name().as_internal_str().to_owned(),
                value: Some(tree_value_to_proto(entry.value())),
            });
    }
    proto
}

fn tree_from_proto(proto: crate::protos::simple_store::Tree) -> Tree {
    // Serialized data should be sorted
    let entries = proto
        .entries
        .into_iter()
        .map(|proto_entry| {
            let value = tree_value_from_proto(proto_entry.value.unwrap());
            (RepoPathComponentBuf::new(proto_entry.name).unwrap(), value)
        })
        .collect();
    Tree::from_sorted_entries(entries)
}

fn tree_value_to_proto(value: &TreeValue) -> crate::protos::simple_store::TreeValue {
    let mut proto = crate::protos::simple_store::TreeValue::default();
    match value {
        TreeValue::File {
            id,
            executable,
            copy_id,
        } => {
            proto.value = Some(crate::protos::simple_store::tree_value::Value::File(
                crate::protos::simple_store::tree_value::File {
                    id: id.to_bytes(),
                    executable: *executable,
                    copy_id: copy_id.to_bytes(),
                },
            ));
        }
        TreeValue::Symlink(id) => {
            proto.value = Some(crate::protos::simple_store::tree_value::Value::SymlinkId(
                id.to_bytes(),
            ));
        }
        TreeValue::GitSubmodule(_id) => {
            panic!("cannot store git submodules");
        }
        TreeValue::Tree(id) => {
            proto.value = Some(crate::protos::simple_store::tree_value::Value::TreeId(
                id.to_bytes(),
            ));
        }
    }
    proto
}

fn tree_value_from_proto(proto: crate::protos::simple_store::TreeValue) -> TreeValue {
    match proto.value.unwrap() {
        crate::protos::simple_store::tree_value::Value::TreeId(id) => {
            TreeValue::Tree(TreeId::new(id))
        }
        crate::protos::simple_store::tree_value::Value::File(
            crate::protos::simple_store::tree_value::File {
                id,
                executable,
                copy_id,
            },
        ) => TreeValue::File {
            id: FileId::new(id),
            executable,
            copy_id: CopyId::new(copy_id),
        },
        crate::protos::simple_store::tree_value::Value::SymlinkId(id) => {
            TreeValue::Symlink(SymlinkId::new(id))
        }
    }
}

fn signature_to_proto(signature: &Signature) -> crate::protos::simple_store::commit::Signature {
    crate::protos::simple_store::commit::Signature {
        name: signature.name.clone(),
        email: signature.email.clone(),
        timestamp: Some(crate::protos::simple_store::commit::Timestamp {
            millis_since_epoch: signature.timestamp.timestamp.0,
            tz_offset: signature.timestamp.tz_offset,
        }),
    }
}

fn signature_from_proto(proto: crate::protos::simple_store::commit::Signature) -> Signature {
    let timestamp = proto.timestamp.unwrap_or_default();
    Signature {
        name: proto.name,
        email: proto.email,
        timestamp: Timestamp {
            timestamp: MillisSinceEpoch(timestamp.millis_since_epoch),
            tz_offset: timestamp.tz_offset,
        },
    }
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use pollster::FutureExt as _;

    use super::*;
    use crate::merge::Merge;
    use crate::tests::new_temp_dir;

    /// Test that parents get written correctly
    #[test]
    fn write_commit_parents() {
        let temp_dir = new_temp_dir();
        let store_path = temp_dir.path();

        let backend = SimpleBackend::init(store_path);
        let mut commit = Commit {
            parents: vec![],
            predecessors: vec![],
            root_tree: Merge::resolved(backend.empty_tree_id().clone()),
            conflict_labels: Merge::resolved(String::new()),
            change_id: ChangeId::from_hex("abc123"),
            description: "".to_string(),
            author: create_signature(),
            committer: create_signature(),
            secure_sig: None,
        };

        let write_commit = |commit: Commit| -> BackendResult<(CommitId, Commit)> {
            backend.write_commit(commit, None).block_on()
        };

        // No parents
        commit.parents = vec![];
        assert_matches!(
            write_commit(commit.clone()),
            Err(BackendError::Other(err)) if err.to_string().contains("no parents")
        );

        // Only root commit as parent
        commit.parents = vec![backend.root_commit_id().clone()];
        let first_id = write_commit(commit.clone()).unwrap().0;
        let first_commit = backend.read_commit(&first_id).block_on().unwrap();
        assert_eq!(first_commit, commit);

        // Only non-root commit as parent
        commit.parents = vec![first_id.clone()];
        let second_id = write_commit(commit.clone()).unwrap().0;
        let second_commit = backend.read_commit(&second_id).block_on().unwrap();
        assert_eq!(second_commit, commit);

        // Merge commit
        commit.parents = vec![first_id.clone(), second_id.clone()];
        let merge_id = write_commit(commit.clone()).unwrap().0;
        let merge_commit = backend.read_commit(&merge_id).block_on().unwrap();
        assert_eq!(merge_commit, commit);

        // Merge commit with root as one parent
        commit.parents = vec![first_id, backend.root_commit_id().clone()];
        let root_merge_id = write_commit(commit.clone()).unwrap().0;
        let root_merge_commit = backend.read_commit(&root_merge_id).block_on().unwrap();
        assert_eq!(root_merge_commit, commit);
    }

    fn create_signature() -> Signature {
        Signature {
            name: "Someone".to_string(),
            email: "someone@example.com".to_string(),
            timestamp: Timestamp {
                timestamp: MillisSinceEpoch(0),
                tz_offset: 0,
            },
        }
    }
}
