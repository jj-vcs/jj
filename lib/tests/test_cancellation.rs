// Copyright 2026 The Jujutsu Authors
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
use std::sync::Mutex;
use std::sync::Arc;
use std::str;

use assert_matches::assert_matches;
use indoc::indoc;
use once_cell::sync::Lazy;
use pollster::FutureExt as _;
use thiserror::Error;

use jj_lib::backend::{CommitId, FileId};
use jj_lib::git_backend::GitBackend;
use jj_lib::git::{self, GitImportOptions};
use jj_lib::working_copy::SnapshotError;
use jj_lib::matchers::EverythingMatcher;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::{RepoPath, RepoPathBuf};
use jj_lib::store::Store;
use jj_lib::transaction::Transaction;
use jj_lib::fix::{fix_files, FileToFix, FixError, ParallelFileFixer};

use testutils::{
    create_tree, read_file, repo_path, write_random_commit,
    write_random_commit_with_parents, TestRepo, TestRepoBackend, TestResult, TestWorkspace,
    CommitBuilderExt as _,
};

// Global lock to ensure cancellation tests run sequentially and do not interfere
static TEST_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

// Git test helper functions
fn get_git_backend(repo: &Arc<jj_lib::repo::ReadonlyRepo>) -> &GitBackend {
    repo.store().backend_impl().unwrap()
}

fn get_git_repo(repo: &Arc<jj_lib::repo::ReadonlyRepo>) -> gix::Repository {
    get_git_repo_from_backend(get_git_backend(repo))
}

fn get_git_repo_from_backend(backend: &GitBackend) -> gix::Repository {
    backend.git_repo()
}

fn default_import_options() -> GitImportOptions {
    GitImportOptions {
        auto_local_bookmark: false,
        abandon_unreachable_commits: true,
        remote_auto_track_bookmarks: HashMap::new(),
    }
}

fn empty_git_commit(
    git_repo: &gix::Repository,
    ref_name: &str,
    parents: &[gix::ObjectId],
) -> gix::ObjectId {
    let empty_tree_id = git_repo.empty_tree().id().detach();
    testutils::git::write_commit(
        git_repo,
        ref_name,
        empty_tree_id,
        &format!("random commit {}", rand::random::<u32>()),
        parents,
    )
}

// Fix test helper functions
type ReplacementKey = (RepoPathBuf, Option<Vec<u8>>, Vec<u8>);

#[derive(Clone, Debug)]
struct TestFileFixer {
    replacements: HashMap<ReplacementKey, Vec<u8>>,
}

impl TestFileFixer {
    fn new() -> Self {
        Self {
            replacements: HashMap::new(),
        }
    }

    fn add_replacement(
        &mut self,
        repo_path: &RepoPath,
        base_content: Option<&[u8]>,
        old_content: impl AsRef<[u8]>,
        new_content: impl AsRef<[u8]>,
    ) {
        self.replacements.insert(
            (
                repo_path.to_owned(),
                base_content.map(|c| c.to_vec()),
                old_content.as_ref().to_vec(),
            ),
            new_content.as_ref().to_vec(),
        );
    }

    fn fix_file(&mut self, store: &Store, file_to_fix: &FileToFix) -> Result<FileId, FixError> {
        let old_content = read_file(store, &file_to_fix.repo_path, &file_to_fix.file_id);
        let base_content = file_to_fix
            .base_file_id
            .as_ref()
            .map(|base_file_id| read_file(store, &file_to_fix.repo_path, base_file_id));

        let key = (
            file_to_fix.repo_path.clone(),
            base_content,
            old_content.clone(),
        );
        let Some(new_content) = self.replacements.remove(&key) else {
            return Err(make_fix_content_error(&format!(
                indoc! {"
                    Unexpected fix request:
                    path: {}
                    old_content: {:?}
                "},
                file_to_fix.repo_path.as_internal_file_string(),
                String::from_utf8_lossy(&old_content)
            )));
        };

        let new_file_id = store
            .write_file(&file_to_fix.repo_path, &mut new_content.as_slice())
            .block_on()?;
        Ok(new_file_id)
    }
}

impl jj_lib::fix::FileFixer for TestFileFixer {
    fn fix_files<'a>(
        &mut self,
        store: &Store,
        files_to_fix: &'a std::collections::HashSet<FileToFix>,
    ) -> Result<HashMap<&'a FileToFix, FileId>, FixError> {
        let mut changed_files = HashMap::new();
        for file_to_fix in files_to_fix {
            let new_file_id = self.fix_file(store, file_to_fix)?;
            changed_files.insert(file_to_fix, new_file_id);
        }
        assert!(self.replacements.is_empty());
        Ok(changed_files)
    }
}

#[derive(Error, Debug)]
#[error("Forced failure: {0}")]
struct MyFixerError(String);

fn make_fix_content_error(message: &str) -> FixError {
    FixError::FixContent(Box::new(MyFixerError(message.into())))
}

fn fix_file(store: &Store, file_to_fix: &FileToFix) -> Result<Option<FileId>, FixError> {
    let old_content = read_file(store, &file_to_fix.repo_path, &file_to_fix.file_id);

    if let Some(rest) = old_content.strip_prefix(b"fixme:") {
        let new_content = rest.to_ascii_uppercase();
        let new_file_id = store
            .write_file(&file_to_fix.repo_path, &mut new_content.as_slice())
            .block_on()?;
        Ok(Some(new_file_id))
    } else if let Some(rest) = old_content.strip_prefix(b"error:") {
        Err(make_fix_content_error(str::from_utf8(rest).unwrap()))
    } else {
        Ok(None)
    }
}

