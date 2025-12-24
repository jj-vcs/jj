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
use std::io::Write as _;
use std::path::Path;
use std::process::Stdio;

use clap_complete::ArgValueCompleter;
use itertools::Itertools as _;
use jj_lib::backend::FileId;
use jj_lib::commit::Commit;
use jj_lib::fileset;
use jj_lib::fileset::FilesetDiagnostics;
use jj_lib::fileset::FilesetExpression;
use jj_lib::fix::FileToFix;
use jj_lib::fix::FixError;
use jj_lib::fix::ParallelFileFixer;
use jj_lib::fix::fix_files;
use jj_lib::matchers::Matcher;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPathUiConverter;
use jj_lib::revset::RevsetIteratorExt as _;
use jj_lib::settings::UserSettings;
use jj_lib::store::Store;
use pollster::FutureExt as _;
use tokio::io::AsyncReadExt as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::cli_util::RevisionArg;
use crate::cli_util::print_unmatched_explicit_paths;
use crate::command_error::CommandError;
use crate::command_error::config_error;
use crate::command_error::print_parse_diagnostics;
use crate::complete;
use crate::config::CommandNameAndArgs;
use crate::ui::Ui;

/// Update files with formatting fixes or other changes
///
/// The primary use case for this command is to apply the results of automatic
/// code formatting tools to revisions that may not be properly formatted yet.
/// It can also be used to modify files with other tools like `sed` or `sort`.
///
/// The modification made by `jj fix` can be reviewed by `jj op show -p`.
///
/// ### How it works
///
/// The changed files in the given revisions will be updated with any fixes
/// determined by passing their file content through any external tools the user
/// has configured for those files. Descendants will also be updated by passing
/// their versions of the same files through the same tools, which will ensure
/// that the fixes are not lost. This will never result in new conflicts. Files
/// with existing conflicts will be updated on all sides of the conflict, which
/// can potentially increase or decrease the number of conflict markers.
///
/// ### Configuration
///
/// See `jj help -k config` chapter `Code formatting and other file content
/// transformations` to understand how to configure your tools.
///
/// ### Execution Example
///
/// Let's consider the following configuration is set. We have two code
/// formatters (`clang-format` and `black`), which apply to three different
/// file extensions (`.cc`, `.h`, and `.py`):
///
/// ```toml
/// [fix.tools.clang-format]
/// command = ["/usr/bin/clang-format", "--assume-filename=$path"]
/// patterns = ["glob:'**/*.cc'",
///             "glob:'**/*.h'"]
///
/// [fix.tools.black]
/// command = ["/usr/bin/black", "-", "--stdin-filename=$path"]
/// patterns = ["glob:'**/*.py'"]
/// ```
///
/// Now, let's see what would happen to the following history, when executing
/// `jj fix`.
///
/// ```text
/// C (mutable)
/// |  Modifies file: foo.py
/// |
/// B @ (working copy - mutable)
/// |  Modifies file: README.md
/// |
/// A (mutable)
/// |  Modifies files: src/bar.cc and src/bar.h
/// |
/// X (immutable)
/// ```
///
/// By default, `jj fix` will modify revisions that matches the revset
/// `reachable(@, mutable())` (see `jj help -k revsets`) which corresponds to
/// the revisions `A`, `B` and `C` here.
///
/// The following operations will then happen:
///
/// - For revision `A`, content from this revision for files `src/bar.cc` and
///   `src/bar.h` will each be provided to `clang-format` and the result output
///   will be used to recreate revision `A` which we will call `A'`. All other
///   files are untouched.
/// - For revision `B`, same thing happen for files `src/bar.cc` and `src/bar.h`
///   Their content from revision `B` will go through `clang-format`. The file
///   `README.md` as any other files, are untouched as no pattern matches it. We
///   obtain revision `B'`.
/// - For revision `C`, `src/bar.cc` and `src/bar.h` goes through `clang-format`
///   and file `foo.py` is fixed using `black`. Any other file is untouched. We
///   obtain revision `C'`.
///
/// ```text
/// C (mutable)                    /-> C'
/// |  src/bar.cc -> clang-format -|   |
/// |  src/bar.h --> clang-format -|   |
/// |  foo.py -----> black --------|   |
/// |  * --------------------------/   |
/// |                                  |
/// B @ (working copy - mutable)   /-> B' @
/// |  src/bar.cc -> clang-format -|   |
/// |  src/bar.h --> clang-format -|   |
/// |  * --------------------------|   |
/// |                                  |
/// A (mutable)                    /-> A'
/// |  src/bar.cc -> clang-format -|   |
/// |  src/bar.h --> clang-format -|   |
/// |  * --------------------------/   |
/// |                                  |
/// X (immutable)                      X
/// ```
///
/// The revisions are now all correctly formatted according to the
/// configuration.
#[derive(clap::Args, Clone, Debug)]
#[command(verbatim_doc_comment)]
pub(crate) struct FixArgs {
    /// Fix files in the specified revision(s) and their descendants. If no
    /// revisions are specified, this defaults to the `revsets.fix` setting, or
    /// `reachable(@, mutable())` if it is not set.
    #[arg(
        long,
        short,
        value_name = "REVSETS",
        add = ArgValueCompleter::new(complete::revset_expression_mutable),
    )]
    source: Vec<RevisionArg>,

    /// Fix only these paths
    #[arg(value_name = "FILESETS", value_hint = clap::ValueHint::AnyPath)]
    paths: Vec<String>,

    /// Fix unchanged files in addition to changed ones. If no paths are
    /// specified, all files in the repo will be fixed.
    #[arg(long)]
    include_unchanged_files: bool,
}

