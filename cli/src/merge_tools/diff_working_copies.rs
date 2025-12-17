use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt as _;
use jj_lib::conflicts::ConflictMarkerStyle;
use jj_lib::fsmonitor::FsmonitorSettings;
use jj_lib::gitignore::GitIgnoreFile;
use jj_lib::local_working_copy::EolConversionMode;
use jj_lib::local_working_copy::ExecChangeSetting;
use jj_lib::local_working_copy::TreeState;
use jj_lib::local_working_copy::TreeStateError;
use jj_lib::local_working_copy::TreeStateSettings;
use jj_lib::matchers::EverythingMatcher;
use jj_lib::matchers::Matcher;
use jj_lib::matchers::NothingMatcher;
use jj_lib::merge::Diff;
use jj_lib::merged_tree::MergedTree;
use jj_lib::merged_tree::TreeDiffEntry;
use jj_lib::working_copy::CheckoutError;
use jj_lib::working_copy::SnapshotOptions;
use pollster::FutureExt as _;
use tempfile::TempDir;
use thiserror::Error;

use super::DiffEditError;
use super::external::ExternalToolError;

#[derive(Debug, Error)]
pub enum DiffCheckoutError {
    #[error("Failed to write directories to diff")]
    Checkout(#[from] CheckoutError),
    #[error("Error setting up temporary directory")]
    SetUpDir(#[source] std::io::Error),
    #[error(transparent)]
    TreeState(#[from] TreeStateError),
}

pub(crate) struct DiffWorkingCopies {
    _temp_dir: TempDir, // Temp dir will be deleted when this is dropped
    left: TreeState,
    right: TreeState,
    output: Option<TreeState>,
}

impl DiffWorkingCopies {
    pub fn set_left_readonly(&self) -> Result<(), ExternalToolError> {
        set_readonly_recursively(self.left.working_copy_path()).map_err(ExternalToolError::SetUpDir)
    }

    pub fn set_right_readonly(&self) -> Result<(), ExternalToolError> {
        set_readonly_recursively(self.right.working_copy_path())
            .map_err(ExternalToolError::SetUpDir)
    }

    pub fn temp_dir(&self) -> &Path {
        self._temp_dir.path()
    }

    pub fn to_command_variables(&self, relative: bool) -> HashMap<&'static str, String> {
        let mut left_wc_dir = self.left.working_copy_path();
        let mut right_wc_dir = self.right.working_copy_path();
        if relative {
            left_wc_dir = left_wc_dir
                .strip_prefix(self.temp_dir())
                .expect("path should be relative to temp_dir");
            right_wc_dir = right_wc_dir
                .strip_prefix(self.temp_dir())
                .expect("path should be relative to temp_dir");
        }
        let mut result = maplit::hashmap! {
            "left" => left_wc_dir.to_str().expect("temp_dir should be valid utf-8").to_owned(),
            "right" => right_wc_dir.to_str().expect("temp_dir should be valid utf-8").to_owned(),
        };
        if let Some(output_state) = &self.output {
            result.insert(
                "output",
                output_state
                    .working_copy_path()
                    .to_str()
                    .expect("temp_dir should be valid utf-8")
                    .to_owned(),
            );
        }
        result
    }
}

pub(crate) fn new_utf8_temp_dir(prefix: &str) -> io::Result<TempDir> {
    let temp_dir = tempfile::Builder::new().prefix(prefix).tempdir()?;
    if temp_dir.path().to_str().is_none() {
        // Not using .display() as we know the path contains unprintable character
        let message = format!("path {:?} is not valid UTF-8", temp_dir.path());
        return Err(io::Error::new(io::ErrorKind::InvalidData, message));
    }
    Ok(temp_dir)
}

pub(crate) fn set_readonly_recursively(path: &Path) -> Result<(), std::io::Error> {
    // Directory permission is unchanged since files under readonly directory cannot
    // be removed.
    let metadata = path.symlink_metadata()?;
    if metadata.is_dir() {
        for entry in path.read_dir()? {
            set_readonly_recursively(&entry?.path())?;
        }
        Ok(())
    } else if metadata.is_file() {
        let mut perms = metadata.permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(path, perms)
    } else {
        Ok(())
    }
}

/// How to prepare tree states from the working copy for a diff viewer/editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiffType {
    /// Prepare a left and right tree.
    TwoWay,
    /// Prepare left, right, and output trees.
    ThreeWay,
}

/// Check out the two trees in temporary directories. Only include changed files
/// in the sparse checkout patterns.
pub(crate) fn check_out_trees(
    trees: Diff<&MergedTree>,
    matcher: &dyn Matcher,
    diff_type: DiffType,
    conflict_marker_style: ConflictMarkerStyle,
) -> Result<DiffWorkingCopies, DiffCheckoutError> {
    let store = trees.before.store();
    let changed_files: Vec<_> = trees
        .before
        .diff_stream(trees.after, matcher)
        .map(|TreeDiffEntry { path, .. }| path)
        .collect()
        .block_on();

    let temp_dir = new_utf8_temp_dir("jj-diff-").map_err(DiffCheckoutError::SetUpDir)?;
    let temp_path = temp_dir.path();

    // Checkout a tree into our temp directory with the given prefix.
    let check_out = |name: &str, tree| -> Result<TreeState, DiffCheckoutError> {
        let wc_path = temp_path.join(name);
        let state_dir = temp_path.join(format!("{name}_state"));
        std::fs::create_dir(&wc_path).map_err(DiffCheckoutError::SetUpDir)?;
        std::fs::create_dir(&state_dir).map_err(DiffCheckoutError::SetUpDir)?;
        let tree_state_settings = TreeStateSettings {
            conflict_marker_style,
            eol_conversion_mode: EolConversionMode::None,
            exec_change_setting: ExecChangeSetting::Auto,
            fsmonitor_settings: FsmonitorSettings::None,
        };
        let mut state = TreeState::init(store.clone(), wc_path, state_dir, &tree_state_settings)?;
        state.set_sparse_patterns(changed_files.clone())?;
        state.check_out(tree)?;
        Ok(state)
    };

    let left = check_out("left", trees.before)?;
    let right = check_out("right", trees.after)?;
    let output = match diff_type {
        DiffType::TwoWay => None,
        DiffType::ThreeWay => Some(check_out("output", trees.after)?),
    };
    Ok(DiffWorkingCopies {
        _temp_dir: temp_dir,
        left,
        right,
        output,
    })
}

pub(crate) struct DiffEditWorkingCopies {
    pub working_copies: DiffWorkingCopies,
    instructions_path_to_cleanup: Option<PathBuf>,
}

impl DiffEditWorkingCopies {
    /// Checks out the trees, populates JJ_INSTRUCTIONS, and makes appropriate
    /// sides readonly.
    pub fn check_out(
        trees: Diff<&MergedTree>,
        matcher: &dyn Matcher,
        diff_type: DiffType,
        instructions: Option<&str>,
        conflict_marker_style: ConflictMarkerStyle,
    ) -> Result<Self, DiffEditError> {
        let working_copies = check_out_trees(trees, matcher, diff_type, conflict_marker_style)?;
        working_copies.set_left_readonly()?;
        if diff_type == DiffType::ThreeWay {
            working_copies.set_right_readonly()?;
        }
        let instructions_path_to_cleanup =
            Self::write_edit_instructions(&working_copies, instructions)?;
        Ok(Self {
            working_copies,
            instructions_path_to_cleanup,
        })
    }

