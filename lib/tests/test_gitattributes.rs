use std::sync::Arc;

use itertools::Itertools as _;
use jj_lib::conflict_labels::ConflictLabels;
use jj_lib::gitattributes::FileLoader as _;
use jj_lib::gitattributes::TreeFileLoader;
use jj_lib::merge::Merge;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathComponent;
use jj_lib::tree::Tree;
use pollster::FutureExt as _;
use test_case::test_case;
use testutils::TestRepo;
use testutils::TestTreeBuilder;
use testutils::create_single_tree;
use testutils::create_tree;
use testutils::repo_path;
use tokio::io::AsyncReadExt as _;

#[test_case(|repo, path, contents| {
    vec![create_single_tree(repo, &[(path, contents)])]
}; "single file")]
#[test_case(|repo, path, contents| {
    let another_file = repo_path("a.txt");
    assert_ne!(another_file, path);
    vec![
        create_single_tree(repo, &[(path, contents)]),
        create_single_tree(repo, &[(another_file, "b\n")]),
        create_single_tree(repo, &[(another_file, "c\n")]),
    ]
}; "added file untouched in 3-way merge")]
#[test_case(|repo, path, contents| {
    let another_file = repo_path("a.txt");
    assert_ne!(another_file, path);
    let old_contents = "old contents\n";
    vec![
        create_single_tree(repo, &[(path, contents)]),
        create_single_tree(repo, &[
            (path, old_contents),
            (another_file, "b\n"),
        ]),
        create_single_tree(repo, &[
            (path, old_contents),
            (another_file, "c\n"),
        ]),
    ]
}; "existing file untouched in 3-way merge")]
#[test_case(|repo, path, contents| {
    let another_file = repo_path("a.txt");
    assert_ne!(another_file, path);
    vec![
        create_single_tree(repo, &[(another_file, "a\n")]),
        create_single_tree(repo, &[]),
        create_single_tree(repo, &[(path, contents)]),
    ]
}; "file added in 3-way merge")]
#[test_case(|repo, path, contents| {
    let old_contents = "old contents";
    assert_ne!(old_contents, contents);
    let another_file = repo_path("a.txt");
    assert_ne!(another_file, path);
    vec![
        create_single_tree(repo, &[
            (another_file, "a\n"),
            (path, old_contents),
        ]),
        create_single_tree(repo, &[(path, old_contents)]),
        create_single_tree(repo, &[(path, contents)]),
    ]
}; "file modified in 3-way merge")]
fn test_gitattr_tree_file_loader_trivially_resolved_to_a_file(
    tree_builder: impl FnOnce(&Arc<ReadonlyRepo>, &RepoPath, &str) -> Vec<Tree>,
) {
    let test_repo = TestRepo::init();
    let path = repo_path(".gitattributes");
    let contents = "a.txt text\n";

    let trees = Merge::from_vec(
        tree_builder(&test_repo.repo, path, contents)
            .into_iter()
            .map(|tree| tree.id().clone())
            .collect_vec(),
    );
    let tree = MergedTree::new(
        Arc::clone(test_repo.repo.store()),
        trees,
        ConflictLabels::unlabeled(),
    );
    let tree_file_loader = TreeFileLoader::new(tree);
    let mut buf = String::new();
    tree_file_loader
        .load(path)
        .block_on()
        .unwrap()
        .expect("The file should exist")
        .read_to_string(&mut buf)
        .block_on()
        .unwrap();
    assert_eq!(buf, contents);
}