#[instrument(skip_all)]
pub(crate) fn cmd_fix(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &FixArgs,
) -> Result<(), CommandError> {
    let mut workspace_command = command.workspace_helper(ui)?;
    let workspace_root = workspace_command.workspace_root().to_owned();
    let path_converter = workspace_command.path_converter().to_owned();
    let tools_config = get_tools_config(ui, workspace_command.settings())?;
    let target_expr = if args.source.is_empty() {
        let revs = workspace_command.settings().get_string("revsets.fix")?;
        workspace_command.parse_revset(ui, &RevisionArg::from(revs))?
    } else {
        workspace_command.parse_union_revsets(ui, &args.source)?
    }
    .resolve()?;
    workspace_command.check_rewritable_expr(&target_expr)?;

    let repo = workspace_command.repo();

    let commits: Vec<Commit> = target_expr
        .descendants()
        .evaluate(repo.as_ref())?
        .iter()
        .commits(repo.store())
        .try_collect()?;

    let commit_ids = commits
        .iter()
        .map(|commit| commit.id().clone())
        .collect_vec();

    let trees: Vec<_> = commits.iter().map(|commit| commit.tree()).collect();

    let fileset_expression = workspace_command.parse_file_patterns(ui, &args.paths)?;
    let matcher = fileset_expression.to_matcher();

    let mut tx = workspace_command.start_transaction();
    let mut parallel_fixer = ParallelFileFixer::new(|store, file_to_fix| {
        fix_one_file(
            ui,
            &workspace_root,
            &path_converter,
            &tools_config,
            store,
            file_to_fix,
        )
        .block_on()
    });

    print_unmatched_explicit_paths(ui, tx.base_workspace_helper(), &fileset_expression, &trees)?;

    let summary = fix_files(
        commit_ids,
        &matcher,
        args.include_unchanged_files,
        tx.repo_mut(),
        &mut parallel_fixer,
    )
    .block_on()?;
    writeln!(
        ui.status(),
        "Fixed {} commits of {} checked.",
        summary.num_fixed_commits,
        summary.num_checked_commits
    )?;
    tx.finish(ui, format!("fixed {} commits", summary.num_fixed_commits))
}

/// Invokes all matching tools (if any) to file_to_fix. If the content is
/// successfully transformed the new content is written and the new FileId is
/// returned. Returns None if the content is unchanged.
///
/// The matching tools are invoked in order, with the result of one tool feeding
/// into the next tool. Returns FixError if there is an error reading or writing
/// the file. However, if a tool invocation fails for whatever reason, the tool
/// is simply skipped and we proceed to invoke the next tool (this is
/// indistinguishable from succeeding with no changes).
///
/// TODO: Better error handling so we can tell the user what went wrong with
/// each failed input.
async fn fix_one_file(
    ui: &Ui,
    workspace_root: &Path,
    path_converter: &RepoPathUiConverter,
    tools_config: &ToolsConfig,
    store: &Store,
    file_to_fix: &FileToFix,
) -> Result<Option<FileId>, FixError> {
    let mut matching_tools = tools_config
        .tools
        .iter()
        .filter(|tool_config| tool_config.matcher.matches(&file_to_fix.repo_path))
        .peekable();
    if matching_tools.peek().is_some() {
        // The first matching tool gets its input from the committed file, and any
        // subsequent matching tool gets its input from the previous matching tool's
        // output.
        let mut old_content = vec![];
        let mut read = store
            .read_file(&file_to_fix.repo_path, &file_to_fix.file_id)
            .await?;
        read.read_to_end(&mut old_content).await?;
        let new_content = matching_tools.fold(old_content.clone(), |prev_content, tool_config| {
            match run_tool(
                ui,
                workspace_root,
                path_converter,
                &tool_config.command,
                file_to_fix,
                &prev_content,
            ) {
                Ok(next_content) => next_content,
                // TODO: Because the stderr is passed through, this isn't always failing
                // silently, but it should do something better will the exit code, tool
                // name, etc.
                Err(()) => prev_content,
            }
        });
        if new_content != old_content {
            // TODO: send futures back over channel
            let new_file_id = store
                .write_file(&file_to_fix.repo_path, &mut new_content.as_slice())
                .await?;
            return Ok(Some(new_file_id));
        }
    }
    Ok(None)
}