    fn write_edit_instructions(
        working_copies: &DiffWorkingCopies,
        instructions: Option<&str>,
    ) -> Result<Option<PathBuf>, DiffEditError> {
        let Some(instructions) = instructions else {
            return Ok(None);
        };
        let (right_wc_path, output_wc_path) = match &working_copies.output {
            Some(output) => (
                Some(working_copies.right.working_copy_path()),
                output.working_copy_path(),
            ),
            None => (None, working_copies.right.working_copy_path()),
        };
        let output_instructions_path = output_wc_path.join("JJ-INSTRUCTIONS");
        // In the unlikely event that the file already exists, then the user will simply
        // not get any instructions.
        if output_instructions_path.exists() {
            return Ok(None);
        }
        let mut output_instructions_file =
            File::create(&output_instructions_path).map_err(ExternalToolError::SetUpDir)?;

        // Write out our experimental three-way merge instructions first.
        if let Some(right_wc_path) = right_wc_path {
            let mut right_instructions_file = File::create(right_wc_path.join("JJ-INSTRUCTIONS"))
                .map_err(ExternalToolError::SetUpDir)?;
            right_instructions_file
                .write_all(
                    b"\
The content of this pane should NOT be edited. Any edits will be
lost.

You are using the experimental 3-pane diff editor config. Some of
the following instructions may have been written with a 2-pane
diff editing in mind and be a little inaccurate.

",
                )
                .map_err(ExternalToolError::SetUpDir)?;
            right_instructions_file
                .write_all(instructions.as_bytes())
                .map_err(ExternalToolError::SetUpDir)?;
            // Note that some diff tools might not show this message and delete the contents
            // of the output dir instead. Meld does show this message.
            output_instructions_file
                .write_all(
                    b"\
Please make your edits in this pane.

You are using the experimental 3-pane diff editor config. Some of
the following instructions may have been written with a 2-pane
diff editing in mind and be a little inaccurate.

",
                )
                .map_err(ExternalToolError::SetUpDir)?;
        }
        // Now write the passed-in instructions.
        output_instructions_file
            .write_all(instructions.as_bytes())
            .map_err(ExternalToolError::SetUpDir)?;
        Ok(Some(output_instructions_path))
    }

    pub fn snapshot_results(
        self,
        base_ignores: Arc<GitIgnoreFile>,
    ) -> Result<MergedTree, DiffEditError> {
        if let Some(path) = self.instructions_path_to_cleanup {
            std::fs::remove_file(path).ok();
        }

        let diff_wc = self.working_copies;
        // Snapshot changes in the temporary output directory.
        let mut output_tree_state = diff_wc.output.unwrap_or(diff_wc.right);
        output_tree_state
            .snapshot(&SnapshotOptions {
                base_ignores,
                progress: None,
                start_tracking_matcher: &EverythingMatcher,
                force_tracking_matcher: &NothingMatcher,
                max_new_file_size: u64::MAX,
            })
            .block_on()?;
        Ok(output_tree_state.current_tree().clone())
    }
}
