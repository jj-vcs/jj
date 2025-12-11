// Copyright 2022 The Jujutsu Authors
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

use std::borrow::Cow;
use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::fmt::Debug;
use std::io;
use std::io::Write as _;
use std::mem;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;
use std::time::SystemTime;

use bstr::ByteVec as _;
use chrono::TimeZone as _;
use clap::ArgAction;
use clap::ArgMatches;
use clap::Command;
use clap::FromArgMatches as _;
use clap::builder::MapValueParser;
use clap::builder::NonEmptyStringValueParser;
use clap::builder::TypedValueParser as _;
use clap::builder::ValueParserFactory;
use clap::error::ContextKind;
use clap::error::ContextValue;
use clap_complete::ArgValueCandidates;
use clap_complete::ArgValueCompleter;
use indexmap::IndexMap;
use indexmap::IndexSet;
use indoc::indoc;
use indoc::writedoc;
use itertools::Itertools as _;
use jj_lib::backend::BackendResult;
use jj_lib::backend::ChangeId;
use jj_lib::backend::CommitId;
use jj_lib::backend::TreeValue;
use jj_lib::commit::Commit;
use jj_lib::config::ConfigGetError;
use jj_lib::config::ConfigGetResultExt as _;
use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigMigrationRule;
use jj_lib::config::ConfigNamePathBuf;
use jj_lib::config::ConfigSource;
use jj_lib::config::StackedConfig;
use jj_lib::conflicts::ConflictMarkerStyle;
use jj_lib::fileset;
use jj_lib::fileset::FilesetDiagnostics;
use jj_lib::fileset::FilesetExpression;
use jj_lib::gitignore::GitIgnoreError;
use jj_lib::gitignore::GitIgnoreFile;
use jj_lib::id_prefix::IdPrefixContext;
use jj_lib::lock::FileLock;
use jj_lib::matchers::Matcher;
use jj_lib::matchers::NothingMatcher;
use jj_lib::merge::Diff;
use jj_lib::merge::MergedTreeValue;
use jj_lib::merged_tree::MergedTree;
use jj_lib::object_id::ObjectId as _;
use jj_lib::op_heads_store;
use jj_lib::op_store::OpStoreError;
use jj_lib::op_store::OperationId;
use jj_lib::op_store::RefTarget;
use jj_lib::op_walk;
use jj_lib::op_walk::OpsetEvaluationError;
use jj_lib::operation::Operation;
use jj_lib::ref_name::RefName;
use jj_lib::ref_name::RefNameBuf;
use jj_lib::ref_name::RemoteName;
use jj_lib::ref_name::RemoteRefSymbol;
use jj_lib::ref_name::WorkspaceName;
use jj_lib::ref_name::WorkspaceNameBuf;
use jj_lib::repo::CheckOutCommitError;
use jj_lib::repo::EditCommitError;
use jj_lib::repo::MutableRepo;
use jj_lib::repo::ReadonlyRepo;
use jj_lib::repo::Repo;
use jj_lib::repo::RepoLoader;
use jj_lib::repo::StoreFactories;
use jj_lib::repo::StoreLoadError;
use jj_lib::repo::merge_factories_map;
use jj_lib::repo_path::RepoPath;
use jj_lib::repo_path::RepoPathBuf;
use jj_lib::repo_path::RepoPathUiConverter;
use jj_lib::repo_path::UiPathParseError;
use jj_lib::revset;
use jj_lib::revset::ResolvedRevsetExpression;
use jj_lib::revset::RevsetAliasesMap;
use jj_lib::revset::RevsetDiagnostics;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::RevsetExtensions;
use jj_lib::revset::RevsetFilterPredicate;
use jj_lib::revset::RevsetFunction;
use jj_lib::revset::RevsetIteratorExt as _;
use jj_lib::revset::RevsetModifier;
use jj_lib::revset::RevsetParseContext;
use jj_lib::revset::RevsetWorkspaceContext;
use jj_lib::revset::SymbolResolverExtension;
use jj_lib::revset::UserRevsetExpression;
use jj_lib::rewrite::restore_tree;
use jj_lib::settings::HumanByteSize;
use jj_lib::settings::UserSettings;
use jj_lib::store::Store;
use jj_lib::str_util::StringExpression;
use jj_lib::str_util::StringMatcher;
use jj_lib::str_util::StringPattern;
use jj_lib::transaction::Transaction;
use jj_lib::working_copy;
use jj_lib::working_copy::CheckoutStats;
use jj_lib::working_copy::LockedWorkingCopy;
use jj_lib::working_copy::SnapshotOptions;
use jj_lib::working_copy::SnapshotStats;
use jj_lib::working_copy::UntrackedReason;
use jj_lib::working_copy::WorkingCopy;
use jj_lib::working_copy::WorkingCopyFactory;
use jj_lib::working_copy::WorkingCopyFreshness;
use jj_lib::workspace::DefaultWorkspaceLoaderFactory;
use jj_lib::workspace::LockedWorkspace;
use jj_lib::workspace::WorkingCopyFactories;
use jj_lib::workspace::Workspace;
use jj_lib::workspace::WorkspaceLoadError;
use jj_lib::workspace::WorkspaceLoader;
use jj_lib::workspace::WorkspaceLoaderFactory;
use jj_lib::workspace::default_working_copy_factories;
use jj_lib::workspace::get_working_copy_factory;
use pollster::FutureExt as _;
use tracing::instrument;
use tracing_chrome::ChromeLayerBuilder;
use tracing_subscriber::prelude::*;

use crate::command_error::CommandError;
use crate::command_error::cli_error;
use crate::command_error::config_error_with_message;
use crate::command_error::handle_command_result;
use crate::command_error::internal_error;
use crate::command_error::internal_error_with_message;
use crate::command_error::print_parse_diagnostics;
use crate::command_error::user_error;
use crate::command_error::user_error_with_hint;
use crate::command_error::user_error_with_message;
use crate::commit_templater::CommitTemplateLanguage;
use crate::commit_templater::CommitTemplateLanguageExtension;
use crate::complete;
use crate::config::ConfigArgKind;
use crate::config::ConfigEnv;
use crate::config::RawConfig;
use crate::config::config_from_environment;
use crate::config::parse_config_args;
use crate::description_util::TextEditor;
use crate::diff_util;
use crate::diff_util::DiffFormat;
use crate::diff_util::DiffFormatArgs;
use crate::diff_util::DiffRenderer;
use crate::formatter::FormatRecorder;
use crate::formatter::Formatter;
use crate::formatter::FormatterExt as _;
use crate::merge_tools::DiffEditor;
use crate::merge_tools::MergeEditor;
use crate::merge_tools::MergeToolConfigError;
use crate::operation_templater::OperationTemplateLanguage;
use crate::operation_templater::OperationTemplateLanguageExtension;
use crate::revset_util;
use crate::revset_util::RevsetExpressionEvaluator;
use crate::revset_util::parse_union_name_patterns;
use crate::template_builder;
use crate::template_builder::TemplateLanguage;
use crate::template_parser::TemplateAliasesMap;
use crate::template_parser::TemplateDiagnostics;
use crate::templater::TemplateRenderer;
use crate::templater::WrapTemplateProperty;
use crate::text_util;
use crate::ui::ColorChoice;
use crate::ui::Ui;

const SHORT_CHANGE_ID_TEMPLATE_TEXT: &str = "format_short_change_id(self.change_id())";

#[derive(Clone)]
struct ChromeTracingFlushGuard {
    _inner: Option<Rc<tracing_chrome::FlushGuard>>,
}

impl Debug for ChromeTracingFlushGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let Self { _inner } = self;
        f.debug_struct("ChromeTracingFlushGuard")
            .finish_non_exhaustive()
    }
}

/// Handle to initialize or change tracing subscription.
#[derive(Clone, Debug)]
pub struct TracingSubscription {
    reload_log_filter: tracing_subscriber::reload::Handle<
        tracing_subscriber::EnvFilter,
        tracing_subscriber::Registry,
    >,
    _chrome_tracing_flush_guard: ChromeTracingFlushGuard,
}

impl TracingSubscription {
    const ENV_VAR_NAME: &str = "JJ_LOG";

    /// Initializes tracing with the default configuration. This should be
    /// called as early as possible.
    pub fn init() -> Self {
        let filter = tracing_subscriber::EnvFilter::builder()
            .with_default_directive(tracing::metadata::LevelFilter::ERROR.into())
            .with_env_var(Self::ENV_VAR_NAME)
            .from_env_lossy();
        let (filter, reload_log_filter) = tracing_subscriber::reload::Layer::new(filter);

        let (chrome_tracing_layer, chrome_tracing_flush_guard) = match std::env::var("JJ_TRACE") {
            Ok(filename) => {
                let filename = if filename.is_empty() {
                    format!(
                        "jj-trace-{}.json",
                        SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap()
                            .as_secs(),
                    )
                } else {
                    filename
                };
                let include_args = std::env::var("JJ_TRACE_INCLUDE_ARGS").is_ok();
                let (layer, guard) = ChromeLayerBuilder::new()
                    .file(filename)
                    .include_args(include_args)
                    .build();
                (
                    Some(layer),
                    ChromeTracingFlushGuard {
                        _inner: Some(Rc::new(guard)),
                    },
                )
            }
            Err(_) => (None, ChromeTracingFlushGuard { _inner: None }),
        };

        tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::Layer::default()
                    .with_writer(std::io::stderr)
                    .with_filter(filter),
            )
            .with(chrome_tracing_layer)
            .init();
        Self {
            reload_log_filter,
            _chrome_tracing_flush_guard: chrome_tracing_flush_guard,
        }
    }

    pub fn enable_debug_logging(&self) -> Result<(), CommandError> {
        self.reload_log_filter
            .modify(|filter| {
                // The default is INFO.
                // jj-lib and jj-cli are whitelisted for DEBUG logging.
                // This ensures that other crates' logging doesn't show up by default.
                *filter = tracing_subscriber::EnvFilter::builder()
                    .with_default_directive(tracing::metadata::LevelFilter::INFO.into())
                    .with_env_var(Self::ENV_VAR_NAME)
                    .from_env_lossy()
                    .add_directive("jj_lib=debug".parse().unwrap())
                    .add_directive("jj_cli=debug".parse().unwrap());
            })
            .map_err(|err| internal_error_with_message("failed to enable debug logging", err))?;
        tracing::info!("debug logging enabled");
        Ok(())
    }
}

#[derive(Clone)]
pub struct CommandHelper {
    data: Rc<CommandHelperData>,
}

struct CommandHelperData {
    app: Command,
    cwd: PathBuf,
    string_args: Vec<String>,
    matches: ArgMatches,
    global_args: GlobalArgs,
    config_env: ConfigEnv,
    config_migrations: Vec<ConfigMigrationRule>,
    raw_config: RawConfig,
    settings: UserSettings,
    revset_extensions: Arc<RevsetExtensions>,
    commit_template_extensions: Vec<Arc<dyn CommitTemplateLanguageExtension>>,
    operation_template_extensions: Vec<Arc<dyn OperationTemplateLanguageExtension>>,
    maybe_workspace_loader: Result<Box<dyn WorkspaceLoader>, CommandError>,
    store_factories: StoreFactories,
    working_copy_factories: WorkingCopyFactories,
    workspace_loader_factory: Box<dyn WorkspaceLoaderFactory>,
}

impl CommandHelper {
    pub fn app(&self) -> &Command {
        &self.data.app
    }

    /// Canonical form of the current working directory path.
    ///
    /// A loaded `Workspace::workspace_root()` also returns a canonical path, so
    /// relative paths can be easily computed from these paths.
    pub fn cwd(&self) -> &Path {
        &self.data.cwd
    }

    pub fn string_args(&self) -> &Vec<String> {
        &self.data.string_args
    }

    pub fn matches(&self) -> &ArgMatches {
        &self.data.matches
    }

    pub fn global_args(&self) -> &GlobalArgs {
        &self.data.global_args
    }

    pub fn config_env(&self) -> &ConfigEnv {
        &self.data.config_env
    }

    /// Unprocessed (or unresolved) configuration data.
    ///
    /// Use this only if the unmodified config data is needed. For example, `jj
    /// config set` should use this to write updated data back to file.
    pub fn raw_config(&self) -> &RawConfig {
        &self.data.raw_config
    }

    /// Settings for the current command and workspace.
    ///
    /// This may be different from the settings for new workspace created by
    /// e.g. `jj git init`. There may be conditional variables and repo config
    /// `.jj/repo/config.toml` loaded for the cwd workspace.
    pub fn settings(&self) -> &UserSettings {
        &self.data.settings
    }

    /// Resolves configuration for new workspace located at the specified path.
    pub fn settings_for_new_workspace(
        &self,
        workspace_root: &Path,
    ) -> Result<UserSettings, CommandError> {
        let mut config_env = self.data.config_env.clone();
        let mut raw_config = self.data.raw_config.clone();
        let repo_path = workspace_root.join(".jj").join("repo");
        config_env.reset_repo_path(&repo_path);
        config_env.reload_repo_config(&mut raw_config)?;
        config_env.reset_workspace_path(workspace_root);
        config_env.reload_workspace_config(&mut raw_config)?;
        let mut config = config_env.resolve_config(&raw_config)?;
        // No migration messages here, which would usually be emitted before.
        jj_lib::config::migrate(&mut config, &self.data.config_migrations)?;
        Ok(self.data.settings.with_new_config(config)?)
    }

    /// Loads text editor from the settings.
    pub fn text_editor(&self) -> Result<TextEditor, ConfigGetError> {
        TextEditor::from_settings(self.settings())
    }

    pub fn revset_extensions(&self) -> &Arc<RevsetExtensions> {
        &self.data.revset_extensions
    }

