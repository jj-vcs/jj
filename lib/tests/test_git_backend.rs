// Copyright 2024 The Jujutsu Authors
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

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;

use futures::executor::block_on_stream;
use jj_lib::backend::CommitId;
use jj_lib::backend::CopyRecord;
use jj_lib::commit::Commit;
use jj_lib::conflict_labels::ConflictLabels;
use jj_lib::git_backend::GitBackend;
use jj_lib::git_backend::JJ_TREES_COMMIT_HEADER;
use jj_lib::merge::Merge;
use jj_lib::merged_tree::MergedTree;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::store::Store;
use jj_lib::transaction::Transaction;
use maplit::hashmap;
use maplit::hashset;
use testutils::TestRepo;
use testutils::TestRepoBackend;
use testutils::assert_tree_eq;
use testutils::commit_with_tree;
use testutils::create_random_commit;
use testutils::create_single_tree;
use testutils::create_tree;
use testutils::is_external_tool_installed;
use testutils::repo_path;
use testutils::repo_path_buf;
use testutils::write_random_commit;
use testutils::write_random_commit_with_parents;

fn get_git_backend(repo: &Arc<ReadonlyRepo>) -> &GitBackend {
    repo.store().backend_impl().unwrap()
}

fn collect_no_gc_refs(git_repo_path: &Path) -> HashSet<CommitId> {
    // Load fresh git repo to isolate from false caching issue. Here we want to
    // ensure that the underlying data is correct. We could test the in-memory
    // data as well, but we don't have any special handling in our code.
    let git_repo = gix::open(git_repo_path).unwrap();
    let git_refs = git_repo.references().unwrap();
    let no_gc_refs_iter = git_refs.prefixed("refs/jj/keep/").unwrap();
    no_gc_refs_iter
        .map(|git_ref| CommitId::from_bytes(git_ref.unwrap().id().as_bytes()))
        .collect()
}

fn get_copy_records(
    store: &Store,
    paths: Option<&[RepoPathBuf]>,
    a: &Commit,
    b: &Commit,
) -> HashMap<String, String> {
    let stream = store.get_copy_records(paths, a.id(), b.id()).unwrap();
    let mut res: HashMap<String, String> = HashMap::new();
    for CopyRecord { target, source, .. } in block_on_stream(stream).filter_map(|r| r.ok()) {
        res.insert(
            target.as_internal_file_string().into(),
            source.as_internal_file_string().into(),
        );
    }
    res
}

fn make_commit(
    tx: &mut Transaction,
    parents: Vec<CommitId>,
    content: &[(&RepoPath, &str)],
) -> Commit {
    let tree = create_tree(tx.base_repo(), content);
    tx.repo_mut().new_commit(parents, tree).write().unwrap()
}

