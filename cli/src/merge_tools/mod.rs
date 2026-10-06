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

mod builtin;
mod diff_working_copies;
mod external;

use std::sync::Arc;

use bstr::ByteSlice as _;
use futures::future::try_join_all;
use jj_lib::backend::BackendError;
use jj_lib::backend::CopyId;
use jj_lib::backend::FileId;
use jj_lib::backend::MergedTreeValue;
use jj_lib::backend::MergedTreeValueExt as _;
use jj_lib::backend::TreeValue;
use jj_lib::config::ConfigGetError;
use jj_lib::config::ConfigGetResultExt as _;
use jj_lib::config::ConfigNamePathBuf;
use jj_lib::conflicts;
use jj_lib::conflicts::ConflictMarkerStyle;
use jj_lib::conflicts::ConflictMaterializeOptions;
use jj_lib::conflicts::MaterializedFileConflictValue;
use jj_lib::conflicts::choose_materialized_conflict_marker_len;
use jj_lib::conflicts::materialize_merge_result_to_bytes;
use jj_lib::conflicts::try_materialize_file_conflict_value;
use jj_lib::gitignore::GitIgnoreFile;
use jj_lib::matchers::Matcher;
use jj_lib::merge::Diff;
use jj_lib::merge::Merge;
use jj_lib::merged_tree::MergedTree;
use jj_lib::merged_tree_builder::MergedTreeBuilder;
use jj_lib::repo_path::InvalidRepoPathError;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::settings::UserSettings;
use jj_lib::store::Store;
use jj_lib::ui_path::RepoPathUiConverter;
use jj_lib::working_copy::SnapshotError;
use thiserror::Error;

use self::builtin::BuiltinToolError;
use self::builtin::edit_diff_builtin;
use self::builtin::edit_merge_builtin;
use self::diff_working_copies::DiffCheckoutError;
pub use self::external::DiffToolMode;
pub use self::external::ExternalMergeTool;
use self::external::ExternalToolError;
use self::external::edit_diff_external;
pub use self::external::generate_diff;
pub use self::external::invoke_external_diff;
use crate::config::CommandNameAndArgs;
use crate::description_util::LineNumber;
use crate::description_util::TextEditError;
use crate::description_util::TextEditor;
use crate::ui::Ui;

const BUILTIN_EDITOR_NAME: &str = ":builtin";
const TEXT_EDITOR_TOOL_NAME: &str = ":editor";
const OURS_TOOL_NAME: &str = ":ours";
const THEIRS_TOOL_NAME: &str = ":theirs";