    /// Parses template of the given language into evaluation tree.
    ///
    /// This function also loads template aliases from the settings. Use
    /// `WorkspaceCommandHelper::parse_template()` if you've already
    /// instantiated the workspace helper.
    pub fn parse_template<'a, C, L>(
        &self,
        ui: &Ui,
        language: &L,
        template_text: &str,
    ) -> Result<TemplateRenderer<'a, C>, CommandError>
    where
        C: Clone + 'a,
        L: TemplateLanguage<'a> + ?Sized,
        L::Property: WrapTemplateProperty<'a, C>,
    {
        let mut diagnostics = TemplateDiagnostics::new();
        let aliases = load_template_aliases(ui, self.settings().config())?;
        let template =
            template_builder::parse(language, &mut diagnostics, template_text, &aliases)?;
        print_parse_diagnostics(ui, "In template expression", &diagnostics)?;
        Ok(template)
    }

    pub fn workspace_loader(&self) -> Result<&dyn WorkspaceLoader, CommandError> {
        self.data
            .maybe_workspace_loader
            .as_deref()
            .map_err(Clone::clone)
    }

    fn new_workspace_loader_at(
        &self,
        workspace_root: &Path,
    ) -> Result<Box<dyn WorkspaceLoader>, CommandError> {
        self.data
            .workspace_loader_factory
            .create(workspace_root)
            .map_err(|err| map_workspace_load_error(err, None))
    }

    /// Loads workspace and repo, then snapshots the working copy if allowed.
    #[instrument(skip(self, ui))]
    pub fn workspace_helper(&self, ui: &Ui) -> Result<WorkspaceCommandHelper, CommandError> {
        let (workspace_command, stats) = self.workspace_helper_with_stats(ui)?;
        print_snapshot_stats(ui, &stats, workspace_command.env().path_converter())?;
        Ok(workspace_command)
    }

    /// Loads workspace and repo, then snapshots the working copy if allowed and
    /// returns the SnapshotStats.
    ///
    /// Note that unless you have a good reason not to do so, you should always
    /// call [`print_snapshot_stats`] with the [`SnapshotStats`] returned by
    /// this function to present possible untracked files to the user.
    #[instrument(skip(self, ui))]
    pub fn workspace_helper_with_stats(
        &self,
        ui: &Ui,
    ) -> Result<(WorkspaceCommandHelper, SnapshotStats), CommandError> {
        let mut workspace_command = self.workspace_helper_no_snapshot(ui)?;

        let (workspace_command, stats) = match workspace_command.maybe_snapshot_impl(ui) {
            Ok(stats) => (workspace_command, stats),
            Err(SnapshotWorkingCopyError::Command(err)) => return Err(err),
            Err(SnapshotWorkingCopyError::StaleWorkingCopy(err)) => {
                let auto_update_stale = self.settings().get_bool("snapshot.auto-update-stale")?;
                if !auto_update_stale {
                    return Err(err);
                }

                // We detected the working copy was stale and the client is configured to
                // auto-update-stale, so let's do that now. We need to do it up here, not at a
                // lower level (e.g. inside snapshot_working_copy()) to avoid recursive locking
                // of the working copy.
                self.recover_stale_working_copy(ui)?
            }
        };

        Ok((workspace_command, stats))
    }

    /// Loads workspace and repo, but never snapshots the working copy. Most
    /// commands should use `workspace_helper()` instead.
    #[instrument(skip(self, ui))]
    pub fn workspace_helper_no_snapshot(
        &self,
        ui: &Ui,
    ) -> Result<WorkspaceCommandHelper, CommandError> {
        let workspace = self.load_workspace()?;
        let op_head = self.resolve_operation(ui, workspace.repo_loader())?;
        let repo = workspace.repo_loader().load_at(&op_head)?;
        let env = self.workspace_environment(ui, &workspace)?;
        revset_util::warn_unresolvable_trunk(ui, repo.as_ref(), &env.revset_parse_context())?;
        WorkspaceCommandHelper::new(ui, workspace, repo, env, self.is_at_head_operation())
    }

    pub fn get_working_copy_factory(&self) -> Result<&dyn WorkingCopyFactory, CommandError> {
        let loader = self.workspace_loader()?;

        // We convert StoreLoadError -> WorkspaceLoadError -> CommandError
        let factory: Result<_, WorkspaceLoadError> =
            get_working_copy_factory(loader, &self.data.working_copy_factories)
                .map_err(|e| e.into());
        let factory = factory.map_err(|err| {
            map_workspace_load_error(err, self.data.global_args.repository.as_deref())
        })?;
        Ok(factory)
    }

    /// Loads workspace for the current command.
    #[instrument(skip_all)]
    pub fn load_workspace(&self) -> Result<Workspace, CommandError> {
        let loader = self.workspace_loader()?;
        loader
            .load(
                &self.data.settings,
                &self.data.store_factories,
                &self.data.working_copy_factories,
            )
            .map_err(|err| {
                map_workspace_load_error(err, self.data.global_args.repository.as_deref())
            })
    }

    /// Loads workspace located at the specified path.
    #[instrument(skip(self, settings))]
    pub fn load_workspace_at(
        &self,
        workspace_root: &Path,
        settings: &UserSettings,
    ) -> Result<Workspace, CommandError> {
        let loader = self.new_workspace_loader_at(workspace_root)?;
        loader
            .load(
                settings,
                &self.data.store_factories,
                &self.data.working_copy_factories,
            )
            .map_err(|err| map_workspace_load_error(err, None))
    }

    /// Note that unless you have a good reason not to do so, you should always
    /// call [`print_snapshot_stats`] with the [`SnapshotStats`] returned by
    /// this function to present possible untracked files to the user.
    pub fn recover_stale_working_copy(
        &self,
        ui: &Ui,
    ) -> Result<(WorkspaceCommandHelper, SnapshotStats), CommandError> {
        let workspace = self.load_workspace()?;
        let op_id = workspace.working_copy().operation_id();

        match workspace.repo_loader().load_operation(op_id) {
            Ok(op) => {
                let repo = workspace.repo_loader().load_at(&op)?;
                let mut workspace_command = self.for_workable_repo(ui, workspace, repo)?;
                workspace_command.check_working_copy_writable()?;

                // Snapshot the current working copy on top of the last known working-copy
                // operation, then merge the divergent operations. The wc_commit_id of the
                // merged repo wouldn't change because the old one wins, but it's probably
                // fine if we picked the new wc_commit_id.
                let stale_stats = workspace_command
                    .snapshot_working_copy(ui)
                    .map_err(|err| err.into_command_error())?;

                let wc_commit_id = workspace_command.get_wc_commit_id().unwrap();
                let repo = workspace_command.repo().clone();
                let stale_wc_commit = repo.store().get_commit(wc_commit_id)?;

                let mut workspace_command = self.workspace_helper_no_snapshot(ui)?;

                let repo = workspace_command.repo().clone();
                let (mut locked_ws, desired_wc_commit) =
                    workspace_command.unchecked_start_working_copy_mutation()?;
                match WorkingCopyFreshness::check_stale(
                    locked_ws.locked_wc(),
                    &desired_wc_commit,
                    &repo,
                )? {
                    WorkingCopyFreshness::Fresh | WorkingCopyFreshness::Updated(_) => {
                        drop(locked_ws);
                        writeln!(
                            ui.status(),
                            "Attempted recovery, but the working copy is not stale"
                        )?;
                    }
                    WorkingCopyFreshness::WorkingCopyStale
                    | WorkingCopyFreshness::SiblingOperation => {
                        let stats = update_stale_working_copy(
                            locked_ws,
                            repo.op_id().clone(),
                            &stale_wc_commit,
                            &desired_wc_commit,
                        )?;
                        workspace_command.print_updated_working_copy_stats(
                            ui,
                            Some(&stale_wc_commit),
                            &desired_wc_commit,
                            &stats,
                        )?;
                        writeln!(
                            ui.status(),
                            "Updated working copy to fresh commit {}",
                            short_commit_hash(desired_wc_commit.id())
                        )?;
                    }
                }

                // There may be Git refs to import, so snapshot again. Git HEAD
                // will also be imported if it was updated after the working
                // copy became stale. The result wouldn't be ideal, but there
                // should be no data loss at least.
                let fresh_stats = workspace_command
                    .maybe_snapshot_impl(ui)
                    .map_err(|err| err.into_command_error())?;
                let merged_stats = {
                    let SnapshotStats {
                        mut untracked_paths,
                    } = stale_stats;
                    untracked_paths.extend(fresh_stats.untracked_paths);
                    SnapshotStats { untracked_paths }
                };
                Ok((workspace_command, merged_stats))
            }
            Err(e @ OpStoreError::ObjectNotFound { .. }) => {
                writeln!(
                    ui.status(),
                    "Failed to read working copy's current operation; attempting recovery. Error \
                     message from read attempt: {e}"
                )?;

                let mut workspace_command = self.workspace_helper_no_snapshot(ui)?;
                let stats = workspace_command.create_and_check_out_recovery_commit(ui)?;
                Ok((workspace_command, stats))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Loads command environment for the given `workspace`.
    pub fn workspace_environment(
        &self,
        ui: &Ui,
        workspace: &Workspace,
    ) -> Result<WorkspaceCommandEnvironment, CommandError> {
        WorkspaceCommandEnvironment::new(ui, self, workspace)
    }

    /// Returns true if the working copy to be loaded is writable, and therefore
    /// should usually be snapshotted.
    pub fn is_working_copy_writable(&self) -> bool {
        self.is_at_head_operation() && !self.data.global_args.ignore_working_copy
    }

    /// Returns true if the current operation is considered to be the head.
    pub fn is_at_head_operation(&self) -> bool {
        // TODO: should we accept --at-op=<head_id> as the head op? or should we
        // make --at-op=@ imply --ignore-working-copy (i.e. not at the head.)
        matches!(
            self.data.global_args.at_operation.as_deref(),
            None | Some("@")
        )
    }

    /// Resolves the current operation from the command-line argument.
    ///
    /// If no `--at-operation` is specified, the head operations will be
    /// loaded. If there are multiple heads, they'll be merged.
    #[instrument(skip_all)]
    pub fn resolve_operation(
        &self,
        ui: &Ui,
        repo_loader: &RepoLoader,
    ) -> Result<Operation, CommandError> {
        if let Some(op_str) = &self.data.global_args.at_operation {
            Ok(op_walk::resolve_op_for_load(repo_loader, op_str)?)
        } else {
            op_heads_store::resolve_op_heads(
                repo_loader.op_heads_store().as_ref(),
                repo_loader.op_store(),
                |op_heads| {
                    writeln!(
                        ui.status(),
                        "Concurrent modification detected, resolving automatically.",
                    )?;
                    let base_repo = repo_loader.load_at(&op_heads[0])?;
                    // TODO: It may be helpful to print each operation we're merging here
                    let mut tx = start_repo_transaction(&base_repo, &self.data.string_args);
                    for other_op_head in op_heads.into_iter().skip(1) {
                        tx.merge_operation(other_op_head)?;
                        let num_rebased = tx.repo_mut().rebase_descendants()?;
                        if num_rebased > 0 {
                            writeln!(
                                ui.status(),
                                "Rebased {num_rebased} descendant commits onto commits rewritten \
                                 by other operation"
                            )?;
                        }
                    }
                    Ok(tx
                        .write("reconcile divergent operations")?
                        .leave_unpublished()
                        .operation()
                        .clone())
                },
            )
        }
    }

    /// Creates helper for the repo whose view is supposed to be in sync with
    /// the working copy. If `--ignore-working-copy` is not specified, the
    /// returned helper will attempt to update the working copy.
    #[instrument(skip_all)]
    pub fn for_workable_repo(
        &self,
        ui: &Ui,
        workspace: Workspace,
        repo: Arc<ReadonlyRepo>,
    ) -> Result<WorkspaceCommandHelper, CommandError> {
        let env = self.workspace_environment(ui, &workspace)?;
        let loaded_at_head = true;
        WorkspaceCommandHelper::new(ui, workspace, repo, env, loaded_at_head)
    }
}

/// A ReadonlyRepo along with user-config-dependent derived data. The derived
/// data is lazily loaded.
struct ReadonlyUserRepo {
    repo: Arc<ReadonlyRepo>,
    id_prefix_context: OnceCell<IdPrefixContext>,
}

impl ReadonlyUserRepo {
    fn new(repo: Arc<ReadonlyRepo>) -> Self {
        Self {
            repo,
            id_prefix_context: OnceCell::new(),
        }
    }
}

/// A advanceable bookmark to satisfy the "advance-bookmarks" feature.
///
/// This is a helper for `WorkspaceCommandTransaction`. It provides a
/// type-safe way to separate the work of checking whether a bookmark
/// can be advanced and actually advancing it. Advancing the bookmark
/// never fails, but can't be done until the new `CommitId` is
/// available. Splitting the work in this way also allows us to
/// identify eligible bookmarks without actually moving them and
/// return config errors to the user early.
pub struct AdvanceableBookmark {
    name: RefNameBuf,
    old_commit_id: CommitId,
}

/// Parses advance-bookmarks settings into matcher.
///
/// Settings are configured in the jj config.toml as lists of string matcher
/// expressions for enabled and disabled bookmarks. Example:
/// ```toml
/// [experimental-advance-branches]
/// # Enable the feature for all branches except "main".
/// enabled-branches = ["glob:*"]
/// disabled-branches = ["main"]
/// ```
fn load_advance_bookmarks_matcher(
    ui: &Ui,
    settings: &UserSettings,
) -> Result<Option<StringMatcher>, CommandError> {
    let get_setting = |setting_key: &str| -> Result<Vec<String>, _> {
        let name = ConfigNamePathBuf::from_iter(["experimental-advance-branches", setting_key]);
        settings.get(&name)
    };
    // TODO: When we stabilize this feature, enabled/disabled patterns can be
    // combined into a single matcher expression.
    let enabled_names = get_setting("enabled-branches")?;
    let disabled_names = get_setting("disabled-branches")?;
    let enabled_expr = parse_union_name_patterns(ui, &enabled_names)?;
    let disabled_expr = parse_union_name_patterns(ui, &disabled_names)?;
    if enabled_names.is_empty() {
        Ok(None)
    } else {
        let expr = enabled_expr.intersection(disabled_expr.negated());
        Ok(Some(expr.to_matcher()))
    }
}

/// Metadata and configuration loaded for a specific workspace.
pub struct WorkspaceCommandEnvironment {
    command: CommandHelper,
    settings: UserSettings,
    revset_aliases_map: RevsetAliasesMap,
    template_aliases_map: TemplateAliasesMap,
    default_ignored_remote: Option<&'static RemoteName>,
    revsets_use_glob_by_default: bool,
    path_converter: RepoPathUiConverter,
    workspace_name: WorkspaceNameBuf,
    immutable_heads_expression: Arc<UserRevsetExpression>,
    short_prefixes_expression: Option<Arc<UserRevsetExpression>>,
    conflict_marker_style: ConflictMarkerStyle,
}

impl WorkspaceCommandEnvironment {
    #[instrument(skip_all)]
    fn new(ui: &Ui, command: &CommandHelper, workspace: &Workspace) -> Result<Self, CommandError> {
        let settings = workspace.settings();
        let revset_aliases_map = revset_util::load_revset_aliases(ui, settings.config())?;
        let template_aliases_map = load_template_aliases(ui, settings.config())?;
        let default_ignored_remote = default_ignored_remote_name(workspace.repo_loader().store());
        let path_converter = RepoPathUiConverter::Fs {
            cwd: command.cwd().to_owned(),
            base: workspace.workspace_root().to_owned(),
        };
        let mut env = Self {
            command: command.clone(),
            settings: settings.clone(),
            revset_aliases_map,
            template_aliases_map,
            default_ignored_remote,
            revsets_use_glob_by_default: settings.get("ui.revsets-use-glob-by-default")?,
            path_converter,
            workspace_name: workspace.workspace_name().to_owned(),
            immutable_heads_expression: RevsetExpression::root(),
            short_prefixes_expression: None,
            conflict_marker_style: settings.get("ui.conflict-marker-style")?,
        };
        env.immutable_heads_expression = env.load_immutable_heads_expression(ui)?;
        env.short_prefixes_expression = env.load_short_prefixes_expression(ui)?;
        Ok(env)
    }

    pub(crate) fn path_converter(&self) -> &RepoPathUiConverter {
        &self.path_converter
    }

    pub fn workspace_name(&self) -> &WorkspaceName {
        &self.workspace_name
    }

    pub(crate) fn revset_parse_context(&self) -> RevsetParseContext<'_> {
        let workspace_context = RevsetWorkspaceContext {
            path_converter: &self.path_converter,
            workspace_name: &self.workspace_name,
        };
        let now = if let Some(timestamp) = self.settings.commit_timestamp() {
            chrono::Local
                .timestamp_millis_opt(timestamp.timestamp.0)
                .unwrap()
        } else {
            chrono::Local::now()
        };
        RevsetParseContext {
            aliases_map: &self.revset_aliases_map,
            local_variables: HashMap::new(),
            user_email: self.settings.user_email(),
            date_pattern_context: now.into(),
            default_ignored_remote: self.default_ignored_remote,
            use_glob_by_default: self.revsets_use_glob_by_default,
            extensions: self.command.revset_extensions(),
            workspace: Some(workspace_context),
        }
    }

    /// Creates fresh new context which manages cache of short commit/change ID
    /// prefixes. New context should be created per repo view (or operation.)
    pub fn new_id_prefix_context(&self) -> IdPrefixContext {
        let context = IdPrefixContext::new(self.command.revset_extensions().clone());
        match &self.short_prefixes_expression {
            None => context,
            Some(expression) => context.disambiguate_within(expression.clone()),
        }
    }

    /// User-configured expression defining the immutable set.
    pub fn immutable_expression(&self) -> Arc<UserRevsetExpression> {
        // Negated ancestors expression `~::(<heads> | root())` is slightly
        // easier to optimize than negated union `~(::<heads> | root())`.
        self.immutable_heads_expression.ancestors()
    }

    /// User-configured expression defining the heads of the immutable set.
    pub fn immutable_heads_expression(&self) -> &Arc<UserRevsetExpression> {
        &self.immutable_heads_expression
    }

    /// User-configured conflict marker style for materializing conflicts
    pub fn conflict_marker_style(&self) -> ConflictMarkerStyle {
        self.conflict_marker_style
    }

    fn load_immutable_heads_expression(
        &self,
        ui: &Ui,
    ) -> Result<Arc<UserRevsetExpression>, CommandError> {
        let mut diagnostics = RevsetDiagnostics::new();
        let expression = revset_util::parse_immutable_heads_expression(
            &mut diagnostics,
            &self.revset_parse_context(),
        )
        .map_err(|e| config_error_with_message("Invalid `revset-aliases.immutable_heads()`", e))?;
        print_parse_diagnostics(ui, "In `revset-aliases.immutable_heads()`", &diagnostics)?;
        Ok(expression)
    }

    fn load_short_prefixes_expression(
        &self,
        ui: &Ui,
    ) -> Result<Option<Arc<UserRevsetExpression>>, CommandError> {
        let revset_string = self
            .settings
            .get_string("revsets.short-prefixes")
            .optional()?
            .map_or_else(|| self.settings.get_string("revsets.log"), Ok)?;
        if revset_string.is_empty() {
            Ok(None)
        } else {
            let mut diagnostics = RevsetDiagnostics::new();
            let (expression, modifier) = revset::parse_with_modifier(
                &mut diagnostics,
                &revset_string,
                &self.revset_parse_context(),
            )
            .map_err(|err| config_error_with_message("Invalid `revsets.short-prefixes`", err))?;
            print_parse_diagnostics(ui, "In `revsets.short-prefixes`", &diagnostics)?;
            let (None | Some(RevsetModifier::All)) = modifier;
            Ok(Some(expression))
        }
    }

    /// Returns first immutable commit.
    fn find_immutable_commit(
        &self,
        repo: &dyn Repo,
        to_rewrite_expr: &Arc<ResolvedRevsetExpression>,
    ) -> Result<Option<CommitId>, CommandError> {
        let immutable_expression = if self.command.global_args().ignore_immutable {
            UserRevsetExpression::root()
        } else {
            self.immutable_expression()
        };

        // Not using self.id_prefix_context() because the disambiguation data
        // must not be calculated and cached against arbitrary repo. It's also
        // unlikely that the immutable expression contains short hashes.
        let id_prefix_context = IdPrefixContext::new(self.command.revset_extensions().clone());
        let immutable_expr = RevsetExpressionEvaluator::new(
            repo,
            self.command.revset_extensions().clone(),
            &id_prefix_context,
            immutable_expression,
        )
        .resolve()
        .map_err(|e| config_error_with_message("Invalid `revset-aliases.immutable_heads()`", e))?;

        let mut commit_id_iter = immutable_expr
            .intersection(to_rewrite_expr)
            .evaluate(repo)?
            .iter();
        Ok(commit_id_iter.next().transpose()?)
    }

    pub fn template_aliases_map(&self) -> &TemplateAliasesMap {
        &self.template_aliases_map
    }

    /// Parses template of the given language into evaluation tree.
    pub fn parse_template<'a, C, L>(
        &self,
        ui: &Ui,
        language: &L,
        template_text: &str,
    ) -> Result<TemplateRenderer<'a, C>, CommandError>
    where
        C: Clone + 'a,
        L: TemplateLanguage<'a> + ?Sized,
        L::Property: WrapTemplateProperty<'a, C>,
    {
        let mut diagnostics = TemplateDiagnostics::new();
        let template = template_builder::parse(
            language,
            &mut diagnostics,
            template_text,
            &self.template_aliases_map,
        )?;
        print_parse_diagnostics(ui, "In template expression", &diagnostics)?;
        Ok(template)
    }

    /// Creates commit template language environment for this workspace and the
    /// given `repo`.
    pub fn commit_template_language<'a>(
        &'a self,
        repo: &'a dyn Repo,
        id_prefix_context: &'a IdPrefixContext,
    ) -> CommitTemplateLanguage<'a> {
        CommitTemplateLanguage::new(
            repo,
            &self.path_converter,
            &self.workspace_name,
            self.revset_parse_context(),
            id_prefix_context,
            self.immutable_expression(),
            self.conflict_marker_style,
            &self.command.data.commit_template_extensions,
        )
    }

    pub fn operation_template_extensions(&self) -> &[Arc<dyn OperationTemplateLanguageExtension>] {
        &self.command.data.operation_template_extensions
    }
}

/// A token that holds a lock for git import/export operations in colocated
/// repositories. For non-colocated repos, this is an empty token (no actual
/// lock held). The lock is automatically released when this token is dropped.
pub struct GitImportExportLock {
    _lock: Option<FileLock>,
}

/// Provides utilities for writing a command that works on a [`Workspace`]
/// (which most commands do).
pub struct WorkspaceCommandHelper {
    workspace: Workspace,
    user_repo: ReadonlyUserRepo,
    env: WorkspaceCommandEnvironment,
    // TODO: Parsed template can be cached if it doesn't capture 'repo lifetime
    commit_summary_template_text: String,
    op_summary_template_text: String,
    may_update_working_copy: bool,
    working_copy_shared_with_git: bool,
}

enum SnapshotWorkingCopyError {
    Command(CommandError),
    StaleWorkingCopy(CommandError),
}

impl SnapshotWorkingCopyError {
    fn into_command_error(self) -> CommandError {
        match self {
            Self::Command(err) => err,
            Self::StaleWorkingCopy(err) => err,
        }
    }
}

fn snapshot_command_error<E>(err: E) -> SnapshotWorkingCopyError
where
    E: Into<CommandError>,
{
    SnapshotWorkingCopyError::Command(err.into())
}