/// Runs the `tool_command` to fix the given file content.
///
/// The `old_content` is assumed to be that of the `file_to_fix`'s `FileId`, but
/// this is not verified.
///
/// Returns the new file content, whose value will be the same as `old_content`
/// unless the command introduced changes. Returns `None` if there were any
/// failures when starting, stopping, or communicating with the subprocess.
fn run_tool(
    ui: &Ui,
    workspace_root: &Path,
    path_converter: &RepoPathUiConverter,
    tool_command: &CommandNameAndArgs,
    file_to_fix: &FileToFix,
    old_content: &[u8],
) -> Result<Vec<u8>, ()> {
    let mut vars: HashMap<&str, &str> = HashMap::new();
    vars.insert("path", file_to_fix.repo_path.as_internal_file_string());
    // TODO: workspace_root.to_str() returns None if the workspace path is not
    // UTF-8, but we ignore that failure so `jj fix` still runs in that
    // situation. Maybe we should do something like substituting bytes instead
    // of strings so we can handle any Path here.
    if let Some(root) = workspace_root.to_str() {
        vars.insert("root", root);
    }
    let mut command = tool_command.to_command_with_variables(&vars);
    tracing::debug!(?command, ?file_to_fix.repo_path, "spawning fix tool");
    let mut child = match command
        .current_dir(workspace_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => {
            writeln!(
                ui.warning_default(),
                "Failed to start `{}`",
                tool_command.split_name(),
            )
            .ok();
            return Err(());
        }
    };
    let mut stdin = child.stdin.take().expect(
        "The child process is created with piped stdin, and it's our first access to stdin.",
    );
    let output = std::thread::scope(|s| {
        s.spawn(move || {
            stdin.write_all(old_content).ok();
        });
        child.wait_with_output().or(Err(()))
    })?;
    tracing::debug!(?command, ?output.status, "fix tool exited:");
    if !output.stderr.is_empty() {
        let mut stderr = ui.stderr();
        writeln!(
            stderr,
            "{}:",
            path_converter.format_file_path(&file_to_fix.repo_path)
        )
        .ok();
        stderr.write_all(&output.stderr).ok();
        writeln!(stderr).ok();
    }
    if output.status.success() {
        Ok(output.stdout)
    } else {
        writeln!(
            ui.warning_default(),
            "Fix tool `{}` exited with non-zero exit code for `{}`",
            tool_command.split_name(),
            path_converter.format_file_path(&file_to_fix.repo_path)
        )
        .ok();
        Err(())
    }
}

/// Represents an entry in the `fix.tools` config table.
struct ToolConfig {
    /// The command that will be run to fix a matching file.
    command: CommandNameAndArgs,
    /// The matcher that determines if this tool matches a file.
    matcher: Box<dyn Matcher>,
    /// Whether the tool is enabled
    enabled: bool,
    // TODO: Store the `name` field here and print it with the command's stderr, to clearly
    // associate any errors/warnings with the tool and its configuration entry.
}

/// Represents the `fix.tools` config table.
struct ToolsConfig {
    /// Some tools, stored in the order they will be executed if more than one
    /// of them matches the same file.
    tools: Vec<ToolConfig>,
}

/// Simplifies deserialization of the config values while building a ToolConfig.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
struct RawToolConfig {
    command: CommandNameAndArgs,
    patterns: Vec<String>,
    #[serde(default = "default_tool_enabled")]
    enabled: bool,
}

fn default_tool_enabled() -> bool {
    true
}

/// Parses the `fix.tools` config table.
///
/// Fails if any of the commands or patterns are obviously unusable, but does
/// not check for issues that might still occur later like missing executables.
/// This is a place where we could fail earlier in some cases, though.
fn get_tools_config(ui: &mut Ui, settings: &UserSettings) -> Result<ToolsConfig, CommandError> {
    let mut tools: Vec<ToolConfig> = settings
        .table_keys("fix.tools")
        // Sort keys early so errors are deterministic.
        .sorted()
        .map(|name| -> Result<ToolConfig, CommandError> {
            let mut diagnostics = FilesetDiagnostics::new();
            let tool: RawToolConfig = settings.get(["fix", "tools", name])?;
            let expression = FilesetExpression::union_all(
                tool.patterns
                    .iter()
                    .map(|arg| {
                        fileset::parse(
                            &mut diagnostics,
                            arg,
                            &RepoPathUiConverter::Fs {
                                cwd: "".into(),
                                base: "".into(),
                            },
                        )
                    })
                    .try_collect()?,
            );
            print_parse_diagnostics(ui, &format!("In `fix.tools.{name}`"), &diagnostics)?;
            Ok(ToolConfig {
                command: tool.command,
                matcher: expression.to_matcher(),
                enabled: tool.enabled,
            })
        })
        .try_collect()?;
    if tools.is_empty() {
        return Err(config_error("No `fix.tools` are configured"));
    }
    tools.retain(|t| t.enabled);
    if tools.is_empty() {
        Err(config_error(
            "At least one entry of `fix.tools` must be enabled.".to_string(),
        ))
    } else {
        Ok(ToolsConfig { tools })
    }
}