#[derive(Debug, Error)]
pub enum DiffEditError {
    #[error(transparent)]
    InternalTool(#[from] Box<BuiltinToolError>),
    #[error(transparent)]
    ExternalTool(#[from] ExternalToolError),
    #[error(transparent)]
    DiffCheckoutError(#[from] DiffCheckoutError),
    #[error("Failed to snapshot changes")]
    Snapshot(#[from] SnapshotError),
    #[error(transparent)]
    Config(#[from] ConfigGetError),
}

#[derive(Debug, Error)]
pub enum DiffGenerateError {
    #[error(transparent)]
    ExternalTool(#[from] ExternalToolError),
    #[error(transparent)]
    DiffCheckoutError(#[from] DiffCheckoutError),
}

#[derive(Debug, Error)]
pub enum ConflictResolveError {
    #[error(transparent)]
    InternalTool(#[from] Box<BuiltinToolError>),
    #[error(transparent)]
    ExternalTool(#[from] ExternalToolError),
    #[error(transparent)]
    TextEditor(#[from] TextEditError),
    #[error(transparent)]
    InvalidRepoPath(#[from] InvalidRepoPathError),
    #[error("Couldn't find the path {0:?} in this revision")]
    PathNotFound(RepoPathBuf),
    #[error("Couldn't find any conflicts at {0:?} in this revision")]
    NotAConflict(RepoPathBuf),
    #[error(
        "Only conflicts that involve normal files (not symlinks, etc.) are supported. Conflict \
         summary for {path:?}:\n{summary}",
        summary = summary.trim_end()
    )]
    NotNormalFiles { path: RepoPathBuf, summary: String },
    #[error("The conflict at {path:?} has {sides} sides. At most 2 sides are supported.")]
    ConflictTooComplicated { path: RepoPathBuf, sides: usize },
    #[error("{path:?} has conflicts in executable bit\n{summary}", summary = summary.trim_end())]
    ExecutableConflict { path: RepoPathBuf, summary: String },
    #[error(
        "The output file is either unchanged or empty after the editor quit (run with --debug to \
         see the exact invocation)."
    )]
    EmptyOrUnchanged,
    #[error(transparent)]
    Backend(#[from] jj_lib::backend::BackendError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Error)]
#[error("Stopped due to error after resolving {resolved_count} conflicts")]
pub struct MergeToolPartialResolutionError {
    pub source: ConflictResolveError,
    pub resolved_count: usize,
}

#[derive(Debug, Error)]
pub enum MergeToolConfigError {
    #[error(transparent)]
    Config(#[from] ConfigGetError),
    #[error("The tool `{tool_name}` cannot be used as a merge tool with `jj resolve`")]
    MergeArgsNotConfigured { tool_name: String },
    #[error("The tool `{tool_name}` cannot be used as a diff editor")]
    EditArgsNotConfigured { tool_name: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MergeTool {
    Builtin,
    /// Edits the materialized conflict markers in the user's text editor.
    /// Unlike the other tools, this supports conflicts with any number of
    /// sides.
    TextEditor(TextEditor),
    Ours,
    Theirs,
    // Boxed because ExternalMergeTool is big compared to the Builtin variant.
    External(Box<ExternalMergeTool>),
}

impl MergeTool {
    fn external(tool: ExternalMergeTool) -> Self {
        Self::External(Box::new(tool))
    }

    /// Resolves builtin merge tool names or loads external tool options from
    /// `[merge-tools.<name>]`.
    fn get_tool_config(
        settings: &UserSettings,
        name: &str,
    ) -> Result<Option<Self>, MergeToolConfigError> {
        match name {
            BUILTIN_EDITOR_NAME => Ok(Some(Self::Builtin)),
            TEXT_EDITOR_TOOL_NAME => {
                Ok(Some(Self::TextEditor(TextEditor::from_settings(settings)?)))
            }
            OURS_TOOL_NAME => Ok(Some(Self::Ours)),
            THEIRS_TOOL_NAME => Ok(Some(Self::Theirs)),
            _ => Ok(get_external_tool_config(settings, name)?.map(Self::external)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DiffEditTool {
    Builtin,
    // Boxed because ExternalMergeTool is big compared to the Builtin variant.
    External(Box<ExternalMergeTool>),
}

impl DiffEditTool {
    fn external(tool: ExternalMergeTool) -> Self {
        Self::External(Box::new(tool))
    }

    /// Resolves builtin merge tool name or loads external tool options from
    /// `[merge-tools.<name>]`.
    fn get_tool_config(
        settings: &UserSettings,
        name: &str,
    ) -> Result<Option<Self>, MergeToolConfigError> {
        match name {
            BUILTIN_EDITOR_NAME => Ok(Some(Self::Builtin)),
            _ => Ok(get_external_tool_config(settings, name)?.map(Self::external)),
        }
    }
}

/// Finds the appropriate tool for diff editing or merges
fn editor_args_from_settings(
    ui: &Ui,
    settings: &UserSettings,
    key: &'static str,
) -> Result<CommandNameAndArgs, ConfigGetError> {
    // TODO: Make this configuration have a table of possible editors and detect the
    // best one here.
    if let Some(args) = settings.get(key).optional()? {
        Ok(args)
    } else {
        let default_editor = BUILTIN_EDITOR_NAME;
        writeln!(
            ui.hint_default(),
            "Using default editor '{default_editor}'; run `jj config set --user {key} :builtin` \
             to disable this message."
        )
        .ok();
        Ok(default_editor.into())
    }
}

/// List configured merge tools (diff editors, diff tools, merge editors)
pub fn configured_merge_tools(settings: &UserSettings) -> impl Iterator<Item = &str> {
    settings.table_keys("merge-tools")
}

/// Loads external diff/merge tool options from `[merge-tools.<name>]`.
pub fn get_external_tool_config(
    settings: &UserSettings,
    name: &str,
) -> Result<Option<ExternalMergeTool>, ConfigGetError> {
    let full_name = ConfigNamePathBuf::from_iter(["merge-tools", name]);
    let Some(mut tool) = settings.get::<ExternalMergeTool>(&full_name).optional()? else {
        return Ok(None);
    };
    if tool.program.is_empty() {
        tool.program = name.to_owned();
    }
    Ok(Some(tool))
}

/// Configured diff editor.
#[derive(Clone, Debug)]
pub struct DiffEditor {
    tool: DiffEditTool,
    base_ignores: Arc<GitIgnoreFile>,
    use_instructions: bool,
    conflict_marker_style: ConflictMarkerStyle,
}

impl DiffEditor {
    /// Creates diff editor of the given name, and loads parameters from the
    /// settings.
    pub fn with_name(
        name: &str,
        settings: &UserSettings,
        base_ignores: Arc<GitIgnoreFile>,
        conflict_marker_style: ConflictMarkerStyle,
    ) -> Result<Self, MergeToolConfigError> {
        let tool = DiffEditTool::get_tool_config(settings, name)?
            .unwrap_or_else(|| DiffEditTool::external(ExternalMergeTool::with_program(name)));
        Self::new_inner(name, tool, settings, base_ignores, conflict_marker_style)
    }

    /// Loads the default diff editor from the settings.
    pub fn from_settings(
        ui: &Ui,
        settings: &UserSettings,
        base_ignores: Arc<GitIgnoreFile>,
        conflict_marker_style: ConflictMarkerStyle,
    ) -> Result<Self, MergeToolConfigError> {
        let args = editor_args_from_settings(ui, settings, "ui.diff-editor")?;
        let tool = if let Some(name) = args.as_str() {
            DiffEditTool::get_tool_config(settings, name)?
        } else {
            None
        }
        .unwrap_or_else(|| DiffEditTool::external(ExternalMergeTool::with_edit_args(&args)));
        Self::new_inner(&args, tool, settings, base_ignores, conflict_marker_style)
    }

    fn new_inner(
        name: impl ToString,
        tool: DiffEditTool,
        settings: &UserSettings,
        base_ignores: Arc<GitIgnoreFile>,
        conflict_marker_style: ConflictMarkerStyle,
    ) -> Result<Self, MergeToolConfigError> {
        if let DiffEditTool::External(mergetool) = &tool
            && mergetool.edit_args.is_empty()
        {
            return Err(MergeToolConfigError::EditArgsNotConfigured {
                tool_name: name.to_string(),
            });
        }
        Ok(Self {
            tool,
            base_ignores,
            use_instructions: settings.get_bool("ui.diff-instructions")?,
            conflict_marker_style,
        })
    }

    /// Starts a diff editor on the two directories.
    pub async fn edit(
        &self,
        trees: Diff<&MergedTree>,
        matcher: &dyn Matcher,
        format_instructions: impl FnOnce() -> String,
    ) -> Result<MergedTree, DiffEditError> {
        match &self.tool {
            DiffEditTool::Builtin => {
                Ok(
                    edit_diff_builtin(trees, matcher, self.conflict_marker_style)
                        .await
                        .map_err(Box::new)?,
                )
            }
            DiffEditTool::External(editor) => {
                let instructions = self.use_instructions.then(format_instructions);
                edit_diff_external(
                    editor,
                    trees,
                    matcher,
                    instructions.as_deref(),
                    self.base_ignores.clone(),
                    self.conflict_marker_style,
                )
                .await
            }
        }
    }
}

/// A file to be merged by a merge tool.
struct MergeToolFile {
    repo_path: RepoPathBuf,
    conflict: MergedTreeValue,
    file: MaterializedFileConflictValue,
}

impl MergeToolFile {
    async fn from_tree_and_path(
        tree: &MergedTree,
        repo_path: &RepoPath,
    ) -> Result<Self, ConflictResolveError> {
        let conflict = match tree.path_value(repo_path).await?.into_resolved() {
            Err(conflict) => conflict,
            Ok(Some(_)) => return Err(ConflictResolveError::NotAConflict(repo_path.to_owned())),
            Ok(None) => return Err(ConflictResolveError::PathNotFound(repo_path.to_owned())),
        };
        let file =
            try_materialize_file_conflict_value(tree.store(), repo_path, &conflict, tree.labels())
                .await?
                .ok_or_else(|| ConflictResolveError::NotNormalFiles {
                    path: repo_path.to_owned(),
                    summary: conflict.describe(tree.labels()),
                })?;
        if file.executable.is_none() {
            return Err(ConflictResolveError::ExecutableConflict {
                path: repo_path.to_owned(),
                summary: conflict.describe(tree.labels()),
            });
        }
        Ok(Self {
            repo_path: repo_path.to_owned(),
            conflict,
            file,
        })
    }

    /// Returns an error if the conflict has more than two sides, since most
    /// merge tools only support 3-way merges.
    fn check_two_sided(&self) -> Result<(), ConflictResolveError> {
        let sides = self.file.ids.num_sides();
        if sides > 2 {
            return Err(ConflictResolveError::ConflictTooComplicated {
                path: self.repo_path.clone(),
                sides,
            });
        }
        Ok(())
    }

    /// Builds the tree value to store for this file given the file ids
    /// produced by a merge tool. If the file is still conflicted, the
    /// executable bits are left unchanged.
    fn new_tree_value(&self, new_file_ids: Merge<Option<FileId>>) -> MergedTreeValue {
        match new_file_ids.into_resolved() {
            Ok(file_id) => {
                let executable = self.file.executable.expect("should have been resolved");
                Merge::resolved(file_id.map(|id| TreeValue::File {
                    id,
                    executable,
                    copy_id: CopyId::placeholder(),
                }))
            }
            Err(file_ids) => self.conflict.with_new_file_ids(&file_ids),
        }
    }
}

/// Configured 3-way merge editor.
#[derive(Clone, Debug)]
pub struct MergeEditor {
    tool: MergeTool,
    path_converter: RepoPathUiConverter,
    conflict_marker_style: ConflictMarkerStyle,
}

impl MergeEditor {
    /// Creates 3-way merge editor of the given name, and loads parameters from
    /// the settings.
    pub fn with_name(
        name: &str,
        settings: &UserSettings,
        path_converter: RepoPathUiConverter,
        conflict_marker_style: ConflictMarkerStyle,
    ) -> Result<Self, MergeToolConfigError> {
        let tool = MergeTool::get_tool_config(settings, name)?
            .unwrap_or_else(|| MergeTool::external(ExternalMergeTool::with_program(name)));
        Self::new_inner(name, tool, path_converter, conflict_marker_style)
    }

    /// Loads the default 3-way merge editor from the settings.
    pub fn from_settings(
        ui: &Ui,
        settings: &UserSettings,
        path_converter: RepoPathUiConverter,
        conflict_marker_style: ConflictMarkerStyle,
    ) -> Result<Self, MergeToolConfigError> {
        let args = editor_args_from_settings(ui, settings, "ui.merge-editor")?;
        let tool = if let Some(name) = args.as_str() {
            MergeTool::get_tool_config(settings, name)?
        } else {
            None
        }
        .unwrap_or_else(|| MergeTool::external(ExternalMergeTool::with_merge_args(&args)));
        Self::new_inner(&args, tool, path_converter, conflict_marker_style)
    }

    fn new_inner(
        name: impl ToString,
        tool: MergeTool,
        path_converter: RepoPathUiConverter,
        conflict_marker_style: ConflictMarkerStyle,
    ) -> Result<Self, MergeToolConfigError> {
        if let MergeTool::External(mergetool) = &tool
            && mergetool.merge_args.is_empty()
        {
            return Err(MergeToolConfigError::MergeArgsNotConfigured {
                tool_name: name.to_string(),
            });
        }
        Ok(Self {
            tool,
            path_converter,
            conflict_marker_style,
        })
    }

    /// Starts a merge editor for the specified files.
    pub async fn edit_files(
        &self,
        ui: &Ui,
        tree: &MergedTree,
        repo_paths: &[&RepoPath],
    ) -> Result<(MergedTree, Option<MergeToolPartialResolutionError>), ConflictResolveError> {
        let merge_tool_files: Vec<MergeToolFile> = try_join_all(
            repo_paths
                .iter()
                .map(|&repo_path| MergeToolFile::from_tree_and_path(tree, repo_path)),
        )
        .await?;
        // Most tools only support 3-way merges
        if !matches!(self.tool, MergeTool::TextEditor(_)) {
            for merge_tool_file in &merge_tool_files {
                merge_tool_file.check_two_sided()?;
            }
        }

        match &self.tool {
            MergeTool::Builtin => {
                let tree = edit_merge_builtin(tree, &merge_tool_files)
                    .await
                    .map_err(Box::new)?;
                Ok((tree, None))
            }
            MergeTool::TextEditor(editor) => {
                resolve_files_one_by_one(
                    ui,
                    &self.path_converter,
                    tree,
                    &merge_tool_files,
                    async |merge_tool_file, tree_builder| {
                        edit_conflict_markers_in_text_editor(
                            editor,
                            tree.store(),
                            merge_tool_file,
                            self.conflict_marker_style,
                            tree_builder,
                        )
                        .await
                    },
                )
                .await
            }
            MergeTool::Ours => {
                let tree = pick_conflict_side(tree, &merge_tool_files, 0).await?;
                Ok((tree, None))
            }
            MergeTool::Theirs => {
                let tree = pick_conflict_side(tree, &merge_tool_files, 1).await?;
                Ok((tree, None))
            }
            MergeTool::External(editor) => {
                external::run_mergetool_external(
                    ui,
                    &self.path_converter,
                    editor,
                    tree,
                    &merge_tool_files,
                    self.conflict_marker_style,
                )
                .await
            }
        }
    }
}

/// Resolves conflicts in the given files one by one by calling
/// `resolve_file`, which should record the result in the tree builder passed
/// to it. If resolving a file fails after at least one file has been
/// resolved, the partially-resolved tree is returned along with the error so
/// that the caller can save the resolved files.
async fn resolve_files_one_by_one(
    ui: &Ui,
    path_converter: &RepoPathUiConverter,
    tree: &MergedTree,
    merge_tool_files: &[MergeToolFile],
    resolve_file: impl AsyncFn(
        &MergeToolFile,
        &mut MergedTreeBuilder,
    ) -> Result<(), ConflictResolveError>,
) -> Result<(MergedTree, Option<MergeToolPartialResolutionError>), ConflictResolveError> {
    let mut tree_builder = MergedTreeBuilder::new(tree.clone());
    let mut partial_resolution_error = None;
    for (i, merge_tool_file) in merge_tool_files.iter().enumerate() {
        writeln!(
            ui.status(),
            "Resolving conflicts in: {}",
            path_converter.format_file_path(&merge_tool_file.repo_path)
        )?;
        match resolve_file(merge_tool_file, &mut tree_builder).await {
            Ok(()) => {}
            Err(err) if i == 0 => {
                // If the first resolution fails, just return the error normally
                return Err(err);
            }
            Err(err) => {
                // Some conflicts were already resolved, so we should return an error with the
                // partially-resolved tree so that the caller can save the resolved files.
                partial_resolution_error = Some(MergeToolPartialResolutionError {
                    source: err,
                    resolved_count: i,
                });
                break;
            }
        }
    }
    let new_tree = tree_builder.write_tree().await?;
    Ok((new_tree, partial_resolution_error))
}

/// Materializes the conflict with markers, lets the user edit it in their
/// text editor, and parses the result back. Any conflict markers left in the
/// file are kept as a (possibly partially resolved) conflict.
async fn edit_conflict_markers_in_text_editor(
    editor: &TextEditor,
    store: &Store,
    merge_tool_file: &MergeToolFile,
    conflict_marker_style: ConflictMarkerStyle,
    tree_builder: &mut MergedTreeBuilder,
) -> Result<(), ConflictResolveError> {
    let MergeToolFile {
        repo_path, file, ..
    } = merge_tool_file;

    let conflict_marker_len = choose_materialized_conflict_marker_len(&file.contents);
    let options = ConflictMaterializeOptions {
        marker_style: conflict_marker_style,
        marker_len: Some(conflict_marker_len),
        merge: store.merge_options().clone(),
    };
    let initial_content = materialize_merge_result_to_bytes(&file.contents, &file.labels, &options);

    // Name the temporary file after the conflicted file so the editor can pick
    // the right syntax highlighting.
    let temp_dir = tempfile::Builder::new()
        .prefix("jj-resolve-")
        .tempdir()
        .map_err(ExternalToolError::SetUpDir)?;
    let file_name = match repo_path.components().next_back() {
        Some(name) => name.to_fs_name().map_err(|err| err.with_path(repo_path))?,
        None => "output",
    };
    let temp_path = temp_dir.path().join(file_name);
    std::fs::write(&temp_path, &initial_content).map_err(ExternalToolError::SetUpDir)?;

    // Open the editor at the first conflict so the user doesn't have to look
    // for it.
    let start_marker = b"<".repeat(conflict_marker_len);
    let first_conflict_line = initial_content
        .lines()
        .position(|line| {
            line.starts_with(&start_marker)
                && line
                    .get(conflict_marker_len)
                    .is_none_or(|b| b.is_ascii_whitespace())
        })
        .and_then(|index| u32::try_from(index + 1).ok())
        .and_then(LineNumber::new)
        .unwrap_or(LineNumber::MIN);
    editor.edit_file(&temp_path, first_conflict_line)?;

    let new_content = std::fs::read(&temp_path).map_err(ExternalToolError::Io)?;
    if new_content.is_empty() || new_content == initial_content {
        return Err(ConflictResolveError::EmptyOrUnchanged);
    }
    let new_file_ids = conflicts::update_from_content(
        &file.unsimplified_ids,
        store,
        repo_path,
        &new_content,
        conflict_marker_len,
    )
    .await?;
    tree_builder.set_or_remove(
        repo_path.to_owned(),
        merge_tool_file.new_tree_value(new_file_ids),
    );
    Ok(())
}

async fn pick_conflict_side(
    tree: &MergedTree,
    merge_tool_files: &[MergeToolFile],
    add_index: usize,
) -> Result<MergedTree, BackendError> {
    let mut tree_builder = MergedTreeBuilder::new(tree.clone());
    for merge_tool_file in merge_tool_files {
        // We use file IDs here to match the logic for the other external merge tools.
        // This ensures that the behavior is consistent.
        let file_id = merge_tool_file.file.ids.get_add(add_index).unwrap();
        let new_tree_value = merge_tool_file.new_tree_value(Merge::resolved(file_id.clone()));
        tree_builder.set_or_remove(merge_tool_file.repo_path.clone(), new_tree_value);
    }
    tree_builder.write_tree().await
}

#[cfg(test)]
mod tests {
    use jj_lib::config::ConfigLayer;
    use jj_lib::config::ConfigSource;
    use jj_lib::config::StackedConfig;

    use super::*;

    fn config_from_string(text: &str) -> StackedConfig {
        let mut config = StackedConfig::with_defaults();
        // Load defaults to test the default args lookup
        config.extend_layers(crate::config::default_config_layers());
        config.add_layer(ConfigLayer::parse(ConfigSource::User, text).unwrap());
        config
    }

    #[test]
    fn test_get_diff_editor_with_name() {
        let get = |name, config_text| {
            let config = config_from_string(config_text);
            let settings = testutils::user_settings_from_config(config);
            DiffEditor::with_name(
                name,
                &settings,
                GitIgnoreFile::empty(),
                ConflictMarkerStyle::Diff,
            )
            .map(|editor| editor.tool)
        };

        insta::assert_debug_snapshot!(get(":builtin", "").unwrap(), @"Builtin");

        // Just program name, edit_args are filled by default
        insta::assert_debug_snapshot!(get("my diff", "").unwrap(), @r#"
        External(
            ExternalMergeTool {
                program: "my diff",
                diff_args: [
                    "$left",
                    "$right",
                ],
                diff_expected_exit_codes: [
                    0,
                ],
                diff_invocation_mode: Dir,
                diff_do_chdir: true,
                edit_args: [
                    "$left",
                    "$right",
                ],
                edit_invocation_mode: Dir,
                merge_args: [],
                merge_conflict_exit_codes: [],
                merge_tool_edits_conflict_markers: false,
                conflict_marker_style: None,
            },
        )
        "#);

        // Pick from merge-tools
        insta::assert_debug_snapshot!(get(
            "foo bar", r#"
        [merge-tools."foo bar"]
        edit-args = ["--edit", "args", "$left", "$right"]
        "#,
        ).unwrap(), @r#"
        External(
            ExternalMergeTool {
                program: "foo bar",
                diff_args: [
                    "$left",
                    "$right",
                ],
                diff_expected_exit_codes: [
                    0,
                ],
                diff_invocation_mode: Dir,
                diff_do_chdir: true,
                edit_args: [
                    "--edit",
                    "args",
                    "$left",
                    "$right",
                ],
                edit_invocation_mode: Dir,
                merge_args: [],
                merge_conflict_exit_codes: [],
                merge_tool_edits_conflict_markers: false,
                conflict_marker_style: None,
            },
        )
        "#);
    }

    #[test]
    fn test_get_diff_editor_from_settings() {
        let get = |text| {
            let config = config_from_string(text);
            let ui = Ui::with_config(&config).unwrap();
            let settings = testutils::user_settings_from_config(config);
            DiffEditor::from_settings(
                &ui,
                &settings,
                GitIgnoreFile::empty(),
                ConflictMarkerStyle::Diff,
            )
            .map(|editor| editor.tool)
        };

        // Default
        insta::assert_debug_snapshot!(get("").unwrap(), @"Builtin");

        // Just program name, edit_args are filled by default
        insta::assert_debug_snapshot!(get(r#"ui.diff-editor = "my-diff""#).unwrap(), @r#"
        External(
            ExternalMergeTool {
                program: "my-diff",
                diff_args: [
                    "$left",
                    "$right",
                ],
                diff_expected_exit_codes: [
                    0,
                ],
                diff_invocation_mode: Dir,
                diff_do_chdir: true,
                edit_args: [
                    "$left",
                    "$right",
                ],
                edit_invocation_mode: Dir,
                merge_args: [],
                merge_conflict_exit_codes: [],
                merge_tool_edits_conflict_markers: false,
                conflict_marker_style: None,
            },
        )
        "#);

        // String args (with interpolation variables)
        insta::assert_debug_snapshot!(
            get(r#"ui.diff-editor = "my-diff -l $left -r $right""#).unwrap(), @r#"
        External(
            ExternalMergeTool {
                program: "my-diff",
                diff_args: [
                    "$left",
                    "$right",
                ],
                diff_expected_exit_codes: [
                    0,
                ],
                diff_invocation_mode: Dir,
                diff_do_chdir: true,
                edit_args: [
                    "-l",
                    "$left",
                    "-r",
                    "$right",
                ],
                edit_invocation_mode: Dir,
                merge_args: [],
                merge_conflict_exit_codes: [],
                merge_tool_edits_conflict_markers: false,
                conflict_marker_style: None,
            },
        )
        "#);

        // List args (with interpolation variables)
        insta::assert_debug_snapshot!(
            get(r#"ui.diff-editor = ["my-diff", "--diff", "$left", "$right"]"#).unwrap(), @r#"
        External(
            ExternalMergeTool {
                program: "my-diff",
                diff_args: [
                    "$left",
                    "$right",
                ],
                diff_expected_exit_codes: [
                    0,
                ],
                diff_invocation_mode: Dir,
                diff_do_chdir: true,
                edit_args: [
                    "--diff",
                    "$left",
                    "$right",
                ],
                edit_invocation_mode: Dir,
                merge_args: [],
                merge_conflict_exit_codes: [],
                merge_tool_edits_conflict_markers: false,
                conflict_marker_style: None,
            },
        )
        "#);

        // Pick from merge-tools
        insta::assert_debug_snapshot!(get(
        r#"
        ui.diff-editor = "foo bar"
        [merge-tools."foo bar"]
        edit-args = ["--edit", "args", "$left", "$right"]
        diff-args = []  # Should not cause an error, since we're getting the diff *editor*
        "#,
        ).unwrap(), @r#"
        External(
            ExternalMergeTool {
                program: "foo bar",
                diff_args: [],
                diff_expected_exit_codes: [
                    0,
                ],
                diff_invocation_mode: Dir,
                diff_do_chdir: true,
                edit_args: [
                    "--edit",
                    "args",
                    "$left",
                    "$right",
                ],
                edit_invocation_mode: Dir,
                merge_args: [],
                merge_conflict_exit_codes: [],
                merge_tool_edits_conflict_markers: false,
                conflict_marker_style: None,
            },
        )
        "#);

        // Pick from merge-tools, but no edit-args specified
        insta::assert_debug_snapshot!(get(
        r#"
        ui.diff-editor = "my-diff"
        [merge-tools.my-diff]
        program = "MyDiff"
        "#,
        ).unwrap(), @r#"
        External(
            ExternalMergeTool {
                program: "MyDiff",
                diff_args: [
                    "$left",
                    "$right",
                ],
                diff_expected_exit_codes: [
                    0,
                ],
                diff_invocation_mode: Dir,
                diff_do_chdir: true,
                edit_args: [
                    "$left",
                    "$right",
                ],
                edit_invocation_mode: Dir,
                merge_args: [],
                merge_conflict_exit_codes: [],
                merge_tool_edits_conflict_markers: false,
                conflict_marker_style: None,
            },
        )
        "#);

        // List args should never be a merge-tools key, edit_args are filled by default
        insta::assert_debug_snapshot!(get(r#"ui.diff-editor = ["meld"]"#).unwrap(), @r#"
        External(
            ExternalMergeTool {
                program: "meld",
                diff_args: [
                    "$left",
                    "$right",
                ],
                diff_expected_exit_codes: [
                    0,
                ],
                diff_invocation_mode: Dir,
                diff_do_chdir: true,
                edit_args: [
                    "$left",
                    "$right",
                ],
                edit_invocation_mode: Dir,
                merge_args: [],
                merge_conflict_exit_codes: [],
                merge_tool_edits_conflict_markers: false,
                conflict_marker_style: None,
            },
        )
        "#);

        // Invalid type
        assert!(get(r#"ui.diff-editor.k = 0"#).is_err());

        // Explicitly empty edit-args cause an error
        insta::assert_debug_snapshot!(get(
        r#"
        ui.diff-editor = "my-diff"
        [merge-tools.my-diff]
        program = "MyDiff"
        edit-args = []
        "#,
        ), @r#"
        Err(
            EditArgsNotConfigured {
                tool_name: "my-diff",
            },
        )
        "#);
    }

    #[test]
    fn test_get_merge_editor_with_name() {
        let get = |name, config_text| {
            let config = config_from_string(config_text);
            let settings = testutils::user_settings_from_config(config);
            let path_converter = RepoPathUiConverter::Fs {
                cwd: "".into(),
                base: "".into(),
            };
            MergeEditor::with_name(name, &settings, path_converter, ConflictMarkerStyle::Diff)
                .map(|editor| editor.tool)
        };

        insta::assert_debug_snapshot!(get(":builtin", "").unwrap(), @"Builtin");

        // Just program name
        insta::assert_debug_snapshot!(get("my diff", "").unwrap_err(), @r#"
        MergeArgsNotConfigured {
            tool_name: "my diff",
        }
        "#);

        // Pick from merge-tools
        insta::assert_debug_snapshot!(get(
            "foo bar", r#"
        [merge-tools."foo bar"]
        merge-args = ["$base", "$left", "$right", "$output"]
        "#,
        ).unwrap(), @r#"
        External(
            ExternalMergeTool {
                program: "foo bar",
                diff_args: [
                    "$left",
                    "$right",
                ],
                diff_expected_exit_codes: [
                    0,
                ],
                diff_invocation_mode: Dir,
                diff_do_chdir: true,
                edit_args: [
                    "$left",
                    "$right",
                ],
                edit_invocation_mode: Dir,
                merge_args: [
                    "$base",
                    "$left",
                    "$right",
                    "$output",
                ],
                merge_conflict_exit_codes: [],
                merge_tool_edits_conflict_markers: false,
                conflict_marker_style: None,
            },
        )
        "#);
    }

    #[test]
    fn test_get_merge_editor_from_settings() {
        let get = |text| {
            let config = config_from_string(text);
            let ui = Ui::with_config(&config).unwrap();
            let settings = testutils::user_settings_from_config(config);
            let path_converter = RepoPathUiConverter::Fs {
                cwd: "".into(),
                base: "".into(),
            };
            MergeEditor::from_settings(&ui, &settings, path_converter, ConflictMarkerStyle::Diff)
                .map(|editor| editor.tool)
        };

        // Default
        insta::assert_debug_snapshot!(get("").unwrap(), @"Builtin");

        // Just program name
        insta::assert_debug_snapshot!(get(r#"ui.merge-editor = "my-merge""#).unwrap_err(), @r#"
        MergeArgsNotConfigured {
            tool_name: "my-merge",
        }
        "#);

        // String args
        insta::assert_debug_snapshot!(
            get(r#"ui.merge-editor = "my-merge $left $base $right $output""#).unwrap(), @r#"
        External(
            ExternalMergeTool {
                program: "my-merge",
                diff_args: [
                    "$left",
                    "$right",
                ],
                diff_expected_exit_codes: [
                    0,
                ],
                diff_invocation_mode: Dir,
                diff_do_chdir: true,
                edit_args: [
                    "$left",
                    "$right",
                ],
                edit_invocation_mode: Dir,
                merge_args: [
                    "$left",
                    "$base",
                    "$right",
                    "$output",
                ],
                merge_conflict_exit_codes: [],
                merge_tool_edits_conflict_markers: false,
                conflict_marker_style: None,
            },
        )
        "#);

        // List args
        insta::assert_debug_snapshot!(
            get(
                r#"ui.merge-editor = ["my-merge", "$left", "$base", "$right", "$output"]"#,
            ).unwrap(), @r#"
        External(
            ExternalMergeTool {
                program: "my-merge",
                diff_args: [
                    "$left",
                    "$right",
                ],
                diff_expected_exit_codes: [
                    0,
                ],
                diff_invocation_mode: Dir,
                diff_do_chdir: true,
                edit_args: [
                    "$left",
                    "$right",
                ],
                edit_invocation_mode: Dir,
                merge_args: [
                    "$left",
                    "$base",
                    "$right",
                    "$output",
                ],
                merge_conflict_exit_codes: [],
                merge_tool_edits_conflict_markers: false,
                conflict_marker_style: None,
            },
        )
        "#);

        // Pick from merge-tools
        insta::assert_debug_snapshot!(get(
        r#"
        ui.merge-editor = "foo bar"
        [merge-tools."foo bar"]
        merge-args = ["$base", "$left", "$right", "$output"]
        "#,
        ).unwrap(), @r#"
        External(
            ExternalMergeTool {
                program: "foo bar",
                diff_args: [
                    "$left",
                    "$right",
                ],
                diff_expected_exit_codes: [
                    0,
                ],
                diff_invocation_mode: Dir,
                diff_do_chdir: true,
                edit_args: [
                    "$left",
                    "$right",
                ],
                edit_invocation_mode: Dir,
                merge_args: [
                    "$base",
                    "$left",
                    "$right",
                    "$output",
                ],
                merge_conflict_exit_codes: [],
                merge_tool_edits_conflict_markers: false,
                conflict_marker_style: None,
            },
        )
        "#);

        // List args should never be a merge-tools key
        insta::assert_debug_snapshot!(
            get(r#"ui.merge-editor = ["meld"]"#).unwrap_err(), @r#"
        MergeArgsNotConfigured {
            tool_name: "meld",
        }
        "#);

        // Invalid type
        assert!(get(r#"ui.merge-editor.k = 0"#).is_err());
    }
}