impl WorkspaceCommandHelper {
    #[instrument(skip_all)]
    fn new(
        ui: &Ui,
        workspace: Workspace,
        repo: Arc<ReadonlyRepo>,
        env: WorkspaceCommandEnvironment,
        loaded_at_head: bool,
    ) -> Result<Self, CommandError> {
        let settings = workspace.settings();
        let commit_summary_template_text = settings.get_string("templates.commit_summary")?;
        let op_summary_template_text = settings.get_string("templates.op_summary")?;
        let may_update_working_copy =
            loaded_at_head && !env.command.global_args().ignore_working_copy;
        let working_copy_shared_with_git =
            crate::git_util::is_colocated_git_workspace(&workspace, &repo);

        let helper = Self {
            workspace,
            user_repo: ReadonlyUserRepo::new(repo),
            env,
            commit_summary_template_text,
            op_summary_template_text,
            may_update_working_copy,
            working_copy_shared_with_git,
        };
        // Parse commit_summary template early to report error before starting
        // mutable operation.
        helper.parse_operation_template(ui, &helper.op_summary_template_text)?;
        helper.parse_commit_template(ui, &helper.commit_summary_template_text)?;
        helper.parse_commit_template(ui, SHORT_CHANGE_ID_TEMPLATE_TEXT)?;
        Ok(helper)
    }

    /// Settings for this workspace.
    pub fn settings(&self) -> &UserSettings {
        self.workspace.settings()
    }

    pub fn check_working_copy_writable(&self) -> Result<(), CommandError> {
        if self.may_update_working_copy {
            Ok(())
        } else {
            let hint = if self.env.command.global_args().ignore_working_copy {
                "Don't use --ignore-working-copy."
            } else {
                "Don't use --at-op."
            };
            Err(user_error_with_hint(
                "This command must be able to update the working copy.",
                hint,
            ))
        }
    }

    /// Acquires a lock for git import/export operations if the workspace is
    /// colocated with Git. Returns a token that can be passed to functions
    /// that need to import from or export to Git. For non-colocated repos,
    /// returns a token with no lock inside.
    fn lock_git_import_export(&self) -> Result<GitImportExportLock, CommandError> {
        let lock = if self.working_copy_shared_with_git {
            let lock_path = self.workspace.repo_path().join("git_import_export.lock");
            Some(FileLock::lock(lock_path.clone()).map_err(|err| {
                user_error_with_message("Failed to take lock for Git import/export", err)
            })?)
        } else {
            None
        };
        Ok(GitImportExportLock { _lock: lock })
    }

    /// Note that unless you have a good reason not to do so, you should always
    /// call [`print_snapshot_stats`] with the [`SnapshotStats`] returned by
    /// this function to present possible untracked files to the user.
    #[instrument(skip_all)]
    fn maybe_snapshot_impl(&mut self, ui: &Ui) -> Result<SnapshotStats, SnapshotWorkingCopyError> {
        if !self.may_update_working_copy {
            return Ok(SnapshotStats::default());
        }

        // Acquire git import/export lock once for the entire import/snapshot/export
        // cycle. This prevents races with other processes during Git HEAD and
        // refs import/export.
        let git_import_export_lock = self
            .lock_git_import_export()
            .map_err(snapshot_command_error)?;

        // Reload at current head to avoid creating divergent operations if another
        // process committed an operation while we were waiting for the lock.
        if self.working_copy_shared_with_git {
            let repo = self.repo().clone();
            let op_heads_store = repo.loader().op_heads_store();
            let op_heads = op_heads_store
                .get_op_heads()
                .block_on()
                .map_err(snapshot_command_error)?;
            if std::slice::from_ref(repo.op_id()) != op_heads {
                let op = self
                    .env
                    .command
                    .resolve_operation(ui, repo.loader())
                    .map_err(snapshot_command_error)?;
                let current_repo = repo.loader().load_at(&op).map_err(snapshot_command_error)?;
                self.user_repo = ReadonlyUserRepo::new(current_repo);
            }
        }

        #[cfg(feature = "git")]
        if self.working_copy_shared_with_git {
            self.import_git_head(ui, &git_import_export_lock)
                .map_err(snapshot_command_error)?;
        }
        // Because the Git refs (except HEAD) aren't imported yet, the ref
        // pointing to the new working-copy commit might not be exported.
        // In that situation, the ref would be conflicted anyway, so export
        // failure is okay.
        let stats = self.snapshot_working_copy(ui)?;

        // import_git_refs() can rebase the working-copy commit.
        #[cfg(feature = "git")]
        if self.working_copy_shared_with_git {
            self.import_git_refs(ui, &git_import_export_lock)
                .map_err(snapshot_command_error)?;
        }
        Ok(stats)
    }

    /// Snapshot the working copy if allowed, and import Git refs if the working
    /// copy is collocated with Git.
    #[instrument(skip_all)]
    pub fn maybe_snapshot(&mut self, ui: &Ui) -> Result<(), CommandError> {
        let stats = self
            .maybe_snapshot_impl(ui)
            .map_err(|err| err.into_command_error())?;
        print_snapshot_stats(ui, &stats, self.env().path_converter())?;
        Ok(())
    }

    /// Imports new HEAD from the colocated Git repo.
    ///
    /// If the Git HEAD has changed, this function checks out the new Git HEAD.
    /// The old working-copy commit will be abandoned if it's discardable. The
    /// working-copy state will be reset to point to the new Git HEAD. The
    /// working-copy contents won't be updated.
    #[cfg(feature = "git")]
    #[instrument(skip_all)]
    fn import_git_head(
        &mut self,
        ui: &Ui,
        git_import_export_lock: &GitImportExportLock,
    ) -> Result<(), CommandError> {
        assert!(self.may_update_working_copy);
        let mut tx = self.start_transaction();
        jj_lib::git::import_head(tx.repo_mut())?;
        if !tx.repo().has_changes() {
            return Ok(());
        }

        let mut tx = tx.into_inner();
        let old_git_head = self.repo().view().git_head().clone();
        let new_git_head = tx.repo().view().git_head().clone();
        if let Some(new_git_head_id) = new_git_head.as_normal() {
            let workspace_name = self.workspace_name().to_owned();
            let new_git_head_commit = tx.repo().store().get_commit(new_git_head_id)?;
            let wc_commit = tx
                .repo_mut()
                .check_out(workspace_name, &new_git_head_commit)?;
            let mut locked_ws = self.workspace.start_working_copy_mutation()?;
            // The working copy was presumably updated by the git command that updated
            // HEAD, so we just need to reset our working copy
            // state to it without updating working copy files.
            locked_ws.locked_wc().reset(&wc_commit).block_on()?;
            tx.repo_mut().rebase_descendants()?;
            self.user_repo = ReadonlyUserRepo::new(tx.commit("import git head")?);
            locked_ws.finish(self.user_repo.repo.op_id().clone())?;
            if old_git_head.is_present() {
                writeln!(
                    ui.status(),
                    "Reset the working copy parent to the new Git HEAD."
                )?;
            } else {
                // Don't print verbose message on initial checkout.
            }
        } else {
            // Unlikely, but the HEAD ref got deleted by git?
            self.finish_transaction(ui, tx, "import git head", git_import_export_lock)?;
        }
        Ok(())
    }

    /// Imports branches and tags from the underlying Git repo, abandons old
    /// bookmarks.
    ///
    /// If the working-copy branch is rebased, and if update is allowed, the
    /// new working-copy commit will be checked out.
    ///
    /// This function does not import the Git HEAD, but the HEAD may be reset to
    /// the working copy parent if the repository is colocated.
    #[cfg(feature = "git")]
    #[instrument(skip_all)]
    fn import_git_refs(
        &mut self,
        ui: &Ui,
        git_import_export_lock: &GitImportExportLock,
    ) -> Result<(), CommandError> {
        use jj_lib::git;
        let git_settings = git::GitSettings::from_settings(self.settings())?;
        let remote_settings = self.settings().remote_settings()?;
        let import_options =
            crate::git_util::load_git_import_options(ui, &git_settings, &remote_settings)?;
        let mut tx = self.start_transaction();
        let stats = git::import_refs(tx.repo_mut(), &import_options)?;
        crate::git_util::print_git_import_stats(ui, tx.repo(), &stats, false)?;
        if !tx.repo().has_changes() {
            return Ok(());
        }

        let mut tx = tx.into_inner();
        // Rebase here to show slightly different status message.
        let num_rebased = tx.repo_mut().rebase_descendants()?;
        if num_rebased > 0 {
            writeln!(
                ui.status(),
                "Rebased {num_rebased} descendant commits off of commits rewritten from git"
            )?;
        }
        self.finish_transaction(ui, tx, "import git refs", git_import_export_lock)?;
        writeln!(
            ui.status(),
            "Done importing changes from the underlying Git repo."
        )?;
        Ok(())
    }

    pub fn repo(&self) -> &Arc<ReadonlyRepo> {
        &self.user_repo.repo
    }

