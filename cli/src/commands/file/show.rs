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

use std::io::Write as _;

use clap_complete::ArgValueCandidates;
use clap_complete::ArgValueCompleter;
use itertools::Itertools as _;
use jj_lib::backend::BackendResult;
use jj_lib::conflicts::ConflictMaterializeOptions;
use jj_lib::conflicts::MaterializedTreeValue;
use jj_lib::conflicts::materialize_merge_result;
use jj_lib::conflicts::materialize_tree_value;
use jj_lib::file_util::copy_async_to_sync;
use jj_lib::fileset::FilePattern;
use jj_lib::fileset::FilesetExpression;
use jj_lib::merged_tree::MergedTree;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPath;
use pollster::FutureExt as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::WorkspaceCommandHelper;
use crate::cli_util::print_unmatched_explicit_paths;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::commit_templater::TreeEntry;
use crate::complete;
use crate::templater::TemplateRenderer;
use crate::ui::Ui;

/// Print contents of files in a revision
///
/// If the given path is a directory, files in the directory will be visited
/// recursively.
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct FileShowArgs {
    /// The revision to get the file contents from
    #[arg(
        long, short,
        default_value = "@",
        value_name = "REVSET",
        add = ArgValueCompleter::new(complete::revset_expression_all),
    )]
    revision: RevisionArg,

    /// Render each file metadata using the given template
    ///
    /// All 0-argument methods of the [`TreeEntry` type] are available as
    /// keywords in the template expression. See [`jj help -k templates`] for
    /// more information.
    ///
    /// If not specified, this defaults to the `templates.file_show` setting.
    ///
    /// [`TreeEntry` type]:
    ///     https://docs.jj-vcs.dev/latest/templates/#treeentry-type
    ///
    /// [`jj help -k templates`]:
    ///     https://docs.jj-vcs.dev/latest/templates/
    #[arg(long, short = 'T', add = ArgValueCandidates::new(complete::template_aliases))]
    template: Option<String>,

    /// Paths to print
    #[arg(
        required = true,
        value_name = "FILESETS",
        value_hint = clap::ValueHint::FilePath,
        add = ArgValueCompleter::new(complete::all_revision_files),
    )]
    paths: Vec<String>,
}

#[instrument(skip_all)]
pub(crate) fn cmd_file_show(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &FileShowArgs,
) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper(ui)?;
    let commit = workspace_command.resolve_single_rev(ui, &args.revision)?;
    let tree = commit.tree();
    // TODO: No need to add special case for empty paths when switching to
    // parse_union_filesets(). paths = [] should be "none()" if supported.
    let fileset_expression = workspace_command.parse_file_patterns(ui, &args.paths)?;
    let template = {
        let language = workspace_command.commit_template_language();
        let text = match &args.template {
            Some(value) => value.to_owned(),
            None => workspace_command.settings().get("templates.file_show")?,
        };
        workspace_command
            .parse_template(ui, &language, &text)?
            .labeled(["file_show"])
    };

    // Try fast path for single file entry
    if let Some(path) = get_single_path(&fileset_expression) {
        let value = tree.path_value(path)?;
        if value.is_absent() {
            let ui_path = workspace_command.format_file_path(path);
            return Err(user_error(format!("No such path: {ui_path}")));
        }
        if !value.is_tree() {
            ui.request_pager();
            let entry = TreeEntry {
                path: path.to_owned(),
                value,
            };
            write_tree_entries(ui, &workspace_command, &template, &tree, [Ok(entry)])?;
            return Ok(());
        }
    }

    let matcher = fileset_expression.to_matcher();
    ui.request_pager();
    write_tree_entries(
        ui,
        &workspace_command,
        &template,
        &tree,
        tree.entries_matching(matcher.as_ref())
            .map(|(path, value)| Ok((path, value?)))
            .map_ok(|(path, value)| TreeEntry { path, value }),
    )?;
    print_unmatched_explicit_paths(ui, &workspace_command, &fileset_expression, [&tree])?;
    Ok(())
}

fn get_single_path(expression: &FilesetExpression) -> Option<&RepoPath> {
    match &expression {
        FilesetExpression::Pattern(pattern) => match pattern {
            // Not using pattern.as_path() because files-in:<path> shouldn't
            // select the literal <path> itself.
            FilePattern::FilePath(path) | FilePattern::PrefixPath(path) => Some(path),
            FilePattern::FileGlob { .. } | FilePattern::PrefixGlob { .. } => None,
        },
        _ => None,
    }
}

fn write_tree_entries(
    ui: &Ui,
    workspace_command: &WorkspaceCommandHelper,
    template: &TemplateRenderer<TreeEntry>,
    tree: &MergedTree,
    entries: impl IntoIterator<Item = BackendResult<TreeEntry>>,
) -> Result<(), CommandError> {
    let repo = workspace_command.repo();
    for entry in entries {
        let entry = entry?;
        template.format(&entry, ui.stdout_formatter().as_mut())?;
        let materialized =
            materialize_tree_value(repo.store(), &entry.path, entry.value, tree.labels())
                .block_on()?;
        match materialized {
            MaterializedTreeValue::Absent => panic!("absent values should be excluded"),
            MaterializedTreeValue::AccessDenied(err) => {
                let ui_path = workspace_command.format_file_path(&entry.path);
                writeln!(
                    ui.warning_default(),
                    "Path '{ui_path}' exists but access is denied: {err}"
                )?;
            }
            MaterializedTreeValue::File(file) => {
                copy_async_to_sync(file.reader, ui.stdout_formatter().as_mut()).block_on()?;
            }
            MaterializedTreeValue::FileConflict(file) => {
                let options = ConflictMaterializeOptions {
                    marker_style: workspace_command.env().conflict_marker_style(),
                    marker_len: None,
                    merge: repo.store().merge_options().clone(),
                };
                materialize_merge_result(
                    &file.contents,
                    &file.labels,
                    &mut ui.stdout_formatter(),
                    &options,
                )?;
            }
            MaterializedTreeValue::OtherConflict { id } => {
                ui.stdout_formatter().write_all(id.describe().as_bytes())?;
            }
            MaterializedTreeValue::Symlink { .. } | MaterializedTreeValue::GitSubmodule(_) => {
                let ui_path = workspace_command.format_file_path(&entry.path);
                writeln!(
                    ui.warning_default(),
                    "Path '{ui_path}' exists but is not a file"
                )?;
            }
            MaterializedTreeValue::Tree(_) => panic!("entries should not contain trees"),
        }
    }
    Ok(())
}
