// Copyright 2026 The Jujutsu Authors
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

//! Command-line argument definitions and parsing, including expansion of
//! aliases and the default command.

use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::io::Write as _;
use std::ops::Range;

use clap::ArgAction;
use clap::ArgMatches;
use clap::Command;
use clap::FromArgMatches as _;
use clap_complete::ArgValueCandidates;
use clap_complete::ArgValueCompleter;
use itertools::Itertools as _;
use jj_lib::config::ConfigGetResultExt as _;
use jj_lib::config::ConfigLayer;
use jj_lib::config::ConfigSource;
use jj_lib::config::ConfigValue;
use jj_lib::config::StackedConfig;

use crate::command_error::CommandError;
use crate::command_error::cli_error;
use crate::command_error::print_error_sources;
use crate::command_error::user_error;
use crate::complete;
use crate::config::ConfigArgKind;
use crate::config::parse_config_args;
use crate::ui::ColorChoice;
use crate::ui::Ui;

/// Jujutsu (An experimental VCS)
///
/// To get started, see the tutorial at [`jj help -k tutorial`][tutorial].
///
/// [tutorial]: https://docs.jj-vcs.dev/latest/tutorial/
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

    /// Run the command as usual but don't integrate any operations
    ///
    /// When this option is given, the operations will still be created as usual
    /// but they will not be integrated to the operation log. The working copy
    /// will also not be updated.
    ///
    /// The command will print the resulting operation ID. You can pass that to
    /// e.g. `jj --at-op` to inspect the resulting repo state, or you can pass
    /// it to `jj op restore` to restore the repo to that state. You can also
    /// pass the ID to `jj op integrate` to integrate the operation.
    ///
    /// Note that this does *not* prevent side effects outside the repo. For
    /// example, `jj git push --no-integrate-operation` will still perform the
    /// push.
    #[arg(long, global = true)]
    pub no_integrate_operation: bool,

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
    #[arg(long, visible_alias = "at-op", global = true)]
    #[arg(add = ArgValueCandidates::new(complete::operations))]
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
    #[arg(long, value_name = "NAME=VALUE", global = true)]
    #[arg(add = ArgValueCompleter::new(complete::leaf_config_key_value))]
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

    pub(crate) fn has_config_args(&self) -> bool {
        !self.config.is_empty() || !self.config_file.is_empty()
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
        // Try loading as either a string or an array.
        // Only use `.optional()?` on the latter call to `config.get` - if it
        // was used on both, the type error from the first check would return
        // early.
        let default_command = if let Ok(string) = config.get::<String>("ui.default-command") {
            // Warn when the user has misconfigured their default command as
            // "log -n 5" instead of ["log", "-n", "5"]
            if string.contains(' ') {
                let elements: ConfigValue = string.split_whitespace().collect();
                writeln!(
                    ui.warning_default(),
                    "To include flags/arguments in `ui.default-command`, use an array instead of \
                     a string: `ui.default-command = {elements}`"
                )?;
            }

            vec![string]
        } else if let Some(array) = config.get::<Vec<String>>("ui.default-command").optional()? {
            array
        } else {
            writeln!(
                ui.hint_default(),
                "Use `jj -h` for a list of available commands."
            )?;
            writeln!(
                ui.hint_no_heading(),
                "Run `jj config set --user ui.default-command log` to disable this message."
            )?;

            vec!["log".to_string()]
        };

        // Insert the default command directly after the path to the binary.
        string_args.splice(1..1, default_command);
    }
    Ok(string_args)
}

/// Loads alias definitions from the `aliases` table.
fn load_aliases<'config>(
    ui: &Ui,
    config: &'config StackedConfig,
    app: &Command,
) -> Result<HashMap<&'config str, Vec<String>>, CommandError> {
    let mut defined_aliases = HashMap::new();
    for alias in config.table_keys("aliases") {
        if config
            .get::<bool>(["aliases", alias, "enabled"])
            .optional()?
            .is_some_and(|enabled| !enabled)
        {
            continue;
        }
        let definition: Result<Vec<String>, _> = match config.get(["aliases", alias]) {
            Ok(definition) => Ok(definition),
            Err(original_error) => match config.get(["aliases", alias, "definition"]) {
                Ok(definition) => Ok(definition),
                Err(_) => Err(original_error),
            },
        };
        match definition {
            Ok(definition) => {
                defined_aliases.insert(alias, definition);
            }
            Err(err) => {
                writeln!(
                    ui.warning_default(),
                    "Failed to load `aliases.{alias}`: Expected a string list or a table with a \
                     `definition`"
                )?;
                print_error_sources(ui, Some(&err))?;
            }
        }
    }
    // Print warnings for aliases that match real commands and remove them from
    // the mapping.
    let mut real_commands = HashSet::new();
    for command in app.get_subcommands() {
        real_commands.insert(command.get_name());
        for alias in command.get_all_aliases() {
            real_commands.insert(alias);
        }
    }
    for alias in defined_aliases
        .extract_if(|a, _| real_commands.contains(*a))
        .map(|(a, _)| a)
        .sorted()
    {
        writeln!(
            ui.warning_default(),
            "Cannot define an alias that overrides the built-in command '{alias}'."
        )?;
    }
    Ok(defined_aliases)
}