    pub fn repo_path(&self) -> &Path {
        self.workspace.repo_path()
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub fn working_copy(&self) -> &dyn WorkingCopy {
        self.workspace.working_copy()
    }

    pub fn env(&self) -> &WorkspaceCommandEnvironment {
        &self.env
    }

    pub fn unchecked_start_working_copy_mutation(
        &mut self,
    ) -> Result<(LockedWorkspace<'_>, Commit), CommandError> {
        self.check_working_copy_writable()?;
        let wc_commit = if let Some(wc_commit_id) = self.get_wc_commit_id() {
            self.repo().store().get_commit(wc_commit_id)?
        } else {
            return Err(user_error("Nothing checked out in this workspace"));
        };

        let locked_ws = self.workspace.start_working_copy_mutation()?;

        Ok((locked_ws, wc_commit))
    }

    pub fn start_working_copy_mutation(
        &mut self,
    ) -> Result<(LockedWorkspace<'_>, Commit), CommandError> {
        let (mut locked_ws, wc_commit) = self.unchecked_start_working_copy_mutation()?;
        if wc_commit.tree().tree_ids_and_labels()
            != locked_ws.locked_wc().old_tree().tree_ids_and_labels()
        {
            return Err(user_error("Concurrent working copy operation. Try again."));
        }
        Ok((locked_ws, wc_commit))
    }

    fn create_and_check_out_recovery_commit(
        &mut self,
        ui: &Ui,
    ) -> Result<SnapshotStats, CommandError> {
        self.check_working_copy_writable()?;

        let workspace_name = self.workspace_name().to_owned();
        let mut locked_ws = self.workspace.start_working_copy_mutation()?;
        let (repo, new_commit) = working_copy::create_and_check_out_recovery_commit(
            locked_ws.locked_wc(),
            &self.user_repo.repo,
            workspace_name,
            "RECOVERY COMMIT FROM `jj workspace update-stale`

This commit contains changes that were written to the working copy by an
operation that was subsequently lost (or was at least unavailable when you ran
`jj workspace update-stale`). Because the operation was lost, we don't know
what the parent commits are supposed to be. That means that the diff compared
to the current parents may contain changes from multiple commits.
",
        )?;

        writeln!(
            ui.status(),
            "Created and checked out recovery commit {}",
            short_commit_hash(new_commit.id())
        )?;
        locked_ws.finish(repo.op_id().clone())?;
        self.user_repo = ReadonlyUserRepo::new(repo);

        self.maybe_snapshot_impl(ui)
            .map_err(|err| err.into_command_error())
    }

    pub fn workspace_root(&self) -> &Path {
        self.workspace.workspace_root()
    }

    pub fn workspace_name(&self) -> &WorkspaceName {
        self.workspace.workspace_name()
    }

    pub fn get_wc_commit_id(&self) -> Option<&CommitId> {
        self.repo().view().get_wc_commit_id(self.workspace_name())
    }

    pub fn working_copy_shared_with_git(&self) -> bool {
        self.working_copy_shared_with_git
    }

    pub fn format_file_path(&self, file: &RepoPath) -> String {
        self.path_converter().format_file_path(file)
    }

    /// Parses a path relative to cwd into a RepoPath, which is relative to the
    /// workspace root.
    pub fn parse_file_path(&self, input: &str) -> Result<RepoPathBuf, UiPathParseError> {
        self.path_converter().parse_file_path(input)
    }

    /// Parses the given strings as file patterns.
    pub fn parse_file_patterns(
        &self,
        ui: &Ui,
        values: &[String],
    ) -> Result<FilesetExpression, CommandError> {
        // TODO: This function might be superseded by parse_union_filesets(),
        // but it would be weird if parse_union_*() had a special case for the
        // empty arguments.
        if values.is_empty() {
            Ok(FilesetExpression::all())
        } else {
            self.parse_union_filesets(ui, values)
        }
    }

    /// Parses the given fileset expressions and concatenates them all.
    pub fn parse_union_filesets(
        &self,
        ui: &Ui,
        file_args: &[String], // TODO: introduce FileArg newtype?
    ) -> Result<FilesetExpression, CommandError> {
        let mut diagnostics = FilesetDiagnostics::new();
        let expressions: Vec<_> = file_args
            .iter()
            .map(|arg| fileset::parse_maybe_bare(&mut diagnostics, arg, self.path_converter()))
            .try_collect()?;
        print_parse_diagnostics(ui, "In fileset expression", &diagnostics)?;
        Ok(FilesetExpression::union_all(expressions))
    }

    pub fn auto_tracking_matcher(&self, ui: &Ui) -> Result<Box<dyn Matcher>, CommandError> {
        let mut diagnostics = FilesetDiagnostics::new();
        let pattern = self.settings().get_string("snapshot.auto-track")?;
        let expression = fileset::parse(
            &mut diagnostics,
            &pattern,
            &RepoPathUiConverter::Fs {
                cwd: "".into(),
                base: "".into(),
            },
        )?;
        print_parse_diagnostics(ui, "In `snapshot.auto-track`", &diagnostics)?;
        Ok(expression.to_matcher())
    }

    pub fn snapshot_options_with_start_tracking_matcher<'a>(
        &self,
        start_tracking_matcher: &'a dyn Matcher,
    ) -> Result<SnapshotOptions<'a>, CommandError> {
        let base_ignores = self.base_ignores()?;
        let HumanByteSize(mut max_new_file_size) = self
            .settings()
            .get_value_with("snapshot.max-new-file-size", TryInto::try_into)?;
        if max_new_file_size == 0 {
            max_new_file_size = u64::MAX;
        }
        Ok(SnapshotOptions {
            base_ignores,
            progress: None,
            start_tracking_matcher,
            force_tracking_matcher: &NothingMatcher,
            max_new_file_size,
        })
    }

    pub(crate) fn path_converter(&self) -> &RepoPathUiConverter {
        self.env.path_converter()
    }

    #[cfg(not(feature = "git"))]
    pub fn base_ignores(&self) -> Result<Arc<GitIgnoreFile>, GitIgnoreError> {
        Ok(GitIgnoreFile::empty())
    }

    #[cfg(feature = "git")]
    #[instrument(skip_all)]
    pub fn base_ignores(&self) -> Result<Arc<GitIgnoreFile>, GitIgnoreError> {
        let get_excludes_file_path = |config: &gix::config::File| -> Option<PathBuf> {
            // TODO: maybe use path() and interpolate(), which can process non-utf-8
            // path on Unix.
            if let Some(value) = config.string("core.excludesFile") {
                let path = str::from_utf8(&value)
                    .ok()
                    .map(jj_lib::file_util::expand_home_path)?;
                // The configured path is usually absolute, but if it's relative,
                // the "git" command would read the file at the work-tree directory.
                Some(self.workspace_root().join(path))
            } else {
                xdg_config_home().ok().map(|x| x.join("git").join("ignore"))
            }
        };

        fn xdg_config_home() -> Result<PathBuf, std::env::VarError> {
            if let Ok(x) = std::env::var("XDG_CONFIG_HOME")
                && !x.is_empty()
            {
                return Ok(PathBuf::from(x));
            }
            std::env::var("HOME").map(|x| Path::new(&x).join(".config"))
        }

        let mut git_ignores = GitIgnoreFile::empty();
        if let Ok(git_backend) = jj_lib::git::get_git_backend(self.repo().store()) {
            let git_repo = git_backend.git_repo();
            if let Some(excludes_file_path) = get_excludes_file_path(&git_repo.config_snapshot()) {
                git_ignores = git_ignores.chain_with_file("", excludes_file_path)?;
            }
            git_ignores = git_ignores
                .chain_with_file("", git_backend.git_repo_path().join("info").join("exclude"))?;
        } else if let Ok(git_config) = gix::config::File::from_globals()
            && let Some(excludes_file_path) = get_excludes_file_path(&git_config)
        {
            git_ignores = git_ignores.chain_with_file("", excludes_file_path)?;
        }
        Ok(git_ignores)
    }

    /// Creates textual diff renderer of the specified `formats`.
    pub fn diff_renderer(&self, formats: Vec<DiffFormat>) -> DiffRenderer<'_> {
        DiffRenderer::new(
            self.repo().as_ref(),
            self.path_converter(),
            self.env.conflict_marker_style(),
            formats,
        )
    }

    /// Loads textual diff renderer from the settings and command arguments.
    pub fn diff_renderer_for(
        &self,
        args: &DiffFormatArgs,
    ) -> Result<DiffRenderer<'_>, CommandError> {
        let formats = diff_util::diff_formats_for(self.settings(), args)?;
        Ok(self.diff_renderer(formats))
    }

    /// Loads textual diff renderer from the settings and log-like command
    /// arguments. Returns `Ok(None)` if there are no command arguments that
    /// enable patch output.
    pub fn diff_renderer_for_log(
        &self,
        args: &DiffFormatArgs,
        patch: bool,
    ) -> Result<Option<DiffRenderer<'_>>, CommandError> {
        let formats = diff_util::diff_formats_for_log(self.settings(), args, patch)?;
        Ok((!formats.is_empty()).then(|| self.diff_renderer(formats)))
    }

    /// Loads diff editor from the settings.
    ///
    /// If the `tool_name` isn't specified, the default editor will be returned.
    pub fn diff_editor(
        &self,
        ui: &Ui,
        tool_name: Option<&str>,
    ) -> Result<DiffEditor, CommandError> {
        let base_ignores = self.base_ignores()?;
        let conflict_marker_style = self.env.conflict_marker_style();
        if let Some(name) = tool_name {
            Ok(DiffEditor::with_name(
                name,
                self.settings(),
                base_ignores,
                conflict_marker_style,
            )?)
        } else {
            Ok(DiffEditor::from_settings(
                ui,
                self.settings(),
                base_ignores,
                conflict_marker_style,
            )?)
        }
    }

    /// Conditionally loads diff editor from the settings.
    ///
    /// If the `tool_name` is specified, interactive session is implied.
    pub fn diff_selector(
        &self,
        ui: &Ui,
        tool_name: Option<&str>,
        force_interactive: bool,
    ) -> Result<DiffSelector, CommandError> {
        if tool_name.is_some() || force_interactive {
            Ok(DiffSelector::Interactive(self.diff_editor(ui, tool_name)?))
        } else {
            Ok(DiffSelector::NonInteractive)
        }
    }

    /// Loads 3-way merge editor from the settings.
    ///
    /// If the `tool_name` isn't specified, the default editor will be returned.
    pub fn merge_editor(
        &self,
        ui: &Ui,
        tool_name: Option<&str>,
    ) -> Result<MergeEditor, MergeToolConfigError> {
        let conflict_marker_style = self.env.conflict_marker_style();
        if let Some(name) = tool_name {
            MergeEditor::with_name(
                name,
                self.settings(),
                self.path_converter().clone(),
                conflict_marker_style,
            )
        } else {
            MergeEditor::from_settings(
                ui,
                self.settings(),
                self.path_converter().clone(),
                conflict_marker_style,
            )
        }
    }

    /// Loads text editor from the settings.
    pub fn text_editor(&self) -> Result<TextEditor, ConfigGetError> {
        TextEditor::from_settings(self.settings())
    }

    pub fn resolve_single_op(&self, op_str: &str) -> Result<Operation, OpsetEvaluationError> {
        op_walk::resolve_op_with_repo(self.repo(), op_str)
    }

    /// Resolve a revset to a single revision. Return an error if the revset is
    /// empty or has multiple revisions.
    pub fn resolve_single_rev(
        &self,
        ui: &Ui,
        revision_arg: &RevisionArg,
    ) -> Result<Commit, CommandError> {
        let expression = self.parse_revset(ui, revision_arg)?;
        revset_util::evaluate_revset_to_single_commit(revision_arg.as_ref(), &expression, || {
            self.commit_summary_template()
        })
    }

    /// Evaluates revset expressions to non-empty set of commit IDs. The
    /// returned set preserves the order of the input expressions.
    pub fn resolve_some_revsets_default_single(
        &self,
        ui: &Ui,
        revision_args: &[RevisionArg],
    ) -> Result<IndexSet<CommitId>, CommandError> {
        let mut all_commits = IndexSet::new();
        for revision_arg in revision_args {
            let (expression, modifier) = self.parse_revset_with_modifier(ui, revision_arg)?;
            let all = match modifier {
                Some(RevsetModifier::All) => true,
                None => self.settings().get_bool("ui.always-allow-large-revsets")?,
            };
            if all {
                for commit_id in expression.evaluate_to_commit_ids()? {
                    all_commits.insert(commit_id?);
                }
            } else {
                let commit = revset_util::evaluate_revset_to_single_commit(
                    revision_arg.as_ref(),
                    &expression,
                    || self.commit_summary_template(),
                )?;
                if !all_commits.insert(commit.id().clone()) {
                    let commit_hash = short_commit_hash(commit.id());
                    return Err(user_error(format!(
                        r#"More than one revset resolved to revision {commit_hash}"#,
                    )));
                }
            }
        }
        if all_commits.is_empty() {
            Err(user_error("Empty revision set"))
        } else {
            Ok(all_commits)
        }
    }

    pub fn parse_revset(
        &self,
        ui: &Ui,
        revision_arg: &RevisionArg,
    ) -> Result<RevsetExpressionEvaluator<'_>, CommandError> {
        let (expression, modifier) = self.parse_revset_with_modifier(ui, revision_arg)?;
        // Whether the caller accepts multiple revisions or not, "all:" should
        // be valid. For example, "all:@" is a valid single-rev expression.
        let (None | Some(RevsetModifier::All)) = modifier;
        Ok(expression)
    }

    fn parse_revset_with_modifier(
        &self,
        ui: &Ui,
        revision_arg: &RevisionArg,
    ) -> Result<(RevsetExpressionEvaluator<'_>, Option<RevsetModifier>), CommandError> {
        let mut diagnostics = RevsetDiagnostics::new();
        let context = self.env.revset_parse_context();
        let (expression, modifier) =
            revset::parse_with_modifier(&mut diagnostics, revision_arg.as_ref(), &context)?;
        print_parse_diagnostics(ui, "In revset expression", &diagnostics)?;
        Ok((self.attach_revset_evaluator(expression), modifier))
    }

    /// Parses the given revset expressions and concatenates them all.
    pub fn parse_union_revsets(
        &self,
        ui: &Ui,
        revision_args: &[RevisionArg],
    ) -> Result<RevsetExpressionEvaluator<'_>, CommandError> {
        let mut diagnostics = RevsetDiagnostics::new();
        let context = self.env.revset_parse_context();
        let expressions: Vec<_> = revision_args
            .iter()
            .map(|arg| revset::parse_with_modifier(&mut diagnostics, arg.as_ref(), &context))
            .map_ok(|(expression, None | Some(RevsetModifier::All))| expression)
            .try_collect()?;
        print_parse_diagnostics(ui, "In revset expression", &diagnostics)?;
        let expression = RevsetExpression::union_all(&expressions);
        Ok(self.attach_revset_evaluator(expression))
    }

    pub fn attach_revset_evaluator(
        &self,
        expression: Arc<UserRevsetExpression>,
    ) -> RevsetExpressionEvaluator<'_> {
        RevsetExpressionEvaluator::new(
            self.repo().as_ref(),
            self.env.command.revset_extensions().clone(),
            self.id_prefix_context(),
            expression,
        )
    }

    pub fn id_prefix_context(&self) -> &IdPrefixContext {
        self.user_repo
            .id_prefix_context
            .get_or_init(|| self.env.new_id_prefix_context())
    }

    /// Parses template of the given language into evaluation tree.
    pub fn parse_template<'a, C, L>(
        &self,
        ui: &Ui,
        language: &L,
        template_text: &str,
    ) -> Result<TemplateRenderer<'a, C>, CommandError>
    where
        C: Clone + 'a,
        L: TemplateLanguage<'a> + ?Sized,
        L::Property: WrapTemplateProperty<'a, C>,
    {
        self.env.parse_template(ui, language, template_text)
    }

    /// Parses template that is validated by `Self::new()`.
    fn reparse_valid_template<'a, C, L>(
        &self,
        language: &L,
        template_text: &str,
    ) -> TemplateRenderer<'a, C>
    where
        C: Clone + 'a,
        L: TemplateLanguage<'a> + ?Sized,
        L::Property: WrapTemplateProperty<'a, C>,
    {
        template_builder::parse(
            language,
            &mut TemplateDiagnostics::new(),
            template_text,
            &self.env.template_aliases_map,
        )
        .expect("parse error should be confined by WorkspaceCommandHelper::new()")
    }

    /// Parses commit template into evaluation tree.
    pub fn parse_commit_template(
        &self,
        ui: &Ui,
        template_text: &str,
    ) -> Result<TemplateRenderer<'_, Commit>, CommandError> {
        let language = self.commit_template_language();
        self.parse_template(ui, &language, template_text)
    }

    /// Parses commit template into evaluation tree.
    pub fn parse_operation_template(
        &self,
        ui: &Ui,
        template_text: &str,
    ) -> Result<TemplateRenderer<'_, Operation>, CommandError> {
        let language = self.operation_template_language();
        self.parse_template(ui, &language, template_text)
    }

    /// Creates commit template language environment for this workspace.
    pub fn commit_template_language(&self) -> CommitTemplateLanguage<'_> {
        self.env
            .commit_template_language(self.repo().as_ref(), self.id_prefix_context())
    }

    /// Creates operation template language environment for this workspace.
    pub fn operation_template_language(&self) -> OperationTemplateLanguage {
        OperationTemplateLanguage::new(
            self.workspace.repo_loader(),
            Some(self.repo().op_id()),
            self.env.operation_template_extensions(),
        )
    }

    /// Template for one-line summary of a commit.
    pub fn commit_summary_template(&self) -> TemplateRenderer<'_, Commit> {
        let language = self.commit_template_language();
        self.reparse_valid_template(&language, &self.commit_summary_template_text)
            .labeled(["commit"])
    }

    /// Template for one-line summary of an operation.
    pub fn operation_summary_template(&self) -> TemplateRenderer<'_, Operation> {
        let language = self.operation_template_language();
        self.reparse_valid_template(&language, &self.op_summary_template_text)
            .labeled(["operation"])
    }

    pub fn short_change_id_template(&self) -> TemplateRenderer<'_, Commit> {
        let language = self.commit_template_language();
        self.reparse_valid_template(&language, SHORT_CHANGE_ID_TEMPLATE_TEXT)
            .labeled(["commit"])
    }

    /// Returns one-line summary of the given `commit`.
    ///
    /// Use `write_commit_summary()` to get colorized output. Use
    /// `commit_summary_template()` if you have many commits to process.
    pub fn format_commit_summary(&self, commit: &Commit) -> String {
        let output = self.commit_summary_template().format_plain_text(commit);
        output.into_string_lossy()
    }

    /// Writes one-line summary of the given `commit`.
    ///
    /// Use `commit_summary_template()` if you have many commits to process.
    #[instrument(skip_all)]
    pub fn write_commit_summary(
        &self,
        formatter: &mut dyn Formatter,
        commit: &Commit,
    ) -> std::io::Result<()> {
        self.commit_summary_template().format(commit, formatter)
    }

    pub fn check_rewritable<'a>(
        &self,
        commits: impl IntoIterator<Item = &'a CommitId>,
    ) -> Result<(), CommandError> {
        let commit_ids = commits.into_iter().cloned().collect_vec();
        let to_rewrite_expr = RevsetExpression::commits(commit_ids);
        self.check_rewritable_expr(&to_rewrite_expr)
    }

    pub fn check_rewritable_expr(
        &self,
        to_rewrite_expr: &Arc<ResolvedRevsetExpression>,
    ) -> Result<(), CommandError> {
        let repo = self.repo().as_ref();
        let Some(commit_id) = self.env.find_immutable_commit(repo, to_rewrite_expr)? else {
            return Ok(());
        };
        let error = if &commit_id == repo.store().root_commit_id() {
            user_error(format!("The root commit {commit_id:.12} is immutable"))
        } else {
            let mut error = user_error(format!("Commit {commit_id:.12} is immutable"));
            let commit = repo.store().get_commit(&commit_id)?;
            error.add_formatted_hint_with(|formatter| {
                write!(formatter, "Could not modify commit: ")?;
                self.write_commit_summary(formatter, &commit)?;
                Ok(())
            });
            error.add_hint("Immutable commits are used to protect shared history.");
            error.add_hint(indoc::indoc! {"
                For more information, see:
                      - https://docs.jj-vcs.dev/latest/config/#set-of-immutable-commits
                      - `jj help -k config`, \"Set of immutable commits\""});

            // Not using self.id_prefix_context() for consistency with
            // find_immutable_commit().
            let id_prefix_context =
                IdPrefixContext::new(self.env.command.revset_extensions().clone());
            let (lower_bound, upper_bound) = RevsetExpressionEvaluator::new(
                repo,
                self.env.command.revset_extensions().clone(),
                &id_prefix_context,
                self.env.immutable_expression(),
            )
            .resolve()?
            .intersection(&to_rewrite_expr.descendants())
            .evaluate(repo)?
            .count_estimate()?;
            let exact = upper_bound == Some(lower_bound);
            let or_more = if exact { "" } else { " or more" };
            error.add_hint(format!(
                "This operation would rewrite {lower_bound}{or_more} immutable commits."
            ));

            error
        };
        Err(error)
    }

    #[instrument(skip_all)]
    fn snapshot_working_copy(
        &mut self,
        ui: &Ui,
    ) -> Result<SnapshotStats, SnapshotWorkingCopyError> {
        let workspace_name = self.workspace_name().to_owned();
        let repo = self.repo().clone();
        let auto_tracking_matcher = self
            .auto_tracking_matcher(ui)
            .map_err(snapshot_command_error)?;
        let options = self
            .snapshot_options_with_start_tracking_matcher(&auto_tracking_matcher)
            .map_err(snapshot_command_error)?;

        // Compare working-copy tree and operation with repo's, and reload as needed.
        let mut locked_ws = self
            .workspace
            .start_working_copy_mutation()
            .map_err(snapshot_command_error)?;

        let Some((repo, wc_commit)) =
            handle_stale_working_copy(locked_ws.locked_wc(), repo, &workspace_name)?
        else {
            // If the workspace has been deleted, it's unclear what to do, so we just skip
            // committing the working copy.
            return Ok(SnapshotStats::default());
        };

        self.user_repo = ReadonlyUserRepo::new(repo);
        let (new_tree, stats) = {
            let mut options = options;
            let progress = crate::progress::snapshot_progress(ui);
            options.progress = progress.as_ref().map(|x| x as _);
            locked_ws
                .locked_wc()
                .snapshot(&options)
                .block_on()
                .map_err(snapshot_command_error)?
        };
        if new_tree.tree_ids_and_labels() != wc_commit.tree().tree_ids_and_labels() {
            let mut tx =
                start_repo_transaction(&self.user_repo.repo, self.env.command.string_args());
            tx.set_is_snapshot(true);
            let mut_repo = tx.repo_mut();
            let commit = mut_repo
                .rewrite_commit(&wc_commit)
                .set_tree(new_tree)
                .write()
                .map_err(snapshot_command_error)?;
            mut_repo
                .set_wc_commit(workspace_name, commit.id().clone())
                .map_err(snapshot_command_error)?;

            // Rebase descendants
            let num_rebased = mut_repo
                .rebase_descendants()
                .map_err(snapshot_command_error)?;
            if num_rebased > 0 {
                writeln!(
                    ui.status(),
                    "Rebased {num_rebased} descendant commits onto updated working copy"
                )
                .map_err(snapshot_command_error)?;
            }

            #[cfg(feature = "git")]
            if self.working_copy_shared_with_git {
                let old_tree = wc_commit.tree();
                let new_tree = commit.tree();
                export_working_copy_changes_to_git(ui, mut_repo, &old_tree, &new_tree)
                    .map_err(snapshot_command_error)?;
            }

            let repo = tx
                .commit("snapshot working copy")
                .map_err(snapshot_command_error)?;
            self.user_repo = ReadonlyUserRepo::new(repo);
        }
        locked_ws
            .finish(self.user_repo.repo.op_id().clone())
            .map_err(snapshot_command_error)?;
        Ok(stats)
    }

    fn update_working_copy(
        &mut self,
        ui: &Ui,
        maybe_old_commit: Option<&Commit>,
        new_commit: &Commit,
    ) -> Result<(), CommandError> {
        assert!(self.may_update_working_copy);
        let stats = update_working_copy(
            &self.user_repo.repo,
            &mut self.workspace,
            maybe_old_commit,
            new_commit,
        )?;
        self.print_updated_working_copy_stats(ui, maybe_old_commit, new_commit, &stats)
    }

    fn print_updated_working_copy_stats(
        &self,
        ui: &Ui,
        maybe_old_commit: Option<&Commit>,
        new_commit: &Commit,
        stats: &CheckoutStats,
    ) -> Result<(), CommandError> {
        if Some(new_commit) != maybe_old_commit
            && let Some(mut formatter) = ui.status_formatter()
        {
            let template = self.commit_summary_template();
            write!(formatter, "Working copy  (@) now at: ")?;
            template.format(new_commit, formatter.as_mut())?;
            writeln!(formatter)?;
            for parent in new_commit.parents() {
                let parent = parent?;
                //                "Working copy  (@) now at: "
                write!(formatter, "Parent commit (@-)      : ")?;
                template.format(&parent, formatter.as_mut())?;
                writeln!(formatter)?;
            }
        }
        print_checkout_stats(ui, stats, new_commit)?;
        if Some(new_commit) != maybe_old_commit
            && let Some(mut formatter) = ui.status_formatter()
            && new_commit.has_conflict()
        {
            let conflicts = new_commit.tree().conflicts().collect_vec();
            writeln!(
                formatter.labeled("warning").with_heading("Warning: "),
                "There are unresolved conflicts at these paths:"
            )?;
            print_conflicted_paths(conflicts, formatter.as_mut(), self)?;
        }
        Ok(())
    }

    pub fn start_transaction(&mut self) -> WorkspaceCommandTransaction<'_> {
        let tx = start_repo_transaction(self.repo(), self.env.command.string_args());
        let id_prefix_context = mem::take(&mut self.user_repo.id_prefix_context);
        WorkspaceCommandTransaction {
            helper: self,
            tx,
            id_prefix_context,
        }
    }

    fn finish_transaction(
        &mut self,
        ui: &Ui,
        mut tx: Transaction,
        description: impl Into<String>,
        _git_import_export_lock: &GitImportExportLock,
    ) -> Result<(), CommandError> {
        let num_rebased = tx.repo_mut().rebase_descendants()?;
        if num_rebased > 0 {
            writeln!(ui.status(), "Rebased {num_rebased} descendant commits")?;
        }

        for (name, wc_commit_id) in &tx.repo().view().wc_commit_ids().clone() {
            if self
                .env
                .find_immutable_commit(tx.repo(), &RevsetExpression::commit(wc_commit_id.clone()))?
                .is_some()
            {
                let wc_commit = tx.repo().store().get_commit(wc_commit_id)?;
                tx.repo_mut().check_out(name.clone(), &wc_commit)?;
                writeln!(
                    ui.warning_default(),
                    "The working-copy commit in workspace '{name}' became immutable, so a new \
                     commit has been created on top of it.",
                    name = name.as_symbol()
                )?;
            }
        }

        let old_repo = tx.base_repo().clone();

        let maybe_old_wc_commit = old_repo
            .view()
            .get_wc_commit_id(self.workspace_name())
            .map(|commit_id| tx.base_repo().store().get_commit(commit_id))
            .transpose()?;
        let maybe_new_wc_commit = tx
            .repo()
            .view()
            .get_wc_commit_id(self.workspace_name())
            .map(|commit_id| tx.repo().store().get_commit(commit_id))
            .transpose()?;

        #[cfg(feature = "git")]
        if self.working_copy_shared_with_git {
            use std::error::Error as _;
            if let Some(wc_commit) = &maybe_new_wc_commit {
                // Export Git HEAD while holding the git-head lock to prevent races:
                // - Between two finish_transaction calls updating HEAD
                // - With import_git_head importing HEAD concurrently
                // This can still fail if HEAD was updated concurrently by another JJ process
                // (overlapping transaction) or a non-JJ process (e.g., git checkout). In that
                // case, the actual state will be imported on the next snapshot.
                match jj_lib::git::reset_head(tx.repo_mut(), wc_commit) {
                    Ok(()) => {}
                    Err(err @ jj_lib::git::GitResetHeadError::UpdateHeadRef(_)) => {
                        writeln!(ui.warning_default(), "{err}")?;
                        crate::command_error::print_error_sources(ui, err.source())?;
                    }
                    Err(err) => return Err(err.into()),
                }
            }
            let stats = jj_lib::git::export_refs(tx.repo_mut())?;
            crate::git_util::print_git_export_stats(ui, &stats)?;
        }

        self.user_repo = ReadonlyUserRepo::new(tx.commit(description)?);

        // Update working copy before reporting repo changes, so that
        // potential errors while reporting changes (broken pipe, etc)
        // don't leave the working copy in a stale state.
        if self.may_update_working_copy {
            if let Some(new_commit) = &maybe_new_wc_commit {
                self.update_working_copy(ui, maybe_old_wc_commit.as_ref(), new_commit)?;
            } else {
                // It seems the workspace was deleted, so we shouldn't try to
                // update it.
            }
        }

        self.report_repo_changes(ui, &old_repo)?;

        let settings = self.settings();
        let missing_user_name = settings.user_name().is_empty();
        let missing_user_mail = settings.user_email().is_empty();
        if missing_user_name || missing_user_mail {
            let not_configured_msg = match (missing_user_name, missing_user_mail) {
                (true, true) => "Name and email not configured.",
                (true, false) => "Name not configured.",
                (false, true) => "Email not configured.",
                _ => unreachable!(),
            };
            writeln!(
                ui.warning_default(),
                "{not_configured_msg} Until configured, your commits will be created with the \
                 empty identity, and can't be pushed to remotes."
            )?;
            writeln!(ui.hint_default(), "To configure, run:")?;
            if missing_user_name {
                writeln!(
                    ui.hint_no_heading(),
                    r#"  jj config set --user user.name "Some One""#
                )?;
            }
            if missing_user_mail {
                writeln!(
                    ui.hint_no_heading(),
                    r#"  jj config set --user user.email "someone@example.com""#
                )?;
            }
        }
        Ok(())
    }

    /// Inform the user about important changes to the repo since the previous
    /// operation (when `old_repo` was loaded).
    fn report_repo_changes(
        &self,
        ui: &Ui,
        old_repo: &Arc<ReadonlyRepo>,
    ) -> Result<(), CommandError> {
        let Some(mut fmt) = ui.status_formatter() else {
            return Ok(());
        };
        let old_view = old_repo.view();
        let new_repo = self.repo().as_ref();
        let new_view = new_repo.view();
        let old_heads = RevsetExpression::commits(old_view.heads().iter().cloned().collect());
        let new_heads = RevsetExpression::commits(new_view.heads().iter().cloned().collect());
        // Filter the revsets by conflicts instead of reading all commits and doing the
        // filtering here. That way, we can afford to evaluate the revset even if there
        // are millions of commits added to the repo, assuming the revset engine can
        // efficiently skip non-conflicting commits. Filter out empty commits mostly so
        // `jj new <conflicted commit>` doesn't result in a message about new conflicts.
        let conflicts = RevsetExpression::filter(RevsetFilterPredicate::HasConflict)
            .filtered(RevsetFilterPredicate::File(FilesetExpression::all()));
        let removed_conflicts_expr = new_heads.range(&old_heads).intersection(&conflicts);
        let added_conflicts_expr = old_heads.range(&new_heads).intersection(&conflicts);

        let get_commits =
            |expr: Arc<ResolvedRevsetExpression>| -> Result<Vec<Commit>, CommandError> {
                let commits = expr
                    .evaluate(new_repo)?
                    .iter()
                    .commits(new_repo.store())
                    .try_collect()?;
                Ok(commits)
            };
        let removed_conflict_commits = get_commits(removed_conflicts_expr)?;
        let added_conflict_commits = get_commits(added_conflicts_expr)?;

        fn commits_by_change_id(commits: &[Commit]) -> IndexMap<&ChangeId, Vec<&Commit>> {
            let mut result: IndexMap<&ChangeId, Vec<&Commit>> = IndexMap::new();
            for commit in commits {
                result.entry(commit.change_id()).or_default().push(commit);
            }
            result
        }
        let removed_conflicts_by_change_id = commits_by_change_id(&removed_conflict_commits);
        let added_conflicts_by_change_id = commits_by_change_id(&added_conflict_commits);
        let mut resolved_conflicts_by_change_id = removed_conflicts_by_change_id.clone();
        resolved_conflicts_by_change_id
            .retain(|change_id, _commits| !added_conflicts_by_change_id.contains_key(change_id));
        let mut new_conflicts_by_change_id = added_conflicts_by_change_id.clone();
        new_conflicts_by_change_id
            .retain(|change_id, _commits| !removed_conflicts_by_change_id.contains_key(change_id));

        // TODO: Also report new divergence and maybe resolved divergence
        if !resolved_conflicts_by_change_id.is_empty() {
            // TODO: Report resolved and abandoned numbers separately. However,
            // that involves resolving the change_id among the visible commits in the new
            // repo, which isn't currently supported by Google's revset engine.
            let num_resolved: usize = resolved_conflicts_by_change_id
                .values()
                .map(|commits| commits.len())
                .sum();
            writeln!(
                fmt,
                "Existing conflicts were resolved or abandoned from {num_resolved} commits."
            )?;
        }
        if !new_conflicts_by_change_id.is_empty() {
            let num_conflicted: usize = new_conflicts_by_change_id
                .values()
                .map(|commits| commits.len())
                .sum();
            writeln!(fmt, "New conflicts appeared in {num_conflicted} commits:")?;
            print_updated_commits(
                fmt.as_mut(),
                &self.commit_summary_template(),
                new_conflicts_by_change_id.values().flatten().copied(),
            )?;
        }

        // Hint that the user might want to `jj new` to the first conflict commit to
        // resolve conflicts. Only show the hints if there were any new or resolved
        // conflicts, and only if there are still some conflicts.
        if !(added_conflict_commits.is_empty()
            || resolved_conflicts_by_change_id.is_empty() && new_conflicts_by_change_id.is_empty())
        {
            // If the user just resolved some conflict and squashed them in, there won't be
            // any new conflicts. Clarify to them that there are still some other conflicts
            // to resolve. (We don't mention conflicts in commits that weren't affected by
            // the operation, however.)
            if new_conflicts_by_change_id.is_empty() {
                writeln!(
                    fmt,
                    "There are still unresolved conflicts in rebased descendants.",
                )?;
            }

            self.report_repo_conflicts(
                fmt.as_mut(),
                new_repo,
                added_conflict_commits
                    .iter()
                    .map(|commit| commit.id().clone())
                    .collect(),
            )?;
        }
        revset_util::warn_unresolvable_trunk(ui, new_repo, &self.env.revset_parse_context())?;

        Ok(())
    }

    pub fn report_repo_conflicts(
        &self,
        fmt: &mut dyn Formatter,
        repo: &ReadonlyRepo,
        conflicted_commits: Vec<CommitId>,
    ) -> Result<(), CommandError> {
        if !self.settings().get_bool("hints.resolving-conflicts")? || conflicted_commits.is_empty()
        {
            return Ok(());
        }

        let only_one_conflicted_commit = conflicted_commits.len() == 1;
        let root_conflicts_revset = RevsetExpression::commits(conflicted_commits)
            .roots()
            .evaluate(repo)?;

        let root_conflict_commits: Vec<_> = root_conflicts_revset
            .iter()
            .commits(repo.store())
            .try_collect()?;

        // The common part of these strings is not extracted, to avoid i18n issues.
        let instruction = if only_one_conflicted_commit {
            indoc! {"
            To resolve the conflicts, start by creating a commit on top of
            the conflicted commit:
            "}
        } else if root_conflict_commits.len() == 1 {
            indoc! {"
            To resolve the conflicts, start by creating a commit on top of
            the first conflicted commit:
            "}
        } else {
            indoc! {"
            To resolve the conflicts, start by creating a commit on top of
            one of the first conflicted commits:
            "}
        };
        write!(fmt.labeled("hint").with_heading("Hint: "), "{instruction}")?;
        let format_short_change_id = self.short_change_id_template();
        {
            let mut fmt = fmt.labeled("hint");
            for commit in &root_conflict_commits {
                write!(fmt, "  jj new ")?;
                format_short_change_id.format(commit, *fmt)?;
                writeln!(fmt)?;
            }
        }
        writedoc!(
            fmt.labeled("hint"),
            "
            Then use `jj resolve`, or edit the conflict markers in the file directly.
            Once the conflicts are resolved, you can inspect the result with `jj diff`.
            Then run `jj squash` to move the resolution into the conflicted commit.
            ",
        )?;
        Ok(())
    }

    /// Identifies bookmarks which are eligible to be moved automatically
    /// during `jj commit` and `jj new`. Whether a bookmark is eligible is
    /// determined by its target and the user and repo config for
    /// "advance-bookmarks".
    ///
    /// Returns a Vec of bookmarks in `repo` that point to any of the `from`
    /// commits and that are eligible to advance. The `from` commits are
    /// typically the parents of the target commit of `jj commit` or `jj new`.
    ///
    /// Bookmarks are not moved until
    /// `WorkspaceCommandTransaction::advance_bookmarks()` is called with the
    /// `AdvanceableBookmark`s returned by this function.
    ///
    /// Returns an empty `std::Vec` if no bookmarks are eligible to advance.
    pub fn get_advanceable_bookmarks<'a>(
        &self,
        ui: &Ui,
        from: impl IntoIterator<Item = &'a CommitId>,
    ) -> Result<Vec<AdvanceableBookmark>, CommandError> {
        let Some(ab_matcher) = load_advance_bookmarks_matcher(ui, self.settings())? else {
            // Return early if we know that there's no work to do.
            return Ok(Vec::new());
        };

        let mut advanceable_bookmarks = Vec::new();
        for from_commit in from {
            for (name, _) in self.repo().view().local_bookmarks_for_commit(from_commit) {
                if ab_matcher.is_match(name.as_str()) {
                    advanceable_bookmarks.push(AdvanceableBookmark {
                        name: name.to_owned(),
                        old_commit_id: from_commit.clone(),
                    });
                }
            }
        }

        Ok(advanceable_bookmarks)
    }
}

#[cfg(feature = "git")]
pub fn export_working_copy_changes_to_git(
    ui: &Ui,
    mut_repo: &mut MutableRepo,
    old_tree: &MergedTree,
    new_tree: &MergedTree,
) -> Result<(), CommandError> {
    let repo = mut_repo.base_repo().as_ref();
    jj_lib::git::update_intent_to_add(repo, old_tree, new_tree)?;
    let stats = jj_lib::git::export_refs(mut_repo)?;
    crate::git_util::print_git_export_stats(ui, &stats)?;
    Ok(())
}
#[cfg(not(feature = "git"))]
pub fn export_working_copy_changes_to_git(
    _ui: &Ui,
    _mut_repo: &mut MutableRepo,
    _old_tree: &MergedTree,
    _new_tree: &MergedTree,
) -> Result<(), CommandError> {
    Ok(())
}

/// An ongoing [`Transaction`] tied to a particular workspace.
///
/// `WorkspaceCommandTransaction`s are created with
/// [`WorkspaceCommandHelper::start_transaction`] and committed with
/// [`WorkspaceCommandTransaction::finish`]. The inner `Transaction` can also be
/// extracted using [`WorkspaceCommandTransaction::into_inner`] in situations
/// where finer-grained control over the `Transaction` is necessary.
#[must_use]
pub struct WorkspaceCommandTransaction<'a> {
    helper: &'a mut WorkspaceCommandHelper,
    tx: Transaction,
    /// Cache of index built against the current MutableRepo state.
    id_prefix_context: OnceCell<IdPrefixContext>,
}

