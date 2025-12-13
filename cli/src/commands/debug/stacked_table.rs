// Copyright 2025 The Jujutsu Authors
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
use std::path::PathBuf;

use itertools::Itertools as _;
use jj_lib::stacked_table::TableSegment as _;
use jj_lib::stacked_table::TableStore;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error_with_message;
use crate::ui::Ui;

/// Show stats of stacked table
#[derive(clap::Args, Clone, Debug)]
pub struct DebugStackedTableArgs {
    /// Path to table store directory
    #[arg(value_hint = clap::ValueHint::DirPath)]
    dir: String,
    /// Key size in bytes
    #[arg(long, short = 'n')]
    key_size: usize,
}

pub fn cmd_debug_stacked_table(
    ui: &mut Ui,
    _command: &CommandHelper,
    args: &DebugStackedTableArgs,
) -> Result<(), CommandError> {
    let store = TableStore::load(PathBuf::from(&args.dir), args.key_size);
    let table = store
        .get_head()
        .map_err(|err| user_error_with_message("Failed to load stacked table", err))?;
    let mut table_segments = table.ancestor_segments().collect_vec();
    table_segments.reverse();

    let total_num_entries: usize = table_segments
        .iter()
        .map(|table| table.segment_num_entries())
        .sum();
    writeln!(ui.stdout(), "Number of entries: {total_num_entries}")?;

    writeln!(ui.stdout(), "Stats per level:")?;
    for (i, table) in table_segments.iter().enumerate() {
        writeln!(ui.stdout(), "  Level {i}:")?;
        writeln!(
            ui.stdout(),
            "    Number of entries: {}",
            table.segment_num_entries()
        )?;
        writeln!(ui.stdout(), "    Name: {}", table.name())?;
    }
    Ok(())
}