fn create_commit(tx: &mut Transaction, parents: Vec<CommitId>, tree: MergedTree) -> CommitId {
    tx.repo_mut()
        .new_commit(parents, tree)
        .write_unwrap()
        .id()
        .clone()
}

#[test]
fn test_snapshot_cancellation() -> TestResult {
    let _lock = TEST_LOCK.lock().unwrap();
    let mut test_workspace = TestWorkspace::init();

    // Request cancellation before snapshotting
    jj_lib::cancellation::request_cancellation();

    // Perform snapshotting, which should fail with SnapshotError::Other
    let result = test_workspace.snapshot();
    assert_matches!(result, Err(SnapshotError::Other { .. }));

    // Reset cancellation to avoid affecting other tests
    jj_lib::cancellation::reset_cancellation();

    Ok(())
}

#[test]
fn test_transform_commits_cancellation() -> TestResult {
    let _lock = TEST_LOCK.lock().unwrap();
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let commit_a = write_random_commit(tx.repo_mut());
    let commit_b = write_random_commit_with_parents(tx.repo_mut(), &[&commit_a]);

    // Request cancellation before transforming
    jj_lib::cancellation::request_cancellation();

    let result = tx.repo_mut()
        .transform_descendants(vec![commit_b.id().clone()], async |rewriter| {
            let _unused = rewriter.rebase().await?.write().await?;
            Ok(())
        })
        .block_on();

    // Reset cancellation to avoid affecting other tests
    jj_lib::cancellation::reset_cancellation();

    assert_matches!(result, Err(jj_lib::backend::BackendError::Interrupted));
    Ok(())
}

#[test]
fn test_git_import_refs_cancellation() -> TestResult {
    let _lock = TEST_LOCK.lock().unwrap();
    let _settings = testutils::user_settings();
    let test_repo = TestRepo::init_with_backend(TestRepoBackend::Git);
    let repo = &test_repo.repo;
    let git_repo = get_git_repo(repo);

    // Write a bookmark
    let _git_commit = empty_git_commit(&git_repo, "refs/heads/main", &[]);

    // Request cancellation before importing
    jj_lib::cancellation::request_cancellation();

    let mut tx = repo.start_transaction();
    let result = git::import_refs(tx.repo_mut(), &default_import_options()).block_on();

    // Reset cancellation to avoid affecting other tests
    jj_lib::cancellation::reset_cancellation();

    assert_matches!(
        result,
        Err(git::GitImportError::Backend(jj_lib::backend::BackendError::Interrupted))
    );
    Ok(())
}

#[test]
fn test_git_export_refs_cancellation() -> TestResult {
    let _lock = TEST_LOCK.lock().unwrap();
    let test_repo = TestRepo::init_with_backend(TestRepoBackend::Git);
    let repo = &test_repo.repo;

    // Request cancellation before exporting
    jj_lib::cancellation::request_cancellation();

    let mut tx = repo.start_transaction();
    let result = git::export_refs(tx.repo_mut());

    // Reset cancellation to avoid affecting other tests
    jj_lib::cancellation::reset_cancellation();

    assert_matches!(
        result,
        Err(git::GitExportError::Git(err)) if err.to_string() == "Interrupted"
    );
    Ok(())
}

#[test]
fn test_fix_files_cancellation() -> TestResult {
    let _lock = TEST_LOCK.lock().unwrap();
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path1 = repo_path("file1");
    let tree1 = create_tree(repo, &[(path1, "unformatted")]);
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    let root_commits = vec![commit_a.clone()];
    let mut file_fixer = TestFileFixer::new();
    let include_unchanged_files = false;
    file_fixer.add_replacement(path1, None, b"unformatted", b"Formatted");

    // Request cancellation before fixing
    jj_lib::cancellation::request_cancellation();

    let result = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut file_fixer,
    )
    .block_on();

    // Reset cancellation to avoid affecting other tests
    jj_lib::cancellation::reset_cancellation();

    assert_matches!(
        result,
        Err(FixError::Backend(jj_lib::backend::BackendError::Interrupted))
    );
    Ok(())
}

#[test]
fn test_parallel_fixer_cancellation() -> TestResult {
    let _lock = TEST_LOCK.lock().unwrap();
    let test_repo = TestRepo::init();
    let repo = &test_repo.repo;

    let mut tx = repo.start_transaction();
    let path1 = repo_path("file1");
    let tree1 = create_tree(repo, &[(path1, "fixme:content")]);
    let commit_a = create_commit(&mut tx, vec![repo.store().root_commit_id().clone()], tree1);

    let root_commits = vec![commit_a.clone()];
    let include_unchanged_files = false;
    let mut parallel_fixer = ParallelFileFixer::new(fix_file);

    // Request cancellation before fixing
    jj_lib::cancellation::request_cancellation();

    let result = fix_files(
        root_commits,
        &EverythingMatcher,
        include_unchanged_files,
        tx.repo_mut(),
        &mut parallel_fixer,
    )
    .block_on();

    // Reset cancellation to avoid affecting other tests
    jj_lib::cancellation::reset_cancellation();

    assert_matches!(
        result,
        Err(FixError::Backend(jj_lib::backend::BackendError::Interrupted))
    );
    Ok(())
}