impl WorkspaceCommandTransaction<'_> {
    /// Workspace helper that may use the base repo.
    pub fn base_workspace_helper(&self) -> &WorkspaceCommandHelper {
        self.helper
    }

    /// Settings for this workspace.
    pub fn settings(&self) -> &UserSettings {
        self.helper.settings()
    }

    pub fn base_repo(&self) -> &Arc<ReadonlyRepo> {
        self.tx.base_repo()
    }

    pub fn repo(&self) -> &MutableRepo {
        self.tx.repo()
    }

    pub fn repo_mut(&mut self) -> &mut MutableRepo {
        self.id_prefix_context.take(); // invalidate
        self.tx.repo_mut()
    }

    pub fn check_out(&mut self, commit: &Commit) -> Result<Commit, CheckOutCommitError> {
        let name = self.helper.workspace_name().to_owned();
        self.id_prefix_context.take(); // invalidate
        self.tx.repo_mut().check_out(name, commit)
    }

    pub fn edit(&mut self, commit: &Commit) -> Result<(), EditCommitError> {
        let name = self.helper.workspace_name().to_owned();
        self.id_prefix_context.take(); // invalidate
        self.tx.repo_mut().edit(name, commit)
    }

    pub fn format_commit_summary(&self, commit: &Commit) -> String {
        let output = self.commit_summary_template().format_plain_text(commit);
        output.into_string_lossy()
    }

    pub fn write_commit_summary(
        &self,
        formatter: &mut dyn Formatter,
        commit: &Commit,
    ) -> std::io::Result<()> {
        self.commit_summary_template().format(commit, formatter)
    }

    /// Template for one-line summary of a commit within transaction.
    pub fn commit_summary_template(&self) -> TemplateRenderer<'_, Commit> {
        let language = self.commit_template_language();
        self.helper
            .reparse_valid_template(&language, &self.helper.commit_summary_template_text)
            .labeled(["commit"])
    }

    /// Creates commit template language environment capturing the current
    /// transaction state.
    pub fn commit_template_language(&self) -> CommitTemplateLanguage<'_> {
        let id_prefix_context = self
            .id_prefix_context
            .get_or_init(|| self.helper.env.new_id_prefix_context());
        self.helper
            .env
            .commit_template_language(self.tx.repo(), id_prefix_context)
    }

    /// Parses commit template with the current transaction state.
    pub fn parse_commit_template(
        &self,
        ui: &Ui,
        template_text: &str,
    ) -> Result<TemplateRenderer<'_, Commit>, CommandError> {
        let language = self.commit_template_language();
        self.helper.env.parse_template(ui, &language, template_text)
    }

    pub fn finish(self, ui: &Ui, description: impl Into<String>) -> Result<(), CommandError> {
        if !self.tx.repo().has_changes() {
            writeln!(ui.status(), "Nothing changed.")?;
            return Ok(());
        }
        // Acquire git import/export lock before finishing the transaction to ensure
        // Git HEAD export happens atomically with the transaction commit.
        let git_import_export_lock = self.helper.lock_git_import_export()?;
        self.helper
            .finish_transaction(ui, self.tx, description, &git_import_export_lock)
    }

    /// Returns the wrapped [`Transaction`] for circumstances where
    /// finer-grained control is needed. The caller becomes responsible for
    /// finishing the `Transaction`, including rebasing descendants and updating
    /// the working copy, if applicable.
    pub fn into_inner(self) -> Transaction {
        self.tx
    }

    /// Moves each bookmark in `bookmarks` from an old commit it's associated
    /// with (configured by `get_advanceable_bookmarks`) to the `move_to`
    /// commit. If the bookmark is conflicted before the update, it will
    /// remain conflicted after the update, but the conflict will involve
    /// the `move_to` commit instead of the old commit.
    pub fn advance_bookmarks(
        &mut self,
        bookmarks: Vec<AdvanceableBookmark>,
        move_to: &CommitId,
    ) -> Result<(), CommandError> {
        for bookmark in bookmarks {
            // This removes the old commit ID from the bookmark's RefTarget and
            // replaces it with the `move_to` ID.
            self.repo_mut().merge_local_bookmark(
                &bookmark.name,
                &RefTarget::normal(bookmark.old_commit_id),
                &RefTarget::normal(move_to.clone()),
            )?;
        }
        Ok(())
    }
}

pub fn find_workspace_dir(cwd: &Path) -> &Path {
    cwd.ancestors()
        .find(|path| path.join(".jj").is_dir())
        .unwrap_or(cwd)
}

