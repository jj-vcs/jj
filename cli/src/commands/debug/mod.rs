// Copyright 2023 The Jujutsu Authors
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

mod copy_detection;
mod fileset;
mod index;
mod index_changed_paths;
mod init_simple;
mod local_working_copy;
mod object;
mod reindex;
mod revset;
mod snapshot;
mod stacked_table;
mod template;
mod tree;
mod watchman;
mod working_copy;

use clap::Subcommand;
use jj_lib::local_working_copy::LocalWorkingCopy;
use jj_lib::working_copy::WorkingCopy;

use self::copy_detection::CopyDetectionArgs;
use self::copy_detection::cmd_debug_copy_detection;
use self::fileset::DebugFilesetArgs;
use self::fileset::cmd_debug_fileset;
use self::index::DebugIndexArgs;
use self::index::cmd_debug_index;
use self::index_changed_paths::DebugIndexChangedPathsArgs;
use self::index_changed_paths::cmd_debug_index_changed_paths;
use self::init_simple::DebugInitSimpleArgs;
use self::init_simple::cmd_debug_init_simple;
use self::local_working_copy::DebugLocalWorkingCopyArgs;
use self::local_working_copy::cmd_debug_local_working_copy;
use self::object::DebugObjectArgs;
use self::object::cmd_debug_object;
use self::reindex::DebugReindexArgs;
use self::reindex::cmd_debug_reindex;
use self::revset::DebugRevsetArgs;
use self::revset::cmd_debug_revset;
use self::snapshot::DebugSnapshotArgs;
use self::snapshot::cmd_debug_snapshot;
use self::stacked_table::DebugStackedTableArgs;
use self::stacked_table::cmd_debug_stacked_table;
use self::template::DebugTemplateArgs;
use self::template::cmd_debug_template;
use self::tree::DebugTreeArgs;
use self::tree::cmd_debug_tree;
use self::watchman::DebugWatchmanCommand;
use self::watchman::cmd_debug_watchman;
use self::working_copy::DebugWorkingCopyArgs;
use self::working_copy::cmd_debug_working_copy;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::ui::Ui;

/// Low-level commands not intended for users
#[derive(Subcommand, Clone, Debug)]
#[command(hide = true)]
pub enum DebugCommand {
    CopyDetection(CopyDetectionArgs),
    Fileset(DebugFilesetArgs),
    Index(DebugIndexArgs),
    IndexChangedPaths(DebugIndexChangedPathsArgs),
    InitSimple(DebugInitSimpleArgs),
    LocalWorkingCopy(DebugLocalWorkingCopyArgs),
    #[command(subcommand)]
    Object(DebugObjectArgs),
    Reindex(DebugReindexArgs),
    Revset(DebugRevsetArgs),
    Snapshot(DebugSnapshotArgs),
    StackedTable(DebugStackedTableArgs),
    Template(DebugTemplateArgs),
    Tree(DebugTreeArgs),
    #[command(subcommand)]
    Watchman(DebugWatchmanCommand),
    WorkingCopy(DebugWorkingCopyArgs),
}

pub fn cmd_debug(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &DebugCommand,
) -> Result<(), CommandError> {
    match subcommand {
        DebugCommand::CopyDetection(args) => cmd_debug_copy_detection(ui, command, args),
        DebugCommand::Fileset(args) => cmd_debug_fileset(ui, command, args),
        DebugCommand::Index(args) => cmd_debug_index(ui, command, args),
        DebugCommand::IndexChangedPaths(args) => cmd_debug_index_changed_paths(ui, command, args),
        DebugCommand::InitSimple(args) => cmd_debug_init_simple(ui, command, args),
        DebugCommand::LocalWorkingCopy(args) => cmd_debug_local_working_copy(ui, command, args),
        DebugCommand::Object(args) => cmd_debug_object(ui, command, args),
        DebugCommand::Reindex(args) => cmd_debug_reindex(ui, command, args),
        DebugCommand::Revset(args) => cmd_debug_revset(ui, command, args),
        DebugCommand::Snapshot(args) => cmd_debug_snapshot(ui, command, args),
        DebugCommand::StackedTable(args) => cmd_debug_stacked_table(ui, command, args),
        DebugCommand::Template(args) => cmd_debug_template(ui, command, args),
        DebugCommand::Tree(args) => cmd_debug_tree(ui, command, args),
        DebugCommand::Watchman(args) => cmd_debug_watchman(ui, command, args),
        DebugCommand::WorkingCopy(args) => cmd_debug_working_copy(ui, command, args),
    }
}

fn check_local_disk_wc(x: &dyn WorkingCopy) -> Result<&LocalWorkingCopy, CommandError> {
    x.downcast_ref()
        .ok_or_else(|| user_error("This command requires a standard local-disk working copy"))
}