#[test_case(|repo, path| {
    let contents = "a.txt text\n";
    let another_file = repo_path("a.txt");
    assert_ne!(another_file, path);
    vec![
        create_single_tree(repo, &[(another_file, "a\n")]),
        create_single_tree(repo, &[(path, contents)]),
        create_single_tree(repo, &[(path, contents)]),
    ]
}; "we remove the file")]
#[test_case(|repo, path| {
    let contents = "a.txt text\n";
    let another_file = repo_path("a.txt");
    assert_ne!(another_file, path);
    vec![
        create_single_tree(repo, &[(path, contents)]),
        create_single_tree(repo, &[(path, contents)]),
        create_single_tree(repo, &[(another_file, "c\n")]),
    ]
}; "they remove the file")]
#[test_case(|repo, path| {
    let path = path.join(RepoPathComponent::new("a.txt").unwrap());
    vec![create_single_tree(repo, &[(&path, "a\n")])]
}; "subtree")]
#[test_case(|repo, path| {
    let another_file = repo_path("a.txt");
    assert_ne!(path, another_file);
    let mut tree_builder = TestTreeBuilder::new(Arc::clone(repo.store()));
    tree_builder.file(another_file, "a.txt text\n");
    tree_builder.symlink(path, another_file.as_internal_file_string());
    vec![tree_builder.write_single_tree()]
}; "symlink")]
#[test_case(|repo, path| {
    let mut res = vec![];

    let child_path = path.join(RepoPathComponent::new("a.txt").unwrap());
    // On our side, we create a subtree at the given path.
    res.push(create_single_tree(repo, &[(&child_path, "a\n")]));

    // On the base side, nothing exists.
    res.push(create_single_tree(repo, &[]));

    // On their side, a symlink is created at the given path.
    let another_file = repo_path("a.txt");
    assert_ne!(path, another_file);
    let mut tree_builder = TestTreeBuilder::new(Arc::clone(repo.store()));
    tree_builder.file(another_file, "a.txt text\n");
    tree_builder.symlink(path, another_file.as_internal_file_string());
    res.push(tree_builder.write_single_tree());

    res
}; "subtree symlink conflict")]
fn test_gitattr_tree_file_loader_not_a_file(
    tree_builder: impl FnOnce(&Arc<ReadonlyRepo>, &RepoPath) -> Vec<Tree>,
) {
    let test_repo = TestRepo::init();
    let path = repo_path(".gitattributes");

    let trees = Merge::from_vec(
        tree_builder(&test_repo.repo, path)
            .into_iter()
            .map(|tree| tree.id().clone())
            .collect_vec(),
    );
    let tree = MergedTree::new(
        Arc::clone(test_repo.repo.store()),
        trees,
        ConflictLabels::unlabeled(),
    );
    let tree_file_loader = TreeFileLoader::new(tree);
    assert!(tree_file_loader.load(path).block_on().unwrap().is_none());
}

#[test_case(|repo, path| {
    vec![
        create_single_tree(repo, &[(path, "a.txt text\n")]),
        create_single_tree(repo, &[]),
        create_single_tree(repo, &[(path, "b.txt text\n")]),
    ]
}; "conflict file change")]
#[test_case(|repo, path| {
    let child_path = path.join(RepoPathComponent::new("a.txt").unwrap());
    vec![
        create_single_tree(repo, &[(path, "a.txt text\n")]),
        create_single_tree(repo, &[]),
        create_single_tree(repo, &[(&child_path, "a\n")]),
    ]
}; "file by us subtree by them")]
#[test_case(|repo, path| {
    let child_path = path.join(RepoPathComponent::new("a.txt").unwrap());
    vec![
        create_single_tree(repo, &[(&child_path, "a\n")]),
        create_single_tree(repo, &[]),
        create_single_tree(repo, &[(path, "a.txt text\n")]),
    ]
}; "file by them subtree by us")]
fn test_gitattr_tree_file_loader_conflicts_with_file_term(
    tree_builder: impl FnOnce(&Arc<ReadonlyRepo>, &RepoPath) -> Vec<Tree>,
) {
    let test_repo = TestRepo::init();
    let path = repo_path(".gitattributes");

    let trees = Merge::from_vec(
        tree_builder(&test_repo.repo, path)
            .into_iter()
            .map(|tree| tree.id().clone())
            .collect_vec(),
    );
    let tree = MergedTree::new(
        Arc::clone(test_repo.repo.store()),
        trees,
        ConflictLabels::unlabeled(),
    );
    let tree_file_loader = TreeFileLoader::new(tree);
    // If one of the conflict side is a file, we should find something.
    assert!(tree_file_loader.load(path).block_on().unwrap().is_some());
}

#[test]
fn test_gitattr_tree_file_loader_debug() {
    let test_repo = TestRepo::init();
    let path = repo_path(".gitattributes");
    let tree = create_tree(&test_repo.repo, &[(path, "a.txt text\n")]);
    let tree_file_loader = TreeFileLoader::new(tree);
    drop(format!("{tree_file_loader:?}"));
}