fn map_workspace_load_error(err: WorkspaceLoadError, user_wc_path: Option<&str>) -> CommandError {
    match err {
        WorkspaceLoadError::NoWorkspaceHere(wc_path) => {
            // Prefer user-specified path instead of absolute wc_path if any.
            let short_wc_path = user_wc_path.map_or(wc_path.as_ref(), Path::new);
            let message = format!(r#"There is no jj repo in "{}""#, short_wc_path.display());
            let git_dir = wc_path.join(".git");
            if git_dir.is_dir() {
                user_error_with_hint(
                    message,
                    "It looks like this is a git repo. You can create a jj repo backed by it by \
                     running this:
jj git init",
                )
            } else {
                user_error(message)
            }
        }
        WorkspaceLoadError::RepoDoesNotExist(repo_dir) => user_error(format!(
            "The repository directory at {} is missing. Was it moved?",
            repo_dir.display(),
        )),
        WorkspaceLoadError::StoreLoadError(err @ StoreLoadError::UnsupportedType { .. }) => {
            internal_error_with_message(
                "This version of the jj binary doesn't support this type of repo",
                err,
            )
        }
        WorkspaceLoadError::StoreLoadError(
            err @ (StoreLoadError::ReadError { .. } | StoreLoadError::Backend(_)),
        ) => internal_error_with_message("The repository appears broken or inaccessible", err),
        WorkspaceLoadError::StoreLoadError(StoreLoadError::Signing(err)) => user_error(err),
        WorkspaceLoadError::WorkingCopyState(err) => internal_error(err),
        WorkspaceLoadError::DecodeRepoPath(_) | WorkspaceLoadError::Path(_) => user_error(err),
    }
}

pub fn start_repo_transaction(repo: &Arc<ReadonlyRepo>, string_args: &[String]) -> Transaction {
    let mut tx = repo.start_transaction();
    // TODO: Either do better shell-escaping here or store the values in some list
    // type (which we currently don't have).
    let shell_escape = |arg: &String| {
        if arg.as_bytes().iter().all(|b| {
            matches!(b,
                b'A'..=b'Z'
                | b'a'..=b'z'
                | b'0'..=b'9'
                | b','
                | b'-'
                | b'.'
                | b'/'
                | b':'
                | b'@'
                | b'_'
            )
        }) {
            arg.clone()
        } else {
            format!("'{}'", arg.replace('\'', "\\'"))
        }
    };
    let mut quoted_strings = vec!["jj".to_string()];
    quoted_strings.extend(string_args.iter().skip(1).map(shell_escape));
    tx.set_tag("args".to_string(), quoted_strings.join(" "));
    tx
}

/// Check if the working copy is stale and reload the repo if the repo is ahead
/// of the working copy.
///
/// Returns Ok(None) if the workspace doesn't exist in the repo (presumably
/// because it was deleted).
fn handle_stale_working_copy(
    locked_wc: &mut dyn LockedWorkingCopy,
    repo: Arc<ReadonlyRepo>,
    workspace_name: &WorkspaceName,
) -> Result<Option<(Arc<ReadonlyRepo>, Commit)>, SnapshotWorkingCopyError> {
    let get_wc_commit = |repo: &ReadonlyRepo| -> Result<Option<_>, _> {
        repo.view()
            .get_wc_commit_id(workspace_name)
            .map(|id| repo.store().get_commit(id))
            .transpose()
            .map_err(snapshot_command_error)
    };
    let Some(wc_commit) = get_wc_commit(&repo)? else {
        return Ok(None);
    };
    let old_op_id = locked_wc.old_operation_id().clone();
    match WorkingCopyFreshness::check_stale(locked_wc, &wc_commit, &repo) {
        Ok(WorkingCopyFreshness::Fresh) => Ok(Some((repo, wc_commit))),
        Ok(WorkingCopyFreshness::Updated(wc_operation)) => {
            let repo = repo
                .reload_at(&wc_operation)
                .map_err(snapshot_command_error)?;
            if let Some(wc_commit) = get_wc_commit(&repo)? {
                Ok(Some((repo, wc_commit)))
            } else {
                Ok(None)
            }
        }
        Ok(WorkingCopyFreshness::WorkingCopyStale) => Err(
            SnapshotWorkingCopyError::StaleWorkingCopy(user_error_with_hint(
                format!(
                    "The working copy is stale (not updated since operation {}).",
                    short_operation_hash(&old_op_id)
                ),
                "Run `jj workspace update-stale` to update it.
See https://docs.jj-vcs.dev/latest/working-copy/#stale-working-copy \
                 for more information.",
            )),
        ),
        Ok(WorkingCopyFreshness::SiblingOperation) => Err(
            SnapshotWorkingCopyError::StaleWorkingCopy(internal_error(format!(
                "The repo was loaded at operation {}, which seems to be a sibling of the working \
                 copy's operation {}",
                short_operation_hash(repo.op_id()),
                short_operation_hash(&old_op_id)
            ))),
        ),
        Err(OpStoreError::ObjectNotFound { .. }) => Err(
            SnapshotWorkingCopyError::StaleWorkingCopy(user_error_with_hint(
                "Could not read working copy's operation.",
                "Run `jj workspace update-stale` to recover.
See https://docs.jj-vcs.dev/latest/working-copy/#stale-working-copy \
                 for more information.",
            )),
        ),
        Err(e) => Err(snapshot_command_error(e)),
    }
}

fn update_stale_working_copy(
    mut locked_ws: LockedWorkspace,
    op_id: OperationId,
    stale_commit: &Commit,
    new_commit: &Commit,
) -> Result<CheckoutStats, CommandError> {
    // The same check as start_working_copy_mutation(), but with the stale
    // working-copy commit.
    if stale_commit.tree().tree_ids_and_labels()
        != locked_ws.locked_wc().old_tree().tree_ids_and_labels()
    {
        return Err(user_error("Concurrent working copy operation. Try again."));
    }
    let stats = locked_ws
        .locked_wc()
        .check_out(new_commit)
        .block_on()
        .map_err(|err| {
            internal_error_with_message(
                format!("Failed to check out commit {}", new_commit.id().hex()),
                err,
            )
        })?;
    locked_ws.finish(op_id)?;

    Ok(stats)
}

/// Prints a list of commits by the given summary template. The list may be
/// elided. Use this to show created, rewritten, or abandoned commits.
pub fn print_updated_commits<'a>(
    formatter: &mut dyn Formatter,
    template: &TemplateRenderer<Commit>,
    commits: impl IntoIterator<Item = &'a Commit>,
) -> io::Result<()> {
    let mut commits = commits.into_iter().fuse();
    for commit in commits.by_ref().take(10) {
        write!(formatter, "  ")?;
        template.format(commit, formatter)?;
        writeln!(formatter)?;
    }
    if commits.next().is_some() {
        writeln!(formatter, "  ...")?;
    }
    Ok(())
}

#[instrument(skip_all)]
pub fn print_conflicted_paths(
    conflicts: Vec<(RepoPathBuf, BackendResult<MergedTreeValue>)>,
    formatter: &mut dyn Formatter,
    workspace_command: &WorkspaceCommandHelper,
) -> Result<(), CommandError> {
    let formatted_paths = conflicts
        .iter()
        .map(|(path, _conflict)| workspace_command.format_file_path(path))
        .collect_vec();
    let max_path_len = formatted_paths.iter().map(|p| p.len()).max().unwrap_or(0);
    let formatted_paths = formatted_paths
        .into_iter()
        .map(|p| format!("{:width$}", p, width = max_path_len.min(32) + 3));

    for ((_, conflict), formatted_path) in std::iter::zip(conflicts, formatted_paths) {
        // TODO: Display the error for the path instead of failing the whole command if
        // `conflict` is an error?
        let conflict = conflict?.simplify();
        let sides = conflict.num_sides();
        let n_adds = conflict.adds().flatten().count();
        let deletions = sides - n_adds;

        let mut seen_objects = BTreeMap::new(); // Sort for consistency and easier testing
        if deletions > 0 {
            seen_objects.insert(
                format!(
                    // Starting with a number sorts this first
                    "{deletions} deletion{}",
                    if deletions > 1 { "s" } else { "" }
                ),
                "normal", // Deletions don't interfere with `jj resolve` or diff display
            );
        }
        // TODO: We might decide it's OK for `jj resolve` to ignore special files in the
        // `removes` of a conflict (see e.g. https://github.com/jj-vcs/jj/pull/978). In
        // that case, `conflict.removes` should be removed below.
        for term in itertools::chain(conflict.removes(), conflict.adds()).flatten() {
            seen_objects.insert(
                match term {
                    TreeValue::File {
                        executable: false, ..
                    } => continue,
                    TreeValue::File {
                        executable: true, ..
                    } => "an executable",
                    TreeValue::Symlink(_) => "a symlink",
                    TreeValue::Tree(_) => "a directory",
                    TreeValue::GitSubmodule(_) => "a git submodule",
                }
                .to_string(),
                "difficult",
            );
        }

        write!(formatter, "{formatted_path} ")?;
        {
            let mut formatter = formatter.labeled("conflict_description");
            let print_pair = |formatter: &mut dyn Formatter, (text, label): &(String, &str)| {
                write!(formatter.labeled(label), "{text}")
            };
            print_pair(
                *formatter,
                &(
                    format!("{sides}-sided"),
                    if sides > 2 { "difficult" } else { "normal" },
                ),
            )?;
            write!(formatter, " conflict")?;

            if !seen_objects.is_empty() {
                write!(formatter, " including ")?;
                let seen_objects = seen_objects.into_iter().collect_vec();
                match &seen_objects[..] {
                    [] => unreachable!(),
                    [only] => print_pair(*formatter, only)?,
                    [first, middle @ .., last] => {
                        print_pair(*formatter, first)?;
                        for pair in middle {
                            write!(formatter, ", ")?;
                            print_pair(*formatter, pair)?;
                        }
                        write!(formatter, " and ")?;
                        print_pair(*formatter, last)?;
                    }
                }
            }
        }
        writeln!(formatter)?;
    }
    Ok(())
}

/// Build human-readable messages explaining why the file was not tracked
fn build_untracked_reason_message(reason: &UntrackedReason) -> Option<String> {
    match reason {
        UntrackedReason::FileTooLarge { size, max_size } => {
            // Show both exact and human bytes sizes to avoid something
            // like '1.0MiB, maximum size allowed is ~1.0MiB'
            let size_approx = HumanByteSize(*size);
            let max_size_approx = HumanByteSize(*max_size);
            Some(format!(
                "{size_approx} ({size} bytes); the maximum size allowed is {max_size_approx} \
                 ({max_size} bytes)",
            ))
        }
        // Paths with UntrackedReason::FileNotAutoTracked shouldn't be warned about
        // every time we make a snapshot. These paths will be printed by
        // "jj status" instead.
        UntrackedReason::FileNotAutoTracked => None,
    }
}

/// Print a warning to the user, listing untracked files that he may care about
pub fn print_untracked_files(
    ui: &Ui,
    untracked_paths: &BTreeMap<RepoPathBuf, UntrackedReason>,
    path_converter: &RepoPathUiConverter,
) -> io::Result<()> {
    let mut untracked_paths = untracked_paths
        .iter()
        .filter_map(|(path, reason)| build_untracked_reason_message(reason).map(|m| (path, m)))
        .peekable();

    if untracked_paths.peek().is_some() {
        writeln!(ui.warning_default(), "Refused to snapshot some files:")?;
        let mut formatter = ui.stderr_formatter();
        for (path, message) in untracked_paths {
            let ui_path = path_converter.format_file_path(path);
            writeln!(formatter, "  {ui_path}: {message}")?;
        }
    }

    Ok(())
}

pub fn print_snapshot_stats(
    ui: &Ui,
    stats: &SnapshotStats,
    path_converter: &RepoPathUiConverter,
) -> io::Result<()> {
    print_untracked_files(ui, &stats.untracked_paths, path_converter)?;

    let large_files_sizes = stats
        .untracked_paths
        .values()
        .filter_map(|reason| match reason {
            UntrackedReason::FileTooLarge { size, .. } => Some(size),
            UntrackedReason::FileNotAutoTracked => None,
        });
    if let Some(size) = large_files_sizes.max() {
        writedoc!(
            ui.hint_default(),
            r"
            This is to prevent large files from being added by accident. You can fix this by:
              - Adding the file to `.gitignore`
              - Run `jj config set --repo snapshot.max-new-file-size {size}`
                This will increase the maximum file size allowed for new files, in this repository only.
              - Run `jj --config snapshot.max-new-file-size={size} st`
                This will increase the maximum file size allowed for new files, for this command only.
            "
        )?;
    }
    Ok(())
}

pub fn print_checkout_stats(
    ui: &Ui,
    stats: &CheckoutStats,
    new_commit: &Commit,
) -> Result<(), std::io::Error> {
    if stats.added_files > 0 || stats.updated_files > 0 || stats.removed_files > 0 {
        writeln!(
            ui.status(),
            "Added {} files, modified {} files, removed {} files",
            stats.added_files,
            stats.updated_files,
            stats.removed_files
        )?;
    }
    if stats.skipped_files != 0 {
        writeln!(
            ui.warning_default(),
            "{} of those updates were skipped because there were conflicting changes in the \
             working copy.",
            stats.skipped_files
        )?;
        writeln!(
            ui.hint_default(),
            "Inspect the changes compared to the intended target with `jj diff --from {}`.
Discard the conflicting changes with `jj restore --from {}`.",
            short_commit_hash(new_commit.id()),
            short_commit_hash(new_commit.id())
        )?;
    }
    Ok(())
}

/// Prints warning about explicit paths that don't match any of the tree
/// entries.
pub fn print_unmatched_explicit_paths<'a>(
    ui: &Ui,
    workspace_command: &WorkspaceCommandHelper,
    expression: &FilesetExpression,
    trees: impl IntoIterator<Item = &'a MergedTree>,
) -> io::Result<()> {
    let mut explicit_paths = expression.explicit_paths().collect_vec();
    for tree in trees {
        // TODO: propagate errors
        explicit_paths.retain(|&path| tree.path_value(path).unwrap().is_absent());
    }

    if !explicit_paths.is_empty() {
        let ui_paths = explicit_paths
            .iter()
            .map(|&path| workspace_command.format_file_path(path))
            .join(", ");
        writeln!(
            ui.warning_default(),
            "No matching entries for paths: {ui_paths}"
        )?;
    }

    Ok(())
}

pub fn update_working_copy(
    repo: &Arc<ReadonlyRepo>,
    workspace: &mut Workspace,
    old_commit: Option<&Commit>,
    new_commit: &Commit,
) -> Result<CheckoutStats, CommandError> {
    let old_tree = old_commit.map(|commit| commit.tree());
    // TODO: CheckoutError::ConcurrentCheckout should probably just result in a
    // warning for most commands (but be an error for the checkout command)
    let stats = workspace
        .check_out(repo.op_id().clone(), old_tree.as_ref(), new_commit)
        .map_err(|err| {
            internal_error_with_message(
                format!("Failed to check out commit {}", new_commit.id().hex()),
                err,
            )
        })?;
    Ok(stats)
}

/// Returns the special remote name that should be ignored by default.
pub fn default_ignored_remote_name(store: &Store) -> Option<&'static RemoteName> {
    #[cfg(feature = "git")]
    {
        use jj_lib::git;
        if git::get_git_backend(store).is_ok() {
            return Some(git::REMOTE_NAME_FOR_LOCAL_GIT_REPO);
        }
    }
    None
}

/// Whether or not the `bookmark` has any tracked remotes (i.e. is a tracking
/// local bookmark.)
pub fn has_tracked_remote_bookmarks(repo: &dyn Repo, bookmark: &RefName) -> bool {
    let remote_matcher = match default_ignored_remote_name(repo.store()) {
        Some(remote) => StringExpression::exact(remote).negated().to_matcher(),
        None => StringMatcher::all(),
    };
    repo.view()
        .remote_bookmarks_matching(&StringMatcher::exact(bookmark), &remote_matcher)
        .any(|(_, remote_ref)| remote_ref.is_tracked())
}

pub fn load_template_aliases(
    ui: &Ui,
    stacked_config: &StackedConfig,
) -> Result<TemplateAliasesMap, CommandError> {
    let table_name = ConfigNamePathBuf::from_iter(["template-aliases"]);
    let mut aliases_map = TemplateAliasesMap::new();
    // Load from all config layers in order. 'f(x)' in default layer should be
    // overridden by 'f(a)' in user.
    for layer in stacked_config.layers() {
        let table = match layer.look_up_table(&table_name) {
            Ok(Some(table)) => table,
            Ok(None) => continue,
            Err(item) => {
                return Err(ConfigGetError::Type {
                    name: table_name.to_string(),
                    error: format!("Expected a table, but is {}", item.type_name()).into(),
                    source_path: layer.path.clone(),
                }
                .into());
            }
        };
        for (decl, item) in table.iter() {
            let r = item
                .as_str()
                .ok_or_else(|| format!("Expected a string, but is {}", item.type_name()))
                .and_then(|v| aliases_map.insert(decl, v).map_err(|e| e.to_string()));
            if let Err(s) = r {
                writeln!(
                    ui.warning_default(),
                    "Failed to load `{table_name}.{decl}`: {s}"
                )?;
            }
        }
    }
    Ok(aliases_map)
}

/// Helper to reformat content of log-like commands.
#[derive(Clone, Debug)]
pub struct LogContentFormat {
    width: usize,
    word_wrap: bool,
}

impl LogContentFormat {
    /// Creates new formatting helper for the terminal.
    pub fn new(ui: &Ui, settings: &UserSettings) -> Result<Self, ConfigGetError> {
        Ok(Self {
            width: ui.term_width(),
            word_wrap: settings.get_bool("ui.log-word-wrap")?,
        })
    }

    /// Subtracts the given `width` and returns new formatting helper.
    #[must_use]
    pub fn sub_width(&self, width: usize) -> Self {
        Self {
            width: self.width.saturating_sub(width),
            word_wrap: self.word_wrap,
        }
    }

    /// Current width available to content.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Writes content which will optionally be wrapped at the current width.
    pub fn write<E: From<io::Error>>(
        &self,
        formatter: &mut dyn Formatter,
        content_fn: impl FnOnce(&mut dyn Formatter) -> Result<(), E>,
    ) -> Result<(), E> {
        if self.word_wrap {
            let mut recorder = FormatRecorder::new();
            content_fn(&mut recorder)?;
            text_util::write_wrapped(formatter, &recorder, self.width)?;
        } else {
            content_fn(formatter)?;
        }
        Ok(())
    }
}

pub fn short_commit_hash(commit_id: &CommitId) -> String {
    format!("{commit_id:.12}")
}

pub fn short_change_hash(change_id: &ChangeId) -> String {
    format!("{change_id:.12}")
}

pub fn short_operation_hash(operation_id: &OperationId) -> String {
    format!("{operation_id:.12}")
}

/// Wrapper around a `DiffEditor` to conditionally start interactive session.
#[derive(Clone, Debug)]
pub enum DiffSelector {
    NonInteractive,
    Interactive(DiffEditor),
}

impl DiffSelector {
    pub fn is_interactive(&self) -> bool {
        matches!(self, Self::Interactive(_))
    }

