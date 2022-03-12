// Copyright 2020 Google LLC
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

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use jujutsu_lib::backend::{BackendError, TreeId};
use jujutsu_lib::matchers::EverythingMatcher;
use jujutsu_lib::repo_path::RepoPath;
use jujutsu_lib::settings::UserSettings;
use jujutsu_lib::store::Store;
use jujutsu_lib::tree::{merge_trees, Tree};
use jujutsu_lib::working_copy::{CheckoutError, TreeState};
use tempfile::tempdir;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DiffEditError {
    #[error("The diff tool exited with a non-zero code")]
    DifftoolAborted,
    #[error("Failed to write directories to diff: {0:?}")]
    CheckoutError(CheckoutError),
    #[error("Internal error: {0:?}")]
    InternalBackendError(BackendError),
}

impl From<CheckoutError> for DiffEditError {
    fn from(err: CheckoutError) -> Self {
        DiffEditError::CheckoutError(err)
    }
}

impl From<BackendError> for DiffEditError {
    fn from(err: BackendError) -> Self {
        DiffEditError::InternalBackendError(err)
    }
}

fn check_out(
    store: Arc<Store>,
    wc_dir: PathBuf,
    state_dir: PathBuf,
    tree: &Tree,
) -> Result<TreeState, DiffEditError> {
    std::fs::create_dir(&wc_dir).unwrap();
    std::fs::create_dir(&state_dir).unwrap();
    let mut tree_state = TreeState::init(store, wc_dir, state_dir);
    tree_state.check_out(tree)?;
    Ok(tree_state)
}

fn set_readonly_recursively(path: &Path) {
    if path.is_dir() {
        for entry in path.read_dir().unwrap() {
            set_readonly_recursively(&entry.unwrap().path());
        }
    }
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(path, perms).unwrap();
}

pub fn edit_diff(
    settings: &UserSettings,
    left_tree: &Tree,
    right_tree: &Tree,
    instructions: &str,
) -> Result<TreeId, DiffEditError> {
    // First create partial Trees of only the subset of the left and right trees
    // that affect files changed between them.
    let store = left_tree.store();
    let mut left_tree_builder = store.tree_builder(store.empty_tree_id().clone());
    let mut right_tree_builder = store.tree_builder(store.empty_tree_id().clone());
    for (file_path, diff) in left_tree.diff(right_tree, &EverythingMatcher) {
        let (left_value, right_value) = diff.as_options();
        if let Some(value) = left_value {
            left_tree_builder.set(file_path.clone(), value.clone());
        }
        if let Some(value) = right_value {
            right_tree_builder.set(file_path.clone(), value.clone());
        }
    }
    let left_partial_tree_id = left_tree_builder.write_tree();
    let right_partial_tree_id = right_tree_builder.write_tree();
    let left_partial_tree = store.get_tree(&RepoPath::root(), &left_partial_tree_id)?;
    let right_partial_tree = store.get_tree(&RepoPath::root(), &right_partial_tree_id)?;

    // Check out the two partial trees in temporary directories.
    let temp_dir = tempdir().unwrap();
    let left_wc_dir = temp_dir.path().join("left");
    let left_state_dir = temp_dir.path().join("left_state");
    let right_wc_dir = temp_dir.path().join("right");
    let right_state_dir = temp_dir.path().join("right_state");
    check_out(
        store.clone(),
        left_wc_dir.clone(),
        left_state_dir,
        &left_partial_tree,
    )?;
    set_readonly_recursively(&left_wc_dir);
    let mut right_tree_state = check_out(
        store.clone(),
        right_wc_dir.clone(),
        right_state_dir,
        &right_partial_tree,
    )?;
    let instructions_path = right_wc_dir.join("JJ-INSTRUCTIONS");
    // In the unlikely event that the file already exists, then the user will simply
    // not get any instructions.
    let add_instructions = !instructions.is_empty() && !instructions_path.exists();
    if add_instructions {
        let mut file = File::create(&instructions_path).unwrap();
        file.write_all(instructions.as_bytes()).unwrap();
    }

    // TODO: Make this configuration have a table of possible editors and detect the
    // best one here.
    let editor_binary = settings
        .config()
        .get_str("ui.diff-editor")
        .unwrap_or_else(|_| "meld".to_string());
    // Start a diff editor on the two directories.
    let exit_status = Command::new(&editor_binary)
        .arg(&left_wc_dir)
        .arg(&right_wc_dir)
        .status()
        .expect("failed to run diff editor");
    if !exit_status.success() {
        return Err(DiffEditError::DifftoolAborted);
    }
    if add_instructions {
        std::fs::remove_file(instructions_path).ok();
    }

    // Create a Tree based on the initial right tree, applying the changes made to
    // that directory by the diff editor.
    let new_right_partial_tree_id = right_tree_state.write_tree();
    let new_right_partial_tree = store.get_tree(&RepoPath::root(), &new_right_partial_tree_id)?;
    let new_tree_id = merge_trees(right_tree, &right_partial_tree, &new_right_partial_tree)?;

    Ok(new_tree_id)
}
