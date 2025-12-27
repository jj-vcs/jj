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

use std::collections::HashSet;
use std::io::Write as _;

use jj_lib::config::ConfigSource;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::ui::Ui;

/// List all available aliases
///
/// Display all command aliases organized by source, including:
/// - Builtin aliases from jj's code
/// - Default aliases from the built-in configuration
/// - User-defined aliases from configuration files
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct AliasListArgs {}

#[instrument(skip_all)]
pub(crate) fn cmd_alias_list(
    ui: &mut Ui,
    command: &CommandHelper,
    _args: &AliasListArgs,
) -> Result<(), CommandError> {
    let stacked_config = command.raw_config().as_ref();

    // Get code-level builtin command aliases from clap Command definition
    let mut builtin_aliases: Vec<(String, String)> = command
        .app()
        .get_subcommands()
        .flat_map(|cmd| {
            cmd.get_all_aliases()
                .map(|alias| (alias.to_string(), cmd.get_name().to_string()))
        })
        .collect();
    builtin_aliases.sort();

    // Collect config aliases, tracking which names we've seen to deduplicate
    let mut builtin_config = Vec::new();
    let mut user_defined = Vec::new();
    let mut seen_default = HashSet::new();
    let mut seen_user = HashSet::new();

    for layer in stacked_config.layers() {
        if let Ok(Some(item)) = layer.look_up_item("aliases")
            && let Some(aliases_table) = item.as_table_like()
        {
            for (alias_name, _) in aliases_table.iter() {
                if let Ok(expansion) = stacked_config.get::<Vec<String>>(["aliases", alias_name]) {
                    let expansion_str = expansion.join(" ");
                    match layer.source {
                        ConfigSource::Default => {
                            if seen_default.insert(alias_name.to_string()) {
                                builtin_config.push((alias_name.to_string(), expansion_str));
                            }
                        }
                        _ => {
                            if seen_user.insert(alias_name.to_string()) {
                                user_defined.push((alias_name.to_string(), expansion_str));
                            }
                        }
                    }
                }
            }
        }
    }

    builtin_config.sort();
    user_defined.sort();

    let mut formatter = ui.stdout_formatter();
    let mut printed_section = false;

    // Print code-level builtin aliases
    if !builtin_aliases.is_empty() {
        writeln!(formatter, "Builtin aliases:")?;
        for (alias, cmd) in &builtin_aliases {
            writeln!(formatter, "  {alias:<20} -> {cmd}")?;
        }
        printed_section = true;
    }

    // Print config-level default aliases
    if !builtin_config.is_empty() {
        if printed_section {
            writeln!(formatter)?;
        }
        writeln!(formatter, "Default aliases:")?;
        for (alias, expansion) in &builtin_config {
            writeln!(formatter, "  {alias:<20} -> {expansion}")?;
        }
        printed_section = true;
    }

    // Print user-defined aliases
    if !user_defined.is_empty() {
        if printed_section {
            writeln!(formatter)?;
        }
        writeln!(formatter, "User-defined aliases:")?;
        for (alias, expansion) in &user_defined {
            writeln!(formatter, "  {alias:<20} -> {expansion}")?;
        }
    } else if !printed_section {
        writeln!(formatter, "No aliases defined")?;
    }

    Ok(())
}
