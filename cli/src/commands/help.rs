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

use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::Write as _;

use clap::builder::PossibleValue;
use clap::builder::StyledStr;
use clap::error::ContextKind;
use crossterm::style::Stylize as _;
use itertools::Itertools as _;
use jj_lib::settings::UserSettings;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::ui::Ui;

/// Print this message or the help of the given subcommand(s)
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct HelpArgs {
    /// Print help for the subcommand(s)
    pub(crate) command: Vec<String>,
    /// Show help for keywords instead of commands
    #[arg(
        long,
        short = 'k',
        conflicts_with = "command",
        value_parser = KEYWORDS
            .iter()
            .map(|k| PossibleValue::new(k.name).help(k.description))
            .collect_vec()
    )]
    pub(crate) keyword: Option<String>,
}

#[instrument(skip_all)]
pub(crate) fn cmd_help(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &HelpArgs,
) -> Result<(), CommandError> {
    if let Some(name) = &args.keyword {
        let keyword = find_keyword(name).expect("clap should check this with `value_parser`");
        ui.request_pager();
        write!(ui.stdout(), "{}", keyword.content)?;

        return Ok(());
    }

    let bin_name = command
        .string_args()
        .first()
        .map_or(command.app().get_name(), |name| name.as_ref());

    // Check if the first command argument is an alias
    let mut resolved_alias_definition: Option<Vec<String>> = None;
    let command_path = if let Some(first_arg) = args.command.first()
        && let Some((original_definition, resolved_definition)) =
            resolve_alias(command.settings(), first_arg)?
    {
        if args.command.len() > 1 {
            return Err(user_error(format!(
                "Invalid arguments following alias '{first_arg}'"
            )));
        }

        resolved_alias_definition = Some(original_definition);
        resolved_definition
    } else {
        args.command.clone()
    };

    let mut args_to_get_command = vec![bin_name];
    args_to_get_command.extend(command_path.iter().map(|s| s.as_str()));

    let mut app = command.app().clone();
    // This propagates global arguments to subcommand, and generates error if
    // the subcommand doesn't exist.
    if let Err(err) = app.try_get_matches_from_mut(args_to_get_command) {
        if err.get(ContextKind::InvalidSubcommand).is_some() {
            return Err(err.into());
        } else {
            // `help log -- -r`, etc. shouldn't generate an argument error.
        }
    }
    // Use command_path (which may be resolved) instead of args.command
    // Walk the subcommand tree, stopping when we hit something that isn't a
    // subcommand First, figure out how many elements are actual subcommands
    let mut subcommand_depth = 0;
    let mut current_cmd = &app;
    for name in &command_path {
        if let Some(subcmd) = current_cmd.find_subcommand(name) {
            current_cmd = subcmd;
            subcommand_depth += 1;
        } else {
            // Not a subcommand (probably an argument like "-r"), stop here
            break;
        }
    }

    // Now walk the mutable path for the actual depth we found
    let subcommand = command_path
        .iter()
        .take(subcommand_depth)
        .try_fold(&mut app, |cmd, name| cmd.find_subcommand_mut(name))
        .unwrap(); // Safe because we already validated the path exists

    ui.request_pager();

    // If this was an alias, print the alias info first
    if let Some(alias_definition) = resolved_alias_definition {
        let alias_display = format_alias_definition(&alias_definition);
        writeln!(ui.stdout(), "{alias_display}")?;
        writeln!(ui.stdout())?; // Blank line separator
    }

    // Render the help for the resolved command
    let help_text = subcommand.render_long_help();
    if ui.color() {
        write!(ui.stdout(), "{}", help_text.ansi())?;
    } else {
        write!(ui.stdout(), "{help_text}")?;
    }
    Ok(())
}

type ResolvedAlias = Option<(Vec<String>, Vec<String>)>;

/// Resolves an alias to its definition, recursively expanding nested aliases.
///
/// Returns `Some((original_definition, resolved_definition))` if the name is an
/// alias, or `None` if it's not an alias.
///
/// The original definition is what the user configured (e.g., `["b",
/// "--no-graph"]`). The resolved definition has all nested aliases expanded
/// (e.g., `["log", "-r", "@", "-T", "bookmarks", "--no-graph"]`).
fn resolve_alias(settings: &UserSettings, alias_name: &str) -> Result<ResolvedAlias, CommandError> {
    let config = settings.config();

    let alias_keys: HashSet<_> = config.table_keys("aliases").collect();
    if !alias_keys.contains(alias_name) {
        return Ok(None);
    }

    let alias_definition: Vec<String> = config.get(["aliases", alias_name])?;

    let mut seen_aliases = HashSet::new();
    let resolved_definition = crate::cli_util::expand_alias_recursively(
        config,
        alias_name,
        &alias_keys,
        &mut seen_aliases,
    )?;

    Ok(Some((alias_definition, resolved_definition)))
}

/// Formats an alias definition for display in help output.
fn format_alias_definition(alias_definition: &[String]) -> String {
    // Use shlex to properly quote the definition if needed
    match shlex::try_join(alias_definition.iter().map(|s| &**s)) {
        Ok(joined) => format!("Alias for \"{joined}\""),
        Err(_) => format!("Alias for {alias_definition:?}"),
    }
}

#[derive(Clone)]
struct Keyword {
    name: &'static str,
    description: &'static str,
    content: &'static str,
}

// TODO: Add all documentation to keywords
//
// Maybe adding some code to build.rs to find all the docs files and build the
// `KEYWORDS` at compile time.
//
// It would be cool to follow the docs hierarchy somehow.
//
// One of the problems would be `config.md`, as it has the same name as a
// subcommand.
//
// TODO: Find a way to render markdown using ANSI escape codes.
//
// Maybe we can steal some ideas from https://github.com/jj-vcs/jj/pull/3130
const KEYWORDS: &[Keyword] = &[
    Keyword {
        name: "bookmarks",
        description: "Named pointers to revisions (similar to Git's branches)",
        content: include_str!(concat!("../../", env!("JJ_DOCS_DIR"), "bookmarks.md")),
    },
    Keyword {
        name: "config",
        description: "How and where to set configuration options",
        content: include_str!(concat!("../../", env!("JJ_DOCS_DIR"), "config.md")),
    },
    Keyword {
        name: "filesets",
        description: "A functional language for selecting a set of files",
        content: include_str!(concat!("../../", env!("JJ_DOCS_DIR"), "filesets.md")),
    },
    Keyword {
        name: "glossary",
        description: "Definitions of various terms",
        content: include_str!(concat!("../../", env!("JJ_DOCS_DIR"), "glossary.md")),
    },
    Keyword {
        name: "revsets",
        description: "A functional language for selecting a set of revision",
        content: include_str!(concat!("../../", env!("JJ_DOCS_DIR"), "revsets.md")),
    },
    Keyword {
        name: "templates",
        description: "A functional language to customize command output",
        content: include_str!(concat!("../../", env!("JJ_DOCS_DIR"), "templates.md")),
    },
    Keyword {
        name: "tutorial",
        description: "Show a tutorial to get started with jj",
        content: include_str!(concat!("../../", env!("JJ_DOCS_DIR"), "tutorial.md")),
    },
];

fn find_keyword(name: &str) -> Option<&Keyword> {
    KEYWORDS.iter().find(|keyword| keyword.name == name)
}

pub fn show_keyword_hint_after_help() -> StyledStr {
    let mut ret = StyledStr::new();
    writeln!(
        ret,
        "{} lists available keywords. Use {} to show help for one of these keywords.",
        "'jj help --help'".bold(),
        "'jj help -k'".bold(),
    )
    .unwrap();
    ret
}
