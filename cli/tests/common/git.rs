// Copyright 2025 The Jujutsu Authors
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

use std::path::Path;
use std::path::PathBuf;

pub const GIT_USER: &str = "Someone";
pub const GIT_EMAIL: &str = "someone@example.org";

fn git_config() -> Vec<bstr::BString> {
    vec![
        format!("user.name = {GIT_USER}").into(),
        format!("user.email = {GIT_EMAIL}").into(),
        "init.defaultBranch = master".into(),
    ]
}

fn open_options() -> gix::open::Options {
    gix::open::Options::isolated().config_overrides(git_config())
}

pub fn open(directory: impl Into<PathBuf>) -> gix::Repository {
    gix::open_opts(directory, open_options()).unwrap()
}

pub fn init(directory: impl AsRef<Path>) -> gix::Repository {
    gix::ThreadSafeRepository::init_opts(
        directory,
        gix::create::Kind::WithWorktree,
        gix::create::Options::default(),
        open_options(),
    )
    .unwrap()
    .to_thread_local()
}

pub fn init_bare(directory: impl AsRef<Path>) -> gix::Repository {
    gix::ThreadSafeRepository::init_opts(
        directory,
        gix::create::Kind::Bare,
        gix::create::Options::default(),
        open_options(),
    )
    .unwrap()
    .to_thread_local()
}

pub fn clone(dest_path: &Path, url: &str) -> gix::Repository {
    let mut prepare_fetch = gix::clone::PrepareFetch::new(
        url,
        dest_path,
        gix::create::Kind::WithWorktree,
        gix::create::Options::default(),
        open_options(),
    )
    .unwrap();
    let (mut prepare_checkout, _outcome) = prepare_fetch
        .fetch_then_checkout(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
        .unwrap();
    let (repo, _outcome) = prepare_checkout
        .main_worktree(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
        .unwrap();

    repo
}

pub struct CommitResult {
    pub tree_id: gix::ObjectId,
    pub commit_id: gix::ObjectId,
}

pub fn add_commit(
    repo: &gix::Repository,
    reference: &str,
    filename: &str,
    content: &[u8],
    message: &str,
    parents: &[gix::ObjectId],
) -> CommitResult {
    let blob_oid = repo.write_blob(content).unwrap();

    let parent_tree_editor = parents.first().map(|commit_id| {
        repo.find_commit(*commit_id)
            .unwrap()
            .tree()
            .unwrap()
            .edit()
            .unwrap()
    });
    let empty_tree_editor_fn = || {
        repo.edit_tree(gix::ObjectId::empty_tree(repo.object_hash()))
            .unwrap()
    };

    let mut tree_editor = parent_tree_editor.unwrap_or_else(empty_tree_editor_fn);
    tree_editor
        .upsert(filename, gix::object::tree::EntryKind::Blob, blob_oid)
        .unwrap();
    let tree_id = tree_editor.write().unwrap().detach();
    let commit_id = write_commit(repo, reference, tree_id, message, parents);
    CommitResult { tree_id, commit_id }
}

pub fn write_commit(
    repo: &gix::Repository,
    reference: &str,
    tree_id: gix::ObjectId,
    message: &str,
    parents: &[gix::ObjectId],
) -> gix::ObjectId {
    let signature = signature();
    repo.commit_as(
        &signature,
        &signature,
        reference,
        message,
        tree_id,
        parents.iter().copied(),
    )
    .unwrap()
    .detach()
}

pub fn set_head_to_id(repo: &gix::Repository, target: gix::ObjectId) {
    repo.edit_reference(gix::refs::transaction::RefEdit {
        change: gix::refs::transaction::Change::Update {
            log: gix::refs::transaction::LogChange::default(),
            expected: gix::refs::transaction::PreviousValue::Any,
            new: gix::refs::Target::Object(target),
        },
        name: "HEAD".try_into().unwrap(),
        deref: false,
    })
    .unwrap();
}

fn signature() -> gix::actor::Signature {
    gix::actor::Signature {
        name: bstr::BString::from(GIT_USER),
        email: bstr::BString::from(GIT_EMAIL),
        time: gix::date::Time::new(0, 0),
    }
}