fn resolve_aliases(
    app: &Command,
    defined_aliases: &HashMap<&str, Vec<String>>,
    mut string_args: Vec<String>,
) -> Result<Vec<String>, CommandError> {
    let mut recursion_check_stack: Vec<(&str, Range<usize>)> = Vec::new();

    loop {
        let app_clone = app.clone().allow_external_subcommands(true);
        let matches = app_clone.try_get_matches_from(&string_args).ok();
        let Some((command_name, submatches)) = matches.as_ref().and_then(|m| m.subcommand()) else {
            // No more alias commands, or hit unknown option
            return Ok(string_args);
        };
        let alias_name = command_name.to_string();
        let alias_args = submatches
            .get_many::<OsString>("")
            .unwrap_or_default()
            .map(|arg| arg.to_str().unwrap().to_string())
            .collect_vec();
        let Some((&alias_name, alias_definition)) = defined_aliases.get_key_value(&*alias_name)
        else {
            // Not a real command and not an alias, so return what we've
            // resolved so far.
            return Ok(string_args);
        };
        let alias_position = string_args.len() - 1 - alias_args.len();

        // recursion check
        while let Some((_, check_range)) = recursion_check_stack.last() {
            if check_range.contains(&alias_position) {
                // The tracked chain of alias expansions produced the current
                // alias. Check for recursion.
                if recursion_check_stack.iter().any(|&(a, _)| a == alias_name) {
                    return Err(user_error(format!(
                        "Recursive alias definition involving `{alias_name}`"
                    )));
                }
                break;
            }
            // Last tracked alias did not produce the currently expanding alias.
            // Remove it from stack and fixup the range of the next one.
            let check_range = check_range.clone();
            recursion_check_stack.pop();
            if let Some((_, next_range)) = recursion_check_stack.last_mut() {
                // Increase next range by the length of the current one, minus
                // one to account for the removed alias name.
                next_range.end += check_range.end - check_range.start - 1;
            }
        }
        recursion_check_stack.push((
            alias_name,
            alias_position..(alias_position + alias_definition.len()),
        ));

        assert!(string_args.ends_with(&alias_args));
        string_args.truncate(alias_position);
        string_args.extend(alias_definition.clone());
        string_args.extend_from_slice(&alias_args);
    }
}

/// Parse args that must be interpreted early, e.g. before printing help.
pub(crate) fn parse_early_args(
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

pub fn expand_args(
    ui: &Ui,
    app: &Command,
    args_os: impl IntoIterator<Item = OsString>,
    config: &StackedConfig,
) -> Result<Vec<String>, CommandError> {
    let mut string_args = to_string_args(args_os)?;
    let aliases = load_aliases(ui, config, app)?;
    string_args = resolve_aliases(app, &aliases, string_args)?;
    string_args = resolve_default_command(ui, config, app, string_args)?;
    string_args = resolve_aliases(app, &aliases, string_args)?;
    Ok(string_args)
}

pub(crate) fn expand_args_for_completion(
    ui: &Ui,
    app: &Command,
    args_os: impl IntoIterator<Item = OsString>,
    config: &StackedConfig,
) -> Result<Vec<String>, CommandError> {
    let mut string_args = to_string_args(args_os)?;
    let aliases = load_aliases(ui, config, app)?;

    // Resolution of subcommand aliases must not consider the argument that is being
    // completed.
    let cursor_arg = string_args.pop();
    string_args = resolve_aliases(app, &aliases, string_args)?;
    string_args.extend(cursor_arg);

    // If a subcommand has been given, including the potentially incomplete argument
    // that is being completed, the default command is not resolved and the
    // completion candidates for the subcommand are prioritized.
    string_args = resolve_default_command(ui, config, app, string_args)?;

    let cursor_arg = string_args.pop();
    string_args = resolve_aliases(app, &aliases, string_args)?;
    string_args.extend(cursor_arg);

    Ok(string_args)
}

pub(crate) fn to_string_args(
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

pub(crate) fn parse_args(
    app: &Command,
    string_args: &[String],
) -> Result<(ArgMatches, Args), clap::Error> {
    let matches = app
        .clone()
        .arg_required_else_help(true)
        .subcommand_required(true)
        .try_get_matches_from(string_args)?;
    let args = Args::from_arg_matches(&matches).unwrap();
    Ok((matches, args))
}

pub(crate) fn command_name(mut matches: &ArgMatches) -> String {
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

// If -R or --config* is specified, check if the expanded arguments differ.
pub(crate) fn warn_if_args_mismatch(
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