    /// Restores diffs from the `right_tree` to the `left_tree` by using an
    /// interactive editor if enabled.
    ///
    /// Only files matching the `matcher` will be copied to the new tree.
    pub fn select(
        &self,
        trees: Diff<&MergedTree>,
        matcher: &dyn Matcher,
        format_instructions: impl FnOnce() -> String,
    ) -> Result<MergedTree, CommandError> {
        let selected_tree = restore_tree(trees.after, trees.before, matcher).block_on()?;
        match self {
            Self::NonInteractive => Ok(selected_tree),
            Self::Interactive(editor) => {
                // edit_diff_external() is designed to edit the right tree,
                // whereas we want to update the left tree. Unmatched paths
                // shouldn't be based off the right tree.
                Ok(editor.edit(
                    Diff::new(trees.before, &selected_tree),
                    matcher,
                    format_instructions,
                )?)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct RemoteBookmarkNamePattern {
    pub bookmark: StringPattern,
    pub remote: StringPattern,
}

impl FromStr for RemoteBookmarkNamePattern {
    type Err = String;

    fn from_str(src: &str) -> Result<Self, Self::Err> {
        // The kind prefix applies to both bookmark and remote fragments. It's
        // weird that unanchored patterns like substring:bookmark@remote is split
        // into two, but I can't think of a better syntax.
        // TODO: should we disable substring pattern? what if we added regex?
        let (maybe_kind, pat) = src
            .split_once(':')
            .map_or((None, src), |(kind, pat)| (Some(kind), pat));
        let to_pattern = |pat: &str| {
            if let Some(kind) = maybe_kind {
                StringPattern::from_str_kind(pat, kind).map_err(|err| err.to_string())
            } else {
                Ok(StringPattern::exact(pat))
            }
        };
        // TODO: maybe reuse revset parser to handle bookmark/remote name containing @
        let (bookmark, remote) = pat.rsplit_once('@').ok_or_else(|| {
            "remote bookmark must be specified in bookmark@remote form".to_owned()
        })?;
        Ok(Self {
            bookmark: to_pattern(bookmark)?,
            remote: to_pattern(remote)?,
        })
    }
}

impl RemoteBookmarkNamePattern {
    pub fn as_exact(&self) -> Option<RemoteRefSymbol<'_>> {
        let bookmark = RefName::new(self.bookmark.as_exact()?);
        let remote = RemoteName::new(self.remote.as_exact()?);
        Some(bookmark.to_remote_symbol(remote))
    }
}

impl fmt::Display for RemoteBookmarkNamePattern {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: use revset::format_remote_symbol() if FromStr is migrated to
        // the revset parser.
        let Self { bookmark, remote } = self;
        write!(f, "{bookmark}@{remote}")
    }
}

/// Computes the location (new parents and new children) to place commits.
///
/// The `destination` argument is mutually exclusive to the `insert_after` and
/// `insert_before` arguments.
pub fn compute_commit_location(
    ui: &Ui,
    workspace_command: &WorkspaceCommandHelper,
    destination: Option<&[RevisionArg]>,
    insert_after: Option<&[RevisionArg]>,
    insert_before: Option<&[RevisionArg]>,
    commit_type: &str,
) -> Result<(Vec<CommitId>, Vec<CommitId>), CommandError> {
    let resolve_revisions =
        |revisions: Option<&[RevisionArg]>| -> Result<Option<Vec<CommitId>>, CommandError> {
            if let Some(revisions) = revisions {
                Ok(Some(
                    workspace_command
                        .resolve_some_revsets_default_single(ui, revisions)?
                        .into_iter()
                        .collect_vec(),
                ))
            } else {
                Ok(None)
            }
        };
    let destination_commit_ids = resolve_revisions(destination)?;
    let after_commit_ids = resolve_revisions(insert_after)?;
    let before_commit_ids = resolve_revisions(insert_before)?;

    let (new_parent_ids, new_child_ids) =
        match (destination_commit_ids, after_commit_ids, before_commit_ids) {
            (Some(destination_commit_ids), None, None) => (destination_commit_ids, vec![]),
            (None, Some(after_commit_ids), Some(before_commit_ids)) => {
                (after_commit_ids, before_commit_ids)
            }
            (None, Some(after_commit_ids), None) => {
                let new_child_ids: Vec<_> = RevsetExpression::commits(after_commit_ids.clone())
                    .children()
                    .evaluate(workspace_command.repo().as_ref())?
                    .iter()
                    .try_collect()?;

                (after_commit_ids, new_child_ids)
            }
            (None, None, Some(before_commit_ids)) => {
                let before_commits: Vec<_> = before_commit_ids
                    .iter()
                    .map(|id| workspace_command.repo().store().get_commit(id))
                    .try_collect()?;
                // Not using `RevsetExpression::parents` here to persist the order of parents
                // specified in `before_commits`.
                let new_parent_ids = before_commits
                    .iter()
                    .flat_map(|commit| commit.parent_ids())
                    .unique()
                    .cloned()
                    .collect_vec();

                (new_parent_ids, before_commit_ids)
            }
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
                panic!("destination cannot be used with insert_after/insert_before")
            }
            (None, None, None) => {
                panic!("expected at least one of destination or insert_after/insert_before")
            }
        };

    if !new_child_ids.is_empty() {
        workspace_command.check_rewritable(new_child_ids.iter())?;
        ensure_no_commit_loop(
            workspace_command.repo().as_ref(),
            &RevsetExpression::commits(new_child_ids.clone()),
            &RevsetExpression::commits(new_parent_ids.clone()),
            commit_type,
        )?;
    }

    Ok((new_parent_ids, new_child_ids))
}

/// Ensure that there is no possible cycle between the potential children and
/// parents of the given commits.
fn ensure_no_commit_loop(
    repo: &ReadonlyRepo,
    children_expression: &Arc<ResolvedRevsetExpression>,
    parents_expression: &Arc<ResolvedRevsetExpression>,
    commit_type: &str,
) -> Result<(), CommandError> {
    if let Some(commit_id) = children_expression
        .dag_range_to(parents_expression)
        .evaluate(repo)?
        .iter()
        .next()
    {
        let commit_id = commit_id?;
        return Err(user_error(format!(
            "Refusing to create a loop: commit {} would be both an ancestor and a descendant of \
             the {commit_type}",
            short_commit_hash(&commit_id),
        )));
    }
    Ok(())
}

/// Jujutsu (An experimental VCS)
///
/// To get started, see the tutorial [`jj help -k tutorial`].
///
/// [`jj help -k tutorial`]:
///     https://docs.jj-vcs.dev/latest/tutorial/
#[derive(clap::Parser, Clone, Debug)]
#[command(name = "jj")]
pub struct Args {
    #[command(flatten)]
    pub global_args: GlobalArgs,
}

#[derive(clap::Args, Clone, Debug)]
#[command(next_help_heading = "Global Options")]
pub struct GlobalArgs {
    /// Path to repository to operate on
    ///
    /// By default, Jujutsu searches for the closest .jj/ directory in an
    /// ancestor of the current working directory.
    #[arg(long, short = 'R', global = true, value_hint = clap::ValueHint::DirPath)]
    pub repository: Option<String>,
    /// Don't snapshot the working copy, and don't update it
    ///
    /// By default, Jujutsu snapshots the working copy at the beginning of every
    /// command. The working copy is also updated at the end of the command,
    /// if the command modified the working-copy commit (`@`). If you want
    /// to avoid snapshotting the working copy and instead see a possibly
    /// stale working-copy commit, you can use `--ignore-working-copy`.
    /// This may be useful e.g. in a command prompt, especially if you have
    /// another process that commits the working copy.
    ///
    /// Loading the repository at a specific operation with `--at-operation`
    /// implies `--ignore-working-copy`.
    #[arg(long, global = true)]
    pub ignore_working_copy: bool,
    /// Allow rewriting immutable commits
    ///
    /// By default, Jujutsu prevents rewriting commits in the configured set of
    /// immutable commits. This option disables that check and lets you rewrite
    /// any commit but the root commit.
    ///
    /// This option only affects the check. It does not affect the
    /// `immutable_heads()` revset or the `immutable` template keyword.
    #[arg(long, global = true)]
    pub ignore_immutable: bool,
    /// Operation to load the repo at
    ///
    /// Operation to load the repo at. By default, Jujutsu loads the repo at the
    /// most recent operation, or at the merge of the divergent operations if
    /// any.
    ///
    /// You can use `--at-op=<operation ID>` to see what the repo looked like at
    /// an earlier operation. For example `jj --at-op=<operation ID> st` will
    /// show you what `jj st` would have shown you when the given operation had
    /// just finished. `--at-op=@` is pretty much the same as the default except
    /// that divergent operations will never be merged.
    ///
    /// Use `jj op log` to find the operation ID you want. Any unambiguous
    /// prefix of the operation ID is enough.
    ///
    /// When loading the repo at an earlier operation, the working copy will be
    /// ignored, as if `--ignore-working-copy` had been specified.
    ///
    /// It is possible to run mutating commands when loading the repo at an
    /// earlier operation. Doing that is equivalent to having run concurrent
    /// commands starting at the earlier operation. There's rarely a reason to
    /// do that, but it is possible.
    #[arg(
        long,
        visible_alias = "at-op",
        global = true,
        add = ArgValueCandidates::new(complete::operations),
    )]
    pub at_operation: Option<String>,
    /// Enable debug logging
    #[arg(long, global = true)]
    pub debug: bool,

    #[command(flatten)]
    pub early_args: EarlyArgs,
}

#[derive(clap::Args, Clone, Debug)]
pub struct EarlyArgs {
    /// When to colorize output
    #[arg(long, value_name = "WHEN", global = true)]
    pub color: Option<ColorChoice>,
    /// Silence non-primary command output
    ///
    /// For example, `jj file list` will still list files, but it won't tell
    /// you if the working copy was snapshotted or if descendants were rebased.
    ///
    /// Warnings and errors will still be printed.
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    // Parsing with ignore_errors will crash if this is bool, so use
    // Option<bool>.
    pub quiet: Option<bool>,
    /// Disable the pager
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    // Parsing with ignore_errors will crash if this is bool, so use
    // Option<bool>.
    pub no_pager: Option<bool>,
    /// Additional configuration options (can be repeated)
    ///
    /// The name should be specified as TOML dotted keys. The value should be
    /// specified as a TOML expression. If string value isn't enclosed by any
    /// TOML constructs (such as array notation), quotes can be omitted.
    #[arg(long, value_name = "NAME=VALUE", global = true, add = ArgValueCompleter::new(complete::leaf_config_key_value))]
    pub config: Vec<String>,
    /// Additional configuration files (can be repeated)
    #[arg(long, value_name = "PATH", global = true, value_hint = clap::ValueHint::FilePath)]
    pub config_file: Vec<String>,
}

impl EarlyArgs {
    pub(crate) fn merged_config_args(&self, matches: &ArgMatches) -> Vec<(ConfigArgKind, &str)> {
        merge_args_with(
            matches,
            &[("config", &self.config), ("config_file", &self.config_file)],
            |id, value| match id {
                "config" => (ConfigArgKind::Item, value.as_ref()),
                "config_file" => (ConfigArgKind::File, value.as_ref()),
                _ => unreachable!("unexpected id {id:?}"),
            },
        )
    }

    fn has_config_args(&self) -> bool {
        !self.config.is_empty() || !self.config_file.is_empty()
    }
}

