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
use std::io::Read as _;
use std::io::Write as _;
use std::path::Path;
use std::process::Stdio;

use clap_complete::ArgValueCompleter;
use itertools::Itertools as _;
use jj_lib::backend::FileId;
use jj_lib::commit::Commit;
use jj_lib::file_util::IoResultExt as _;
use jj_lib::fileset;
use jj_lib::fileset::FilesetDiagnostics;
use jj_lib::fileset::FilesetExpression;
use jj_lib::fix::FileToFix;
use jj_lib::fix::FixError;
use jj_lib::fix::ParallelFileFixer;
use jj_lib::fix::fix_files;
use jj_lib::matchers::Matcher;
use jj_lib::repo::Repo as _;
use jj_lib::repo_path::RepoPath;
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
use crate::command_error::CommandErrorKind;
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
/// The changed files in the given revisions will be updated with any fixes
/// determined by passing their file content through any external tools the user
/// has configured for those files. Descendants will also be updated by passing
/// their versions of the same files through the same tools, which will ensure
/// that the fixes are not lost. This will never result in new conflicts. Files
/// with existing conflicts will be updated on all sides of the conflict, which
/// can potentially increase or decrease the number of conflict markers.
///
/// The external tools must accept the current file content on standard input,
/// and return the updated file content on standard output. A tool's output will
/// not be used unless it exits with a successful exit code. Output on standard
/// error will be passed through to the terminal.
///
/// Tools are defined in a table where the keys are arbitrary identifiers and
/// the values have the following properties:
///  - `command`: The arguments used to run the tool. The first argument is the
///    path to an executable file. Arguments can contain these variables that
///    will be replaced:
///    - `$root` will be replaced with the workspace root path (the directory
///      containing the .jj directory).
///    - `$path` will be replaced with the repo-relative path of the file being
///      fixed. It is useful to provide the path to tools that include the path
///      in error messages, or behave differently based on the directory or file
///      name.
///  - `patterns`: Determines which files the tool will affect. If this list is
///    empty, no files will be affected by the tool. If there are multiple
///    patterns, the tool is applied only once to each file in the union of the
///    patterns.
///  - `enabled`: Enables or disables the tool. If omitted, the tool is enabled.
///    This is useful for defining disabled tools in user configuration that can
///    be enabled in individual repositories with one config setting.
///
/// For example, the following configuration defines how two code formatters
/// (`clang-format` and `black`) will apply to three different file extensions
/// (`.cc`, `.h`, and `.py`):
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
/// Execution order of tools that affect the same file is deterministic, but
/// currently unspecified, and may change between releases. If two tools affect
/// the same file, the second tool to run will receive its input from the
/// output of the first tool.
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

    /// Operate on files read directly from stdin or from disk, and write to
    /// stdout. You must provide a filepath, either one path on disk like
    /// `jj fix --no-index src/main.rs`, or use `jj fix --no-index
    /// --stdin-filepath=...` and supply the file content on stdin.
    ///
    /// This makes `jj fix` behave as a code formatter itself, and with it, you
    /// can configure a text editor's auto-format to do exactly what `jj fix`
    /// would.
    ///
    /// You can also use it to simulate a `jj fix` configuration without writing
    /// anything to disk.
    #[arg(long)]
    no_index: bool,

    /// When using --no-index, read the file content from stdin but match the
    /// `fix.tools` configurations using this path. Relative to CWD.
    #[arg(long)]
    stdin_filepath: Option<String>,
}

#[instrument(skip_all)]
pub(crate) fn cmd_fix(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &FixArgs,
) -> Result<(), CommandError> {
    if args.no_index {
        // Entirely separate codepath. We do not want to snapshot in --no-index mode.
        return fix_no_index(ui, command, args);
    }
    let mut workspace_command = command.workspace_helper(ui)?;
    let workspace_root = workspace_command.workspace_root().to_owned();
    let path_converter = workspace_command.path_converter().to_owned();
    let tools_config = get_tools_config(ui, workspace_command.settings(), true)?;
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
    let read_file = || async {
        let mut old_content = vec![];
        let mut read = store
            .read_file(&file_to_fix.repo_path, &file_to_fix.file_id)
            .await?;
        read.read_to_end(&mut old_content).await?;
        Ok(old_content)
    };
    let maybe_new_content = run_tools_one_file(
        ui,
        workspace_root,
        path_converter,
        tools_config,
        &file_to_fix.repo_path,
        read_file,
    )
    .await?;

    if let ToolsRunOutput::Ran {
        new_content,
        different: true,
    } = maybe_new_content
    {
        // TODO: send futures back over channel
        let new_file_id = store
            .write_file(&file_to_fix.repo_path, &mut new_content.as_slice())
            .await?;
        return Ok(Some(new_file_id));
    }
    Ok(None)
}

