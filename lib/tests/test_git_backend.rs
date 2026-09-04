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
use itertools::Itertools as _;
use jj_core::backend::Backend as _;
use jj_core::backend::CommitId;
use jj_core::backend::CopyRecord;
use jj_core::backend::TreeValue;
use jj_core::commit::Commit;
use jj_core::conflict_labels::ConflictLabels;
use jj_core::index::Index;
use jj_core::merge::Merge;
use jj_core::merged_tree::MergedTree;
use jj_core::object_id::ObjectId;
use jj_core::repo_path::RepoPath;
use jj_core::repo_path::RepoPathBuf;
use jj_core::store::Store;
use jj_git_backend::git_backend::GitBackend;
use jj_git_backend::git_backend::JJ_TREES_COMMIT_HEADER;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::transaction::Transaction;
use jj_local_util::stacked_table::TableSegment as _;
use jj_local_util::stacked_table::TableStore;
use maplit::hashmap;
use maplit::hashset;
use pollster::FutureExt as _;
use testutils::CommitBuilderExt as _;
use testutils::TestRepo;
use testutils::TestRepoBackend;
use testutils::TestResult;
use testutils::assert_tree_eq;
use testutils::commit_with_tree;
use testutils::create_random_commit;
use testutils::create_single_tree;
use testutils::create_tree;
use testutils::is_external_tool_installed;
use testutils::repo_path;
use testutils::repo_path_buf;
use testutils::repo_path_component;
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

/// Checks whether the object has been persisted, using a fresh handle to the
/// Git repo so that in-memory objects in the backend aren't visible.
fn git_object_exists(git_repo_path: &Path, id: &impl ObjectId) -> bool {
    let git_repo = gix::open(git_repo_path).unwrap();
    git_repo.has_object(gix::ObjectId::from_bytes_or_panic(id.as_bytes()))
}

fn make_commit(
    tx: &mut Transaction,
    parents: Vec<CommitId>,
    content: &[(&RepoPath, &str)],
) -> Commit {
    let tree = create_tree(tx.base_repo(), content);
    tx.repo_mut().new_commit(parents, tree).write_unwrap()
}

fn list_dir(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_str().unwrap().to_owned())
        .sorted()
        .collect()
}