/// Wrapper around revset expression argument.
///
/// An empty string is rejected early by the CLI value parser, but it's still
/// allowed to construct an empty `RevisionArg` from a config value for
/// example. An empty expression will be rejected by the revset parser.
#[derive(Clone, Debug)]
pub struct RevisionArg(Cow<'static, str>);

impl RevisionArg {
    /// The working-copy symbol, which is the default of the most commands.
    pub const AT: Self = Self(Cow::Borrowed("@"));
}

impl From<String> for RevisionArg {
    fn from(s: String) -> Self {
        Self(s.into())
    }
}

impl AsRef<str> for RevisionArg {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RevisionArg {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ValueParserFactory for RevisionArg {
    type Parser = MapValueParser<NonEmptyStringValueParser, fn(String) -> Self>;

    fn value_parser() -> Self::Parser {
        NonEmptyStringValueParser::new().map(Self::from)
    }
}

/// Merges multiple clap args in order of appearance.
///
/// The `id_values` is a list of `(id, values)` pairs, where `id` is the name of
/// the clap `Arg`, and `values` are the parsed values for that arg. The
/// `convert` function transforms each `(id, value)` pair to e.g. an enum.
///
/// This is a workaround for <https://github.com/clap-rs/clap/issues/3146>.
pub fn merge_args_with<'k, 'v, T, U>(
    matches: &ArgMatches,
    id_values: &[(&'k str, &'v [T])],
    mut convert: impl FnMut(&'k str, &'v T) -> U,
) -> Vec<U> {
    let mut pos_values: Vec<(usize, U)> = Vec::new();
    for (id, values) in id_values {
        pos_values.extend(itertools::zip_eq(
            matches.indices_of(id).into_iter().flatten(),
            values.iter().map(|v| convert(id, v)),
        ));
    }
    pos_values.sort_unstable_by_key(|&(pos, _)| pos);
    pos_values.into_iter().map(|(_, value)| value).collect()
}

fn get_string_or_array(
    config: &StackedConfig,
    key: &'static str,
) -> Result<Vec<String>, ConfigGetError> {
    config
        .get(key)
        .map(|string| vec![string])
        .or_else(|_| config.get::<Vec<String>>(key))
}

fn resolve_default_command(
    ui: &Ui,
    config: &StackedConfig,
    app: &Command,
    mut string_args: Vec<String>,
) -> Result<Vec<String>, CommandError> {
    const PRIORITY_FLAGS: &[&str] = &["--help", "-h", "--version", "-V"];

    let has_priority_flag = string_args
        .iter()
        .any(|arg| PRIORITY_FLAGS.contains(&arg.as_str()));
    if has_priority_flag {
        return Ok(string_args);
    }

    let app_clone = app
        .clone()
        .allow_external_subcommands(true)
        .ignore_errors(true);
    let matches = app_clone.try_get_matches_from(&string_args).ok();

    if let Some(matches) = matches
        && matches.subcommand_name().is_none()
    {
        let args = get_string_or_array(config, "ui.default-command").optional()?;
        if args.is_none() {
            writeln!(
                ui.hint_default(),
                "Use `jj -h` for a list of available commands."
            )?;
            writeln!(
                ui.hint_no_heading(),
                "Run `jj config set --user ui.default-command log` to disable this message."
            )?;
        }
        let default_command = args.unwrap_or_else(|| vec!["log".to_string()]);

        // Insert the default command directly after the path to the binary.
        string_args.splice(1..1, default_command);
    }
    Ok(string_args)
}

fn resolve_aliases(
    ui: &Ui,
    config: &StackedConfig,
    app: &Command,
    mut string_args: Vec<String>,
) -> Result<Vec<String>, CommandError> {
    let defined_aliases: HashSet<_> = config.table_keys("aliases").collect();
    let mut resolved_aliases = HashSet::new();
    let mut real_commands = HashSet::new();
    for command in app.get_subcommands() {
        real_commands.insert(command.get_name());
        for alias in command.get_all_aliases() {
            real_commands.insert(alias);
        }
    }
    for alias in defined_aliases.intersection(&real_commands).sorted() {
        writeln!(
            ui.warning_default(),
            "Cannot define an alias that overrides the built-in command '{alias}'"
        )?;
    }

    loop {
        let app_clone = app.clone().allow_external_subcommands(true);
        let matches = app_clone.try_get_matches_from(&string_args).ok();
        if let Some((command_name, submatches)) = matches.as_ref().and_then(|m| m.subcommand())
            && !real_commands.contains(command_name)
        {
            let alias_name = command_name.to_string();
            let alias_args = submatches
                .get_many::<OsString>("")
                .unwrap_or_default()
                .map(|arg| arg.to_str().unwrap().to_string())
                .collect_vec();
            if resolved_aliases.contains(&*alias_name) {
                return Err(user_error(format!(
                    "Recursive alias definition involving `{alias_name}`"
                )));
            }
            if let Some(&alias_name) = defined_aliases.get(&*alias_name) {
                let alias_definition: Vec<String> = config.get(["aliases", alias_name])?;
                assert!(string_args.ends_with(&alias_args));
                string_args.truncate(string_args.len() - 1 - alias_args.len());
                string_args.extend(alias_definition);
                string_args.extend_from_slice(&alias_args);
                resolved_aliases.insert(alias_name);
                continue;
            } else {
                // Not a real command and not an alias, so return what we've resolved so far
                return Ok(string_args);
            }
        }
        // No more alias commands, or hit unknown option
        return Ok(string_args);
    }
}

/// Parse args that must be interpreted early, e.g. before printing help.
fn parse_early_args(
    app: &Command,
    args: &[String],
) -> Result<(EarlyArgs, Vec<ConfigLayer>), CommandError> {
    // ignore_errors() bypasses errors like missing subcommand
    let early_matches = app
        .clone()
        .disable_version_flag(true)
        // Do not emit DisplayHelp error
        .disable_help_flag(true)
        // Do not stop parsing at -h/--help
        .arg(
            clap::Arg::new("help")
                .short('h')
                .long("help")
                .global(true)
                .action(ArgAction::Count),
        )
        .ignore_errors(true)
        .try_get_matches_from(args)?;
    let args = EarlyArgs::from_arg_matches(&early_matches).unwrap();

    let mut config_layers = parse_config_args(&args.merged_config_args(&early_matches))?;
    // Command arguments overrides any other configuration including the
    // variables loaded from --config* arguments.
    let mut layer = ConfigLayer::empty(ConfigSource::CommandArg);
    if let Some(choice) = args.color {
        layer.set_value("ui.color", choice.to_string()).unwrap();
    }
    if args.quiet.unwrap_or_default() {
        layer.set_value("ui.quiet", true).unwrap();
    }
    if args.no_pager.unwrap_or_default() {
        layer.set_value("ui.paginate", "never").unwrap();
    }
    if !layer.is_empty() {
        config_layers.push(layer);
    }
    Ok((args, config_layers))
}

fn handle_shell_completion(
    ui: &Ui,
    app: &Command,
    config: &StackedConfig,
    cwd: &Path,
) -> Result<(), CommandError> {
    let mut orig_args = env::args_os();

    let mut args = vec![];
    // Take the first two arguments as is, they must be passed to clap_complete
    // without any changes. They are usually "jj --".
    args.extend(orig_args.by_ref().take(2));

    // Make sure aliases are expanded before passing them to clap_complete. We
    // skip the first two args ("jj" and "--") for alias resolution, then we
    // stitch the args back together, like clap_complete expects them.
    if orig_args.len() > 0 {
        let complete_index: Option<usize> = env::var("_CLAP_COMPLETE_INDEX")
            .ok()
            .and_then(|s| s.parse().ok());
        let resolved_aliases = if let Some(index) = complete_index {
            // As of clap_complete 4.5.38, zsh completion script doesn't pad an
            // empty arg at the complete position. If the args doesn't include a
            // command name, the default command would be expanded at that
            // position. Therefore, no other command names would be suggested.
            let pad_len = usize::saturating_sub(index + 1, orig_args.len());
            let padded_args = orig_args
                .by_ref()
                .chain(std::iter::repeat_n(OsString::new(), pad_len));

            // Expand aliases left of the completion index.
            let mut expanded_args = expand_args(ui, app, padded_args.take(index + 1), config)?;

            // Adjust env var to compensate for shift of the completion point in the
            // expanded command line.
            // SAFETY: Program is running single-threaded at this point.
            unsafe {
                env::set_var(
                    "_CLAP_COMPLETE_INDEX",
                    (expanded_args.len() - 1).to_string(),
                );
            }

            // Remove extra padding again to align with clap_complete's expectations for
            // zsh.
            let split_off_padding = expanded_args.split_off(expanded_args.len() - pad_len);
            assert!(
                split_off_padding.iter().all(|s| s.is_empty()),
                "split-off padding should only consist of empty strings but was \
                 {split_off_padding:?}",
            );

            // Append the remaining arguments to the right of the completion point.
            expanded_args.extend(to_string_args(orig_args)?);
            expanded_args
        } else {
            expand_args(ui, app, orig_args, config)?
        };
        args.extend(resolved_aliases.into_iter().map(OsString::from));
    }
    let ran_completion = clap_complete::CompleteEnv::with_factory(|| {
        app.clone()
            // for completing aliases
            .allow_external_subcommands(true)
    })
    .try_complete(args.iter(), Some(cwd))?;
    assert!(
        ran_completion,
        "This function should not be called without the COMPLETE variable set."
    );
    Ok(())
}

pub fn expand_args(
    ui: &Ui,
    app: &Command,
    args_os: impl IntoIterator<Item = OsString>,
    config: &StackedConfig,
) -> Result<Vec<String>, CommandError> {
    let string_args = to_string_args(args_os)?;
    let string_args = resolve_default_command(ui, config, app, string_args)?;
    resolve_aliases(ui, config, app, string_args)
}

fn to_string_args(
    args_os: impl IntoIterator<Item = OsString>,
) -> Result<Vec<String>, CommandError> {
    args_os
        .into_iter()
        .map(|arg_os| {
            arg_os
                .into_string()
                .map_err(|_| cli_error("Non-UTF-8 argument"))
        })
        .collect()
}

fn parse_args(app: &Command, string_args: &[String]) -> Result<(ArgMatches, Args), clap::Error> {
    let matches = app
        .clone()
        .arg_required_else_help(true)
        .subcommand_required(true)
        .try_get_matches_from(string_args)?;
    let args = Args::from_arg_matches(&matches).unwrap();
    Ok((matches, args))
}

fn command_name(mut matches: &ArgMatches) -> String {
    let mut command = String::new();
    while let Some((subcommand, new_matches)) = matches.subcommand() {
        if !command.is_empty() {
            command.push(' ');
        }
        command.push_str(subcommand);
        matches = new_matches;
    }
    command
}

pub fn format_template<C: Clone>(ui: &Ui, arg: &C, template: &TemplateRenderer<C>) -> String {
    let mut output = vec![];
    template
        .format(arg, ui.new_formatter(&mut output).as_mut())
        .expect("write() to vec backed formatter should never fail");
    // Template output is usually UTF-8, but it can contain file content.
    output.into_string_lossy()
}

/// CLI command builder and runner.
#[must_use]
pub struct CliRunner<'a> {
    tracing_subscription: TracingSubscription,
    app: Command,
    config_layers: Vec<ConfigLayer>,
    config_migrations: Vec<ConfigMigrationRule>,
    store_factories: StoreFactories,
    working_copy_factories: WorkingCopyFactories,
    workspace_loader_factory: Box<dyn WorkspaceLoaderFactory>,
    revset_extensions: RevsetExtensions,
    commit_template_extensions: Vec<Arc<dyn CommitTemplateLanguageExtension>>,
    operation_template_extensions: Vec<Arc<dyn OperationTemplateLanguageExtension>>,
    dispatch_fn: CliDispatchFn<'a>,
    dispatch_hook_fns: Vec<CliDispatchHookFn<'a>>,
    process_global_args_fns: Vec<ProcessGlobalArgsFn<'a>>,
}

pub type CliDispatchFn<'a> =
    Box<dyn FnOnce(&mut Ui, &CommandHelper) -> Result<(), CommandError> + 'a>;

type CliDispatchHookFn<'a> =
    Box<dyn FnOnce(&mut Ui, &CommandHelper, CliDispatchFn<'a>) -> Result<(), CommandError> + 'a>;

type ProcessGlobalArgsFn<'a> =
    Box<dyn FnOnce(&mut Ui, &ArgMatches) -> Result<(), CommandError> + 'a>;

impl<'a> CliRunner<'a> {
    /// Initializes CLI environment and returns a builder. This should be called
    /// as early as possible.
    pub fn init() -> Self {
        let tracing_subscription = TracingSubscription::init();
        crate::cleanup_guard::init();
        Self {
            tracing_subscription,
            app: crate::commands::default_app(),
            config_layers: crate::config::default_config_layers(),
            config_migrations: crate::config::default_config_migrations(),
            store_factories: StoreFactories::default(),
            working_copy_factories: default_working_copy_factories(),
            workspace_loader_factory: Box::new(DefaultWorkspaceLoaderFactory),
            revset_extensions: Default::default(),
            commit_template_extensions: vec![],
            operation_template_extensions: vec![],
            dispatch_fn: Box::new(crate::commands::run_command),
            dispatch_hook_fns: vec![],
            process_global_args_fns: vec![],
        }
    }

    /// Set the name of the CLI application to be displayed in help messages.
    pub fn name(mut self, name: &str) -> Self {
        self.app = self.app.name(name.to_string());
        self
    }

    /// Set the about message to be displayed in help messages.
    pub fn about(mut self, about: &str) -> Self {
        self.app = self.app.about(about.to_string());
        self
    }

    /// Set the version to be displayed by `jj version`.
    pub fn version(mut self, version: &str) -> Self {
        self.app = self.app.version(version.to_string());
        self
    }

    /// Adds default configs in addition to the normal defaults.
    ///
    /// The `layer.source` must be `Default`. Other sources such as `User` would
    /// be replaced by loaded configuration.
    pub fn add_extra_config(mut self, layer: ConfigLayer) -> Self {
        assert_eq!(layer.source, ConfigSource::Default);
        self.config_layers.push(layer);
        self
    }

    /// Adds config migration rule in addition to the default rules.
    pub fn add_extra_config_migration(mut self, rule: ConfigMigrationRule) -> Self {
        self.config_migrations.push(rule);
        self
    }

    /// Adds `StoreFactories` to be used.
    pub fn add_store_factories(mut self, store_factories: StoreFactories) -> Self {
        self.store_factories.merge(store_factories);
        self
    }

    /// Adds working copy factories to be used.
    pub fn add_working_copy_factories(
        mut self,
        working_copy_factories: WorkingCopyFactories,
    ) -> Self {
        merge_factories_map(&mut self.working_copy_factories, working_copy_factories);
        self
    }

    pub fn set_workspace_loader_factory(
        mut self,
        workspace_loader_factory: Box<dyn WorkspaceLoaderFactory>,
    ) -> Self {
        self.workspace_loader_factory = workspace_loader_factory;
        self
    }

    pub fn add_symbol_resolver_extension(
        mut self,
        symbol_resolver: Box<dyn SymbolResolverExtension>,
    ) -> Self {
        self.revset_extensions.add_symbol_resolver(symbol_resolver);
        self
    }

    pub fn add_revset_function_extension(
        mut self,
        name: &'static str,
        func: RevsetFunction,
    ) -> Self {
        self.revset_extensions.add_custom_function(name, func);
        self
    }

    pub fn add_commit_template_extension(
        mut self,
        commit_template_extension: Box<dyn CommitTemplateLanguageExtension>,
    ) -> Self {
        self.commit_template_extensions
            .push(commit_template_extension.into());
        self
    }

    pub fn add_operation_template_extension(
        mut self,
        operation_template_extension: Box<dyn OperationTemplateLanguageExtension>,
    ) -> Self {
        self.operation_template_extensions
            .push(operation_template_extension.into());
        self
    }

    /// Add a hook that gets called when it's time to run the command. It is
    /// the hook's responsibility to call the given inner dispatch function to
    /// run the command.
    pub fn add_dispatch_hook<F>(mut self, dispatch_hook_fn: F) -> Self
    where
        F: FnOnce(&mut Ui, &CommandHelper, CliDispatchFn) -> Result<(), CommandError> + 'a,
    {
        self.dispatch_hook_fns.push(Box::new(dispatch_hook_fn));
        self
    }

    /// Registers new subcommands in addition to the default ones.
    pub fn add_subcommand<C, F>(mut self, custom_dispatch_fn: F) -> Self
    where
        C: clap::Subcommand,
        F: FnOnce(&mut Ui, &CommandHelper, C) -> Result<(), CommandError> + 'a,
    {
        let old_dispatch_fn = self.dispatch_fn;
        let new_dispatch_fn =
            move |ui: &mut Ui, command_helper: &CommandHelper| match C::from_arg_matches(
                command_helper.matches(),
            ) {
                Ok(command) => custom_dispatch_fn(ui, command_helper, command),
                Err(_) => old_dispatch_fn(ui, command_helper),
            };
        self.app = C::augment_subcommands(self.app);
        self.dispatch_fn = Box::new(new_dispatch_fn);
        self
    }

    /// Registers new global arguments in addition to the default ones.
    pub fn add_global_args<A, F>(mut self, process_before: F) -> Self
    where
        A: clap::Args,
        F: FnOnce(&mut Ui, A) -> Result<(), CommandError> + 'a,
    {
        let process_global_args_fn = move |ui: &mut Ui, matches: &ArgMatches| {
            let custom_args = A::from_arg_matches(matches).unwrap();
            process_before(ui, custom_args)
        };
        self.app = A::augment_args(self.app);
        self.process_global_args_fns
            .push(Box::new(process_global_args_fn));
        self
    }

    #[instrument(skip_all)]
    fn run_internal(self, ui: &mut Ui, mut raw_config: RawConfig) -> Result<(), CommandError> {
        // `cwd` is canonicalized for consistency with `Workspace::workspace_root()` and
        // to easily compute relative paths between them.
        let cwd = env::current_dir()
            .and_then(dunce::canonicalize)
            .map_err(|_| {
                user_error_with_hint(
                    "Could not determine current directory",
                    "Did you update to a commit where the directory doesn't exist or can't be \
                     accessed?",
                )
            })?;
        let mut config_env = ConfigEnv::from_environment();
        let mut last_config_migration_descriptions = Vec::new();
        let mut migrate_config = |config: &mut StackedConfig| -> Result<(), CommandError> {
            last_config_migration_descriptions =
                jj_lib::config::migrate(config, &self.config_migrations)?;
            Ok(())
        };

        // Initial load: user, repo, and workspace-level configs for
        // alias/default-command resolution
        // Use cwd-relative workspace configs to resolve default command and
        // aliases. WorkspaceLoader::init() won't do any heavy lifting other
        // than the path resolution.
        let maybe_cwd_workspace_loader = self
            .workspace_loader_factory
            .create(find_workspace_dir(&cwd))
            .map_err(|err| map_workspace_load_error(err, Some(".")));
        config_env.reload_user_config(&mut raw_config)?;
        if let Ok(loader) = &maybe_cwd_workspace_loader {
            config_env.reset_repo_path(loader.repo_path());
            config_env.reload_repo_config(&mut raw_config)?;
            config_env.reset_workspace_path(loader.workspace_root());
            config_env.reload_workspace_config(&mut raw_config)?;
        }
        let mut config = config_env.resolve_config(&raw_config)?;
        migrate_config(&mut config)?;
        ui.reset(&config)?;

        if env::var_os("COMPLETE").is_some() {
            return handle_shell_completion(&Ui::null(), &self.app, &config, &cwd);
        }

        let string_args = expand_args(ui, &self.app, env::args_os(), &config)?;
        let (args, config_layers) = parse_early_args(&self.app, &string_args)?;
        if !config_layers.is_empty() {
            raw_config.as_mut().extend_layers(config_layers);
            config = config_env.resolve_config(&raw_config)?;
            migrate_config(&mut config)?;
            ui.reset(&config)?;
        }

        if args.has_config_args() {
            warn_if_args_mismatch(ui, &self.app, &config, &string_args)?;
        }

        let (matches, args) = parse_args(&self.app, &string_args)
            .map_err(|err| map_clap_cli_error(err, ui, &config))?;
        if args.global_args.debug {
            // TODO: set up debug logging as early as possible
            self.tracing_subscription.enable_debug_logging()?;
        }
        for process_global_args_fn in self.process_global_args_fns {
            process_global_args_fn(ui, &matches)?;
        }
        config_env.set_command_name(command_name(&matches));

        let maybe_workspace_loader = if let Some(path) = &args.global_args.repository {
            // TODO: maybe path should be canonicalized by WorkspaceLoader?
            let abs_path = cwd.join(path);
            let abs_path = dunce::canonicalize(&abs_path).unwrap_or(abs_path);
            // Invalid -R path is an error. No need to proceed.
            let loader = self
                .workspace_loader_factory
                .create(&abs_path)
                .map_err(|err| map_workspace_load_error(err, Some(path)))?;
            config_env.reset_repo_path(loader.repo_path());
            config_env.reload_repo_config(&mut raw_config)?;
            config_env.reset_workspace_path(loader.workspace_root());
            config_env.reload_workspace_config(&mut raw_config)?;
            Ok(loader)
        } else {
            maybe_cwd_workspace_loader
        };

        // Apply workspace configs, --config arguments, and --when.commands.
        config = config_env.resolve_config(&raw_config)?;
        migrate_config(&mut config)?;
        ui.reset(&config)?;

        // Print only the last migration messages to omit duplicates.
        for (source, desc) in &last_config_migration_descriptions {
            let source_str = match source {
                ConfigSource::Default => "default-provided",
                ConfigSource::EnvBase | ConfigSource::EnvOverrides => "environment-provided",
                ConfigSource::User => "user-level",
                ConfigSource::Repo => "repo-level",
                ConfigSource::Workspace => "workspace-level",
                ConfigSource::CommandArg => "CLI-provided",
            };
            writeln!(
                ui.warning_default(),
                "Deprecated {source_str} config: {desc}"
            )?;
        }

        if args.global_args.repository.is_some() {
            warn_if_args_mismatch(ui, &self.app, &config, &string_args)?;
        }

        let settings = UserSettings::from_config(config)?;
        let command_helper_data = CommandHelperData {
            app: self.app,
            cwd,
            string_args,
            matches,
            global_args: args.global_args,
            config_env,
            config_migrations: self.config_migrations,
            raw_config,
            settings,
            revset_extensions: self.revset_extensions.into(),
            commit_template_extensions: self.commit_template_extensions,
            operation_template_extensions: self.operation_template_extensions,
            maybe_workspace_loader,
            store_factories: self.store_factories,
            working_copy_factories: self.working_copy_factories,
            workspace_loader_factory: self.workspace_loader_factory,
        };
        let command_helper = CommandHelper {
            data: Rc::new(command_helper_data),
        };
        let dispatch_fn = self.dispatch_hook_fns.into_iter().fold(
            self.dispatch_fn,
            |old_dispatch_fn, dispatch_hook_fn| {
                Box::new(move |ui: &mut Ui, command_helper: &CommandHelper| {
                    dispatch_hook_fn(ui, command_helper, old_dispatch_fn)
                })
            },
        );
        (dispatch_fn)(ui, &command_helper)
    }

    #[must_use]
    #[instrument(skip(self))]
    pub fn run(mut self) -> u8 {
        // Tell crossterm to ignore NO_COLOR (we check it ourselves)
        crossterm::style::force_color_output(true);
        let config = config_from_environment(self.config_layers.drain(..));
        // Set up ui assuming the default config has no conditional variables.
        // If it had, the configuration will be fixed by the next ui.reset().
        let mut ui = Ui::with_config(config.as_ref())
            .expect("default config should be valid, env vars are stringly typed");
        let result = self.run_internal(&mut ui, config);
        let exit_code = handle_command_result(&mut ui, result);
        ui.finalize_pager();
        exit_code
    }
}

fn map_clap_cli_error(err: clap::Error, ui: &Ui, config: &StackedConfig) -> CommandError {
    if let Some(ContextValue::String(cmd)) = err.get(ContextKind::InvalidSubcommand) {
        let remove_useless_error_context = |mut err: clap::Error| {
            // Clap suggests unhelpful subcommands, e.g. `config` for `clone`.
            // We don't want suggestions when we know this isn't a misspelling.
            err.remove(ContextKind::SuggestedSubcommand);
            err.remove(ContextKind::Suggested); // Remove an empty line
            err.remove(ContextKind::Usage); // Also unhelpful for these errors.
            err
        };
        match cmd.as_str() {
            // git commands that a brand-new user might type during their first
            // experiments with `jj`
            "clone" | "init" => {
                let cmd = cmd.clone();
                return CommandError::from(remove_useless_error_context(err))
                    .hinted(format!(
                        "You probably want `jj git {cmd}`. See also `jj help git`."
                    ))
                    .hinted(format!(
                        r#"You can configure `aliases.{cmd} = ["git", "{cmd}"]` if you want `jj {cmd}` to work and always use the Git backend."#
                    ));
            }
            "amend" => {
                return CommandError::from(remove_useless_error_context(err))
                    .hinted(
                        r#"You probably want `jj squash`. You can configure `aliases.amend = ["squash"]` if you want `jj amend` to work."#);
            }
            _ => {}
        }
    }
    if let (Some(ContextValue::String(arg)), Some(ContextValue::String(value))) = (
        err.get(ContextKind::InvalidArg),
        err.get(ContextKind::InvalidValue),
    ) && arg.as_str() == "--template <TEMPLATE>"
        && value.is_empty()
    {
        // Suppress the error, it's less important than the original error.
        if let Ok(template_aliases) = load_template_aliases(ui, config) {
            return CommandError::from(err).hinted(format_template_aliases_hint(&template_aliases));
        }
    }
    CommandError::from(err)
}

fn format_template_aliases_hint(template_aliases: &TemplateAliasesMap) -> String {
    let mut hint = String::from("The following template aliases are defined:\n");
    hint.push_str(
        &template_aliases
            .symbol_names()
            .sorted_unstable()
            .map(|name| format!("- {name}"))
            .join("\n"),
    );
    hint
}

// If -R or --config* is specified, check if the expanded arguments differ.
fn warn_if_args_mismatch(
    ui: &Ui,
    app: &Command,
    config: &StackedConfig,
    expected_args: &[String],
) -> Result<(), CommandError> {
    let new_string_args = expand_args(ui, app, env::args_os(), config).ok();
    if new_string_args.as_deref() != Some(expected_args) {
        writeln!(
            ui.warning_default(),
            "Command aliases cannot be loaded from -R/--repository path or --config/--config-file \
             arguments."
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::*;

    #[derive(clap::Parser, Clone, Debug)]
    pub struct TestArgs {
        #[arg(long)]
        pub foo: Vec<u32>,
        #[arg(long)]
        pub bar: Vec<u32>,
        #[arg(long)]
        pub baz: bool,
    }

    #[test]
    fn test_merge_args_with() {
        let command = TestArgs::command();
        let parse = |args: &[&str]| -> Vec<(&'static str, u32)> {
            let matches = command.clone().try_get_matches_from(args).unwrap();
            let args = TestArgs::from_arg_matches(&matches).unwrap();
            merge_args_with(
                &matches,
                &[("foo", &args.foo), ("bar", &args.bar)],
                |id, value| (id, *value),
            )
        };

        assert_eq!(parse(&["jj"]), vec![]);
        assert_eq!(parse(&["jj", "--foo=1"]), vec![("foo", 1)]);
        assert_eq!(
            parse(&["jj", "--foo=1", "--bar=2"]),
            vec![("foo", 1), ("bar", 2)]
        );
        assert_eq!(
            parse(&["jj", "--foo=1", "--baz", "--bar=2", "--foo", "3"]),
            vec![("foo", 1), ("bar", 2), ("foo", 3)]
        );
    }
}