#[test]
fn test_gc() {
    // TODO: Better way to disable the test if git command couldn't be executed
    if !is_external_tool_installed("git") {
        eprintln!("Skipping because git command might fail to run");
        return;
    }

    let test_repo = TestRepo::init_with_backend(TestRepoBackend::Git);
    let repo = test_repo.repo;
    let git_repo_path = get_git_backend(&repo).git_repo_path();
    let base_index = repo.readonly_index();

    // Set up commits:
    //
    //     H (predecessor: D)
    //   G |
    //   |\|
    //   | F
    //   E |
    // D | |
    // C |/
    // |/
    // B
    // A
    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);
    let commit_c = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_d = write_random_commit_with_parents(tx.repo_mut(), &[&commit_c]);
    let commit_e = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_f = write_random_commit_with_parents(tx.repo_mut(), &[&commit_b]);
    let commit_g = write_random_commit_with_parents(tx.repo_mut(), &[&commit_e, &commit_f]);
    let commit_h = create_random_commit(tx.repo_mut())
        .set_parents(vec![commit_f.id().clone()])
        .set_predecessors(vec![commit_d.id().clone()])
        .write()
        .unwrap();
    let repo = tx.commit("test").unwrap();
    assert_eq!(
        *repo.view().heads(),
        hashset! {
            commit_d.id().clone(),
            commit_g.id().clone(),
            commit_h.id().clone(),
        },
    );

    // At first, all commits have no-gc refs
    assert_eq!(
        collect_no_gc_refs(git_repo_path),
        hashset! {
            commit_a.id().clone(),
            commit_b.id().clone(),
            commit_c.id().clone(),
            commit_d.id().clone(),
            commit_e.id().clone(),
            commit_f.id().clone(),
            commit_g.id().clone(),
            commit_h.id().clone(),
        },
    );

    // Empty index, but all kept by file modification time
    // (Beware that this invokes "git gc" and refs will be packed.)
    repo.store()
        .gc(base_index.as_index(), SystemTime::UNIX_EPOCH)
        .unwrap();
    assert_eq!(
        collect_no_gc_refs(git_repo_path),
        hashset! {
            commit_a.id().clone(),
            commit_b.id().clone(),
            commit_c.id().clone(),
            commit_d.id().clone(),
            commit_e.id().clone(),
            commit_f.id().clone(),
            commit_g.id().clone(),
            commit_h.id().clone(),
        },
    );

    // Don't rely on the exact system time because file modification time might
    // have lower precision for example.
    let now = || SystemTime::now() + Duration::from_secs(1);

    // All reachable: redundant no-gc refs will be removed
    repo.store().gc(repo.index(), now()).unwrap();
    assert_eq!(
        collect_no_gc_refs(git_repo_path),
        hashset! {
            commit_d.id().clone(),
            commit_g.id().clone(),
            commit_h.id().clone(),
        },
    );

    // G is no longer reachable
    let mut mut_index = base_index.start_modification();
    mut_index.add_commit(&commit_a).unwrap();
    mut_index.add_commit(&commit_b).unwrap();
    mut_index.add_commit(&commit_c).unwrap();
    mut_index.add_commit(&commit_d).unwrap();
    mut_index.add_commit(&commit_e).unwrap();
    mut_index.add_commit(&commit_f).unwrap();
    mut_index.add_commit(&commit_h).unwrap();
    repo.store().gc(mut_index.as_index(), now()).unwrap();
    assert_eq!(
        collect_no_gc_refs(git_repo_path),
        hashset! {
            commit_d.id().clone(),
            commit_e.id().clone(),
            commit_h.id().clone(),
        },
    );

    // D|E|H are no longer reachable
    let mut mut_index = base_index.start_modification();
    mut_index.add_commit(&commit_a).unwrap();
    mut_index.add_commit(&commit_b).unwrap();
    mut_index.add_commit(&commit_c).unwrap();
    mut_index.add_commit(&commit_f).unwrap();
    repo.store().gc(mut_index.as_index(), now()).unwrap();
    assert_eq!(
        collect_no_gc_refs(git_repo_path),
        hashset! {
            commit_c.id().clone(),
            commit_f.id().clone(),
        },
    );

    // B|C|F are no longer reachable
    let mut mut_index = base_index.start_modification();
    mut_index.add_commit(&commit_a).unwrap();
    repo.store().gc(mut_index.as_index(), now()).unwrap();
    assert_eq!(
        collect_no_gc_refs(git_repo_path),
        hashset! {
            commit_a.id().clone(),
        },
    );

    // All unreachable
    repo.store().gc(base_index.as_index(), now()).unwrap();
    assert_eq!(collect_no_gc_refs(git_repo_path), hashset! {});
}

#[test]
fn test_copy_detection() {
    let test_repo = TestRepo::init_with_backend(TestRepoBackend::Git);
    let repo = &test_repo.repo;

    let paths = &[
        repo_path_buf("file0"),
        repo_path_buf("file1"),
        repo_path_buf("file2"),
    ];

    let mut tx = repo.start_transaction();
    let commit_a = make_commit(
        &mut tx,
        vec![repo.store().root_commit_id().clone()],
        &[(&paths[0], "content")],
    );
    let commit_b = make_commit(
        &mut tx,
        vec![commit_a.id().clone()],
        &[(&paths[1], "content")],
    );
    let commit_c = make_commit(
        &mut tx,
        vec![commit_b.id().clone()],
        &[(&paths[2], "content")],
    );

    let store = repo.store();
    assert_eq!(
        get_copy_records(store, Some(paths), &commit_a, &commit_b),
        HashMap::from([("file1".to_string(), "file0".to_string())])
    );
    assert_eq!(
        get_copy_records(store, Some(paths), &commit_b, &commit_c),
        HashMap::from([("file2".to_string(), "file1".to_string())])
    );
    assert_eq!(
        get_copy_records(store, Some(paths), &commit_a, &commit_c),
        HashMap::from([("file2".to_string(), "file0".to_string())])
    );
    assert_eq!(
        get_copy_records(store, None, &commit_a, &commit_c),
        HashMap::from([("file2".to_string(), "file0".to_string())])
    );
    assert_eq!(
        get_copy_records(store, Some(&[paths[1].clone()]), &commit_a, &commit_c),
        HashMap::default(),
    );
    assert_eq!(
        get_copy_records(store, Some(paths), &commit_c, &commit_c),
        HashMap::default(),
    );
}