#[test]
fn test_gc() -> TestResult {
    // TODO: Better way to disable the test if git command couldn't be executed
    if !is_external_tool_installed("git") {
        eprintln!("Skipping because git command might fail to run");
        return Ok(());
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
        .write_unwrap();
    let repo = tx.commit("test").block_on()?;
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

    // Empty list of heads, but all kept by file modification time
    // (Beware that this invokes "git gc" and refs will be packed.)
    get_git_backend(&repo)
        .gc(&[], SystemTime::UNIX_EPOCH)
        .block_on()?;
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

    let gc = |repo: &Arc<ReadonlyRepo>, index: &dyn Index| -> TestResult {
        let heads: Vec<_> = index.all_heads_for_gc().unwrap().collect_vec();
        get_git_backend(repo).gc(&heads, now()).block_on()?;
        Ok(())
    };

    // All reachable: redundant no-gc refs will be removed
    gc(&repo, repo.index())?;
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
    mut_index.add_commit(&commit_a).block_on()?;
    mut_index.add_commit(&commit_b).block_on()?;
    mut_index.add_commit(&commit_c).block_on()?;
    mut_index.add_commit(&commit_d).block_on()?;
    mut_index.add_commit(&commit_e).block_on()?;
    mut_index.add_commit(&commit_f).block_on()?;
    mut_index.add_commit(&commit_h).block_on()?;
    gc(&repo, mut_index.as_index())?;
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
    mut_index.add_commit(&commit_a).block_on()?;
    mut_index.add_commit(&commit_b).block_on()?;
    mut_index.add_commit(&commit_c).block_on()?;
    mut_index.add_commit(&commit_f).block_on()?;
    gc(&repo, mut_index.as_index())?;
    assert_eq!(
        collect_no_gc_refs(git_repo_path),
        hashset! {
            commit_c.id().clone(),
            commit_f.id().clone(),
        },
    );

    // B|C|F are no longer reachable
    let mut mut_index = base_index.start_modification();
    mut_index.add_commit(&commit_a).block_on()?;
    gc(&repo, mut_index.as_index())?;
    assert_eq!(
        collect_no_gc_refs(git_repo_path),
        hashset! {
            commit_a.id().clone(),
        },
    );

    // All unreachable
    gc(&repo, base_index.as_index())?;
    assert_eq!(collect_no_gc_refs(git_repo_path), hashset! {});
    Ok(())
}

#[test]
fn test_gc_extra_table() -> TestResult {
    if !is_external_tool_installed("git") {
        eprintln!("Skipping because git command might fail to run");
        return Ok(());
    }

    let test_repo = TestRepo::init_with_backend(TestRepoBackend::Git);
    let extra_path = test_repo.repo_path().join("store").join("extra");
    let extra_key_size = test_repo.repo.store().root_commit_id().as_bytes().len();
    let load_repo = || {
        test_repo
            .env
            .load_repo_at_head(test_repo.repo.settings(), test_repo.repo_path())
    };
    let load_extra_table = || {
        TableStore::load(extra_path.clone(), extra_key_size)
            .get_head()
            .unwrap()
    };
    let collect_extra_segment_num_entries = || {
        let mut num_entries = load_extra_table()
            .ancestor_segments()
            .map(|table| table.segment_num_entries())
            .collect_vec();
        num_entries.reverse();
        num_entries
    };

    // Sanity check for the initial state
    assert_eq!(collect_extra_segment_num_entries(), [0]);
    assert_eq!(list_dir(&extra_path).len(), 1 + 1); // empty segment + "heads"

    // Write 4 commits
    let mut tx = test_repo.repo.start_transaction();
    for _ in 0..4 {
        write_random_commit(tx.repo_mut());
    }
    tx.commit("test").block_on()?;
    // All 4 commits are persisted by a single flush, so they end up in one table
    // segment
    assert_eq!(collect_extra_segment_num_entries(), [4]);
    assert_eq!(list_dir(&extra_path).len(), 2 + 1);

    // Reload repo to invalidate cache in TableStore
    let repo = load_repo();
    let heads = repo
        .readonly_index()
        .as_index()
        .all_heads_for_gc()?
        .collect_vec();

    // All segments should be kept by modification time
    get_git_backend(&repo)
        .gc(&heads, SystemTime::UNIX_EPOCH)
        .block_on()?;
    assert_eq!(collect_extra_segment_num_entries(), [4]);
    assert_eq!(list_dir(&extra_path).len(), 2 + 1);

    // All unreachable segments should be removed
    let now = SystemTime::now() + Duration::from_secs(1);
    get_git_backend(&repo).gc(&heads, now).block_on()?;
    assert_eq!(collect_extra_segment_num_entries(), [4]);
    assert_eq!(list_dir(&extra_path).len(), 1 + 1);

    // Ensure that repo is still loadable
    load_repo();
    Ok(())
}

#[test]
fn test_copy_detection() -> TestResult {
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
    Ok(())
}

#[test]
fn test_copy_detection_file_and_dir() -> TestResult {
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
    Ok(())
}

#[test]
fn test_jj_trees_header_with_one_tree() -> TestResult {
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
    let git_commit = git_repo.find_commit(git_commit_id)?;

    // Add `jj:trees` with a single tree which is different from the Git commit tree
    let mut new_commit: gix::objs::Commit = git_commit.decode().unwrap().try_into().unwrap();
    new_commit.extra_headers = vec![(
        JJ_TREES_COMMIT_HEADER.into(),
        tree_2.id().to_string().into(),
    )];
    let new_commit_id = git_repo.write_object(&new_commit)?;
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
    Ok(())
}

#[test]
fn test_conflict_headers_roundtrip() -> TestResult {
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
            // Test that various UTF-8 characters can be encoded and decoded. Git doesn't allow
            // storing arbitrary binary data in headers, but it does allow storing UTF-8 encoded
            // strings as long as they don't contain U+0000 codepoints.
            "base 1 \"utf8 \u{67d4}\u{8853} \u{ba4}\u{bc1}\u{bb0}\u{bc1} \u{2699}\u{fe0f}\"".into(),
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
    assert_tree_eq!(repo.store().get_commit(commit.id())?.tree(), merged_tree);
    Ok(())
}

#[test]
fn test_writes_are_buffered_until_flush() -> TestResult {
    let test_repo = TestRepo::init_with_backend(TestRepoBackend::Git);
    let repo = test_repo.repo.clone();
    let store = repo.store();
    let git_repo_path = get_git_backend(&repo).git_repo_path().to_owned();

    let mut tx = repo.start_transaction();
    let commit = make_commit(
        &mut tx,
        vec![store.root_commit_id().clone()],
        &[(repo_path("file"), "contents\n")],
    );
    let tree_id = commit.tree_ids().first().clone();
    let file_value = commit.tree().path_value(repo_path("file")).block_on()?;
    let Some(TreeValue::File { id: file_id, .. }) = file_value.as_resolved().unwrap() else {
        panic!("unexpected file value: {file_value:?}");
    };

    // The objects can be read back through the store before they've been
    // persisted.
    store.clear_caches();
    let read_commit = store.get_commit(commit.id())?;
    assert_eq!(read_commit.change_id(), commit.change_id());
    assert_eq!(read_commit.tree_ids(), commit.tree_ids());
    let read_tree = store.get_tree(RepoPathBuf::root(), &tree_id).block_on()?;
    assert!(read_tree.value(repo_path_component("file")).is_some());

    // But Git doesn't see them yet.
    assert!(!git_object_exists(&git_repo_path, commit.id()));
    assert!(!git_object_exists(&git_repo_path, &tree_id));
    assert!(!git_object_exists(&git_repo_path, file_id));
    assert!(!collect_no_gc_refs(&git_repo_path).contains(commit.id()));

    // Committing the transaction persists the objects, the no-gc ref, and the
    // extra metadata.
    let repo = tx.commit("test").block_on()?;
    assert!(git_object_exists(&git_repo_path, commit.id()));
    assert!(git_object_exists(&git_repo_path, &tree_id));
    assert!(git_object_exists(&git_repo_path, file_id));
    assert!(collect_no_gc_refs(&git_repo_path).contains(commit.id()));
    let reloaded_repo = test_repo
        .env
        .load_repo_at_head(repo.settings(), test_repo.repo_path());
    let reloaded_commit = reloaded_repo.store().get_commit(commit.id())?;
    assert_eq!(reloaded_commit.change_id(), commit.change_id());
    Ok(())
}

#[test]
fn test_dropped_transaction_writes_no_objects() -> TestResult {
    let test_repo = TestRepo::init_with_backend(TestRepoBackend::Git);
    let repo = test_repo.repo;
    let store = repo.store();
    let git_repo_path = get_git_backend(&repo).git_repo_path().to_owned();
    let objects_path = git_repo_path.join("objects");
    let initial_objects_dir = list_dir(&objects_path);

    let mut tx = repo.start_transaction();
    let commit = make_commit(
        &mut tx,
        vec![store.root_commit_id().clone()],
        &[(repo_path("file"), "contents\n")],
    );
    drop(tx);

    // Nothing has been written to the Git repo, not even loose objects.
    assert!(!git_object_exists(&git_repo_path, commit.id()));
    assert_eq!(list_dir(&objects_path), initial_objects_dir);
    assert!(!collect_no_gc_refs(&git_repo_path).contains(commit.id()));
    Ok(())
}

#[test]
fn test_flush_persists_objects_written_outside_transaction() -> TestResult {
    let test_repo = TestRepo::init_with_backend(TestRepoBackend::Git);
    let repo = test_repo.repo;
    let store = repo.store().clone();
    let git_repo_path = get_git_backend(&repo).git_repo_path().to_owned();

    // Trees can be written without being referenced by a commit (the working
    // copy state refers to trees by ID, for example).
    let tree = create_tree(&repo, &[(repo_path("a"), "a\n")]);
    let tree_id = tree.tree_ids().first().clone();
    assert!(!git_object_exists(&git_repo_path, &tree_id));

    // They're persisted by the next flush along with everything else.
    let mut tx = repo.start_transaction();
    let commit = write_random_commit(tx.repo_mut());
    tx.commit("test").block_on()?;
    assert!(git_object_exists(&git_repo_path, commit.id()));
    assert!(git_object_exists(&git_repo_path, &tree_id));

    // An explicit flush also works, even if no commits have been written.
    let tree = create_tree(&repo, &[(repo_path("b"), "b\n")]);
    let tree_id = tree.tree_ids().first().clone();
    assert!(!git_object_exists(&git_repo_path, &tree_id));
    store.flush()?;
    assert!(git_object_exists(&git_repo_path, &tree_id));
    Ok(())
}

#[test]
fn test_large_blobs_are_written_to_disk_immediately() -> TestResult {
    let test_repo = TestRepo::init_with_backend(TestRepoBackend::Git);
    let repo = test_repo.repo;
    let store = repo.store();
    let git_repo_path = get_git_backend(&repo).git_repo_path().to_owned();

    let small_file_id = store
        .write_file(repo_path("small"), &mut &b"small"[..])
        .block_on()?;
    assert!(!git_object_exists(&git_repo_path, &small_file_id));

    // Large blobs bypass the in-memory store to bound memory usage.
    let large_contents = vec![b'x'; 1024 * 1024];
    let large_file_id = store
        .write_file(repo_path("large"), &mut large_contents.as_slice())
        .block_on()?;
    assert!(git_object_exists(&git_repo_path, &large_file_id));

    store.flush()?;
    assert!(git_object_exists(&git_repo_path, &small_file_id));
    Ok(())
}

#[test]
fn test_flush_writes_pack_for_many_objects() -> TestResult {
    let test_repo = TestRepo::init_with_backend(TestRepoBackend::Git);
    let repo = test_repo.repo.clone();
    let store = repo.store();
    let git_repo_path = get_git_backend(&repo).git_repo_path().to_owned();
    let objects_path = git_repo_path.join("objects");
    let pack_path = objects_path.join("pack");
    let count_loose_objects = || {
        std::fs::read_dir(&objects_path)
            .unwrap()
            .map(|entry| entry.unwrap())
            .filter(|entry| entry.file_name().len() == 2)
            .map(|entry| std::fs::read_dir(entry.path()).unwrap().count())
            .sum::<usize>()
    };
    let list_packs = || {
        list_dir(&pack_path)
            .into_iter()
            .filter(|name| name.ends_with(".pack") || name.ends_with(".keep"))
            .collect_vec()
    };

    // A few objects are written as loose objects
    let mut tx = repo.start_transaction();
    let small_commit = make_commit(
        &mut tx,
        vec![store.root_commit_id().clone()],
        &[(repo_path("file"), "contents\n")],
    );
    let repo = tx.commit("test").block_on()?;
    assert!(git_object_exists(&git_repo_path, small_commit.id()));
    let num_loose_objects = count_loose_objects();
    assert_eq!(num_loose_objects, 3); // commit, tree, blob
    assert!(list_packs().is_empty());

    // Many objects (here 200 blobs + 1 tree + 1 commit) are written to a pack
    let contents = (0..200)
        .map(|i| (repo_path_buf(format!("file{i}")), format!("contents {i}\n")))
        .collect_vec();
    let contents = contents
        .iter()
        .map(|(path, contents)| (path.as_ref(), contents.as_str()))
        .collect_vec();
    let mut tx = repo.start_transaction();
    let large_commit = make_commit(&mut tx, vec![store.root_commit_id().clone()], &contents);
    let repo = tx.commit("test").block_on()?;
    assert!(git_object_exists(&git_repo_path, large_commit.id()));
    assert!(git_object_exists(
        &git_repo_path,
        large_commit.tree_ids().first()
    ));
    assert_eq!(count_loose_objects(), num_loose_objects);
    let packs = list_packs();
    assert_eq!(packs.len(), 1, "{packs:?}");
    assert!(packs[0].ends_with(".pack"));
    // The objects can be read by a fresh store
    let reloaded_repo = test_repo
        .env
        .load_repo_at_head(repo.settings(), test_repo.repo_path());
    let reloaded_commit = reloaded_repo.store().get_commit(large_commit.id())?;
    let tree = reloaded_commit.tree();
    assert_eq!(block_on_stream(tree.entries()).count(), 200);
    Ok(())
}