/// Entirely separate code path for --no-index
fn fix_no_index(ui: &mut Ui, command: &CommandHelper, args: &FixArgs) -> Result<(), CommandError> {
    let workspace_command = command.workspace_helper_no_snapshot(ui)?;
    let workspace_root = workspace_command.workspace_root();
    let path_converter = workspace_command.path_converter();
    let tools_config = get_tools_config(ui, workspace_command.settings(), false)?;

    let repo_path;
    let mut read_file: Box<dyn FnMut() -> _>;

    if let Some(stdin_filepath) = args.stdin_filepath.as_deref() {
        repo_path = path_converter.parse_file_path(stdin_filepath)?;
        read_file = Box::new(|| {
            // This is blocking io in async. But it's fine.
            let mut stdin = std::io::stdin();
            let mut old_content = vec![];
            stdin.read_to_end(&mut old_content)?;
            Ok(old_content)
        });
    } else {
        let path = args
            .paths
            .iter()
            .exactly_one()
            .cloned()
            .map_err(|_not_one| {
                CommandError::new(
                    CommandErrorKind::User,
                    "--no-index without --stdin-filepath requires exactly one path argument, and \
                     it cannot be be a fileset"
                        .to_string(),
                )
            })?;
        repo_path = path_converter.parse_file_path(&path)?;
        read_file = Box::new(move || {
            let path = Path::new(&path);
            let mut file = std::fs::File::open(path).context(path)?;
            let mut old_content = vec![];
            file.read_to_end(&mut old_content)?;
            Ok(old_content)
        });
    }

    let run_output = run_tools_one_file(
        ui,
        workspace_root,
        path_converter,
        &tools_config,
        &repo_path,
        || async { read_file() },
    )
    .block_on()?;

    let stdout = std::io::stdout();
    let mut stdout_locked = stdout.lock();

    match run_output {
        ToolsRunOutput::Ran {
            new_content,
            different: _,
        } => {
            // always print
            stdout_locked.write_all(&new_content)?;
        }
        ToolsRunOutput::NoMatchingTools => {
            // We already warned about empty/disabled tools when reading config
            if !tools_config.tools.is_empty() {
                writeln!(
                    ui.warning_default(),
                    "No `fix.tools` matched {}",
                    path_converter.format_file_path(&repo_path)
                )?;
            }
            if args.stdin_filepath.is_some() {
                // just copy stdin to stdout, we haven't read stdin yet
                let stdin = std::io::stdin();
                std::io::copy(&mut stdin.lock(), &mut stdout_locked)?;
            } else {
                // reading from file
                let old_content = read_file()?;
                stdout_locked.write_all(&old_content)?;
            }
        }
    }
    Ok(())
}

enum ToolsRunOutput {
    NoMatchingTools,
    Ran {
        new_content: Vec<u8>,
        different: bool,
    },
}

async fn run_tools_one_file<F>(
    ui: &Ui,
    workspace_root: &Path,
    path_converter: &RepoPathUiConverter,
    tools_config: &ToolsConfig,
    repo_path: &RepoPath,
    read_file: impl FnOnce() -> F,
) -> Result<ToolsRunOutput, FixError>
where
    F: Future<Output = Result<Vec<u8>, FixError>>,
{
    let mut matching_tools = tools_config
        .tools
        .iter()
        .filter(|tool_config| tool_config.matcher.matches(repo_path))
        .peekable();
    if matching_tools.peek().is_some() {
        // The first matching tool gets its input from the committed file, and any
        // subsequent matching tool gets its input from the previous matching tool's
        // output.
        let old_content = read_file().await?;
        let new_content = matching_tools.fold(old_content.clone(), |prev_content, tool_config| {
            match run_tool(
                ui,
                workspace_root,
                path_converter,
                &tool_config.command,
                repo_path,
                &prev_content,
            ) {
                Ok(next_content) => next_content,
                // TODO: Because the stderr is passed through, this isn't always failing
                // silently, but it should do something better will the exit code, tool
                // name, etc.
                Err(()) => prev_content,
            }
        });
        Ok(ToolsRunOutput::Ran {
            different: new_content != old_content,
            new_content,
        })
    } else {
        Ok(ToolsRunOutput::NoMatchingTools)
    }
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
    repo_path: &RepoPath,
    old_content: &[u8],
) -> Result<Vec<u8>, ()> {
    let mut vars: HashMap<&str, &str> = HashMap::new();
    vars.insert("path", repo_path.as_internal_file_string());
    // TODO: workspace_root.to_str() returns None if the workspace path is not
    // UTF-8, but we ignore that failure so `jj fix` still runs in that
    // situation. Maybe we should do something like substituting bytes instead
    // of strings so we can handle any Path here.
    if let Some(root) = workspace_root.to_str() {
        vars.insert("root", root);
    }
    let mut command = tool_command.to_command_with_variables(&vars);
    tracing::debug!(?command, ?repo_path, "spawning fix tool");
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
    let mut stdin = child.stdin.take().unwrap();
    let output = std::thread::scope(|s| {
        s.spawn(move || {
            stdin.write_all(old_content).ok();
        });
        Some(child.wait_with_output().or(Err(())))
    })
    .unwrap()?;
    tracing::debug!(?command, ?output.status, "fix tool exited:");
    if !output.stderr.is_empty() {
        let mut stderr = ui.stderr();
        writeln!(stderr, "{}:", path_converter.format_file_path(repo_path)).ok();
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
            path_converter.format_file_path(repo_path)
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
fn get_tools_config(
    ui: &mut Ui,
    settings: &UserSettings,
    require_nonempty: bool,
) -> Result<ToolsConfig, CommandError> {
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
        if require_nonempty {
            return Err(config_error("No `fix.tools` are configured."));
        } else {
            writeln!(ui.warning_default(), "No `fix.tools` are configured.")?;
            return Ok(ToolsConfig { tools });
        }
    }
    tools.retain(|t| t.enabled);
    if tools.is_empty() {
        if require_nonempty {
            return Err(config_error(
                "At least one entry of `fix.tools` must be enabled.".to_string(),
            ));
        } else {
            writeln!(ui.warning_default(), "No `fix.tools` are enabled.")?;
        }
    }
    Ok(ToolsConfig { tools })
}