#[test]
fn test_copy_detection_file_and_dir() {
    let test_repo = TestRepo::init_with_backend(TestRepoBackend::Git);
    let repo = &test_repo.repo;

    // a -> b (file)
    // b -> a (dir)
    // c -> c/file (file)
    let mut tx = repo.start_transaction();
    let commit_a = make_commit(
        &mut tx,
        vec![repo.store().root_commit_id().clone()],
        &[
            (repo_path("a"), "content1"),
            (repo_path("b/file"), "content2"),
            (repo_path("c"), "content3"),
        ],
    );
    let commit_b = make_commit(
        &mut tx,
        vec![commit_a.id().clone()],
        &[
            (repo_path("a/file"), "content2"),
            (repo_path("b"), "content1"),
            (repo_path("c/file"), "content3"),
        ],
    );

    assert_eq!(
        get_copy_records(repo.store(), None, &commit_a, &commit_b),
        hashmap! {
            "b".to_owned() => "a".to_owned(),
            "a/file".to_owned() => "b/file".to_owned(),
            "c/file".to_owned() => "c".to_owned(),
        }
    );
}

#[test]
fn test_jj_trees_header_with_one_tree() {
    let test_repo = TestRepo::init_with_backend(TestRepoBackend::Git);
    let repo = test_repo.repo;
    let git_backend = get_git_backend(&repo);
    let git_repo = git_backend.git_repo();

    let tree_1 = create_single_tree(&repo, &[(repo_path("file"), "aaa")]);
    let tree_2 = create_single_tree(&repo, &[(repo_path("file"), "bbb")]);

    // Create a normal commit with tree 1
    let commit = commit_with_tree(
        repo.store(),
        MergedTree::resolved(repo.store().clone(), tree_1.id().clone()),
    );
    let git_commit_id = gix::ObjectId::from_bytes_or_panic(commit.id().as_bytes());
    let git_commit = git_repo.find_commit(git_commit_id).unwrap();

    // Add `jj:trees` with a single tree which is different from the Git commit tree
    let mut new_commit: gix::objs::Commit = git_commit.decode().unwrap().into();
    new_commit.extra_headers = vec![(
        JJ_TREES_COMMIT_HEADER.into(),
        tree_2.id().to_string().into(),
    )];
    let new_commit_id = git_repo.write_object(&new_commit).unwrap();
    let new_commit_id = CommitId::from_bytes(new_commit_id.as_bytes());

    // Import new commit into `jj` repo. This should fail, because allowing a
    // non-conflicted commit to have a different tree in `jj` than in Git could be
    // used to hide malicious code.
    insta::assert_debug_snapshot!(git_backend.import_head_commits(std::slice::from_ref(&new_commit_id)), @r#"
    Err(
        ReadObject {
            object_type: "commit",
            hash: "87df728a30166ce1de0bf883948dd66b74cf25a0",
            source: "Invalid jj:trees header",
        },
    )
    "#);
}

#[test]
fn test_conflict_headers_roundtrip() {
    let test_repo = TestRepo::init_with_backend(TestRepoBackend::Git);
    let repo = test_repo.repo;

    let tree_1 = create_single_tree(&repo, &[(repo_path("file"), "aaa")]);
    let tree_2 = create_single_tree(&repo, &[(repo_path("file"), "bbb")]);
    let tree_3 = create_single_tree(&repo, &[(repo_path("file"), "ccc")]);
    let tree_4 = create_single_tree(&repo, &[(repo_path("file"), "ddd")]);
    let tree_5 = create_single_tree(&repo, &[(repo_path("file"), "eee")]);
    let tree_6 = create_single_tree(&repo, &[(repo_path("file"), "fff")]);
    let tree_7 = create_single_tree(&repo, &[(repo_path("file"), "ggg")]);

    // This creates a Git commit header with leading and trailing newlines to ensure
    // that it can still be parsed correctly. The resulting `jj:conflict-labels`
    // header value will look like `\nbase 1\nside 2\n\nside 3\n\n\n`.
    let merged_tree = MergedTree::new(
        repo.store().clone(),
        Merge::from_vec(vec![
            tree_1.id().clone(),
            tree_2.id().clone(),
            tree_3.id().clone(),
            tree_4.id().clone(),
            tree_5.id().clone(),
            tree_6.id().clone(),
            tree_7.id().clone(),
        ]),
        ConflictLabels::from_vec(vec![
            "".into(),
            "base 1".into(),
            "side 2".into(),
            "".into(),
            "side 3".into(),
            "".into(),
            "".into(),
        ]),
    );

    // Create a commit with the conflicted tree.
    let commit = commit_with_tree(repo.store(), merged_tree.clone());
    // Clear cached commit to ensure it is re-read.
    repo.store().clear_caches();
    // Conflict trees and labels should be preserved on read.
    assert_tree_eq!(
        repo.store().get_commit(commit.id()).unwrap().tree(),
        merged_tree
    );
}
