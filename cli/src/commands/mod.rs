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

mod abandon;
mod absorb;
mod backout;
#[cfg(feature = "bench")]
mod bench;
mod bookmark;
mod commit;
mod config;
mod debug;
mod describe;
mod diff;
mod diffedit;
mod duplicate;
mod edit;
mod evolog;
mod file;
mod fix;
#[cfg(feature = "git")]
mod git;
mod help;
mod interdiff;
mod log;
mod new;
mod next;
mod operation;
mod parallelize;
mod prev;
mod rebase;
mod resolve;
mod restore;
mod revert;
mod root;
mod run;
mod show;
mod sign;
mod simplify_parents;
mod sparse;
mod split;
mod squash;
mod status;
mod tag;
mod unsign;
mod util;
mod version;
mod workspace;

use std::fmt::Debug;

use clap::builder::styling::AnsiColor;
use clap::builder::styling::Style as ClapStyle;
use clap::builder::Styles;
use clap::CommandFactory as _;
use clap::FromArgMatches as _;
use clap::Subcommand as _;
use clap_complete::engine::SubcommandCandidates;
use crossterm::style::Color as CrosstermColor;
use jj_lib::config::ConfigGetError;
use jj_lib::config::StackedConfig;
use serde::de::Deserialize as _;
use serde::de::IntoDeserializer as _;
use tracing::instrument;

use crate::cli_util::Args;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::complete;
use crate::formatter::deserialize_color;
use crate::formatter::Style;
use crate::ui::Ui;

fn get_help_styles(ui: &Ui, config: &StackedConfig) -> Styles {
    let mut styles = Styles::styled();

    // Default styles
    let default_header = AnsiColor::Yellow.on_default().bold();
    let default_usage = AnsiColor::Yellow.on_default().bold();
    let default_literal = AnsiColor::Green.on_default().bold();
    let default_placeholder = AnsiColor::Green.on_default();

    // Helper function to convert our Style to clap's Style
    let to_clap_style = |style: &Style| -> Result<ClapStyle, String> {
        let mut clap_style = ClapStyle::new();

        // Set foreground color
        if let Some(fg) = &style.fg {
            clap_style = match fg {
                CrosstermColor::Black => clap_style.fg_color(Some(AnsiColor::Black.into())),
                CrosstermColor::DarkRed => clap_style.fg_color(Some(AnsiColor::Red.into())),
                CrosstermColor::DarkGreen => clap_style.fg_color(Some(AnsiColor::Green.into())),
                CrosstermColor::DarkYellow => clap_style.fg_color(Some(AnsiColor::Yellow.into())),
                CrosstermColor::DarkBlue => clap_style.fg_color(Some(AnsiColor::Blue.into())),
                CrosstermColor::DarkMagenta => clap_style.fg_color(Some(AnsiColor::Magenta.into())),
                CrosstermColor::DarkCyan => clap_style.fg_color(Some(AnsiColor::Cyan.into())),
                CrosstermColor::Grey => clap_style.fg_color(Some(AnsiColor::White.into())),
                CrosstermColor::DarkGrey => {
                    clap_style.fg_color(Some(AnsiColor::BrightBlack.into()))
                }
                CrosstermColor::Red => clap_style.fg_color(Some(AnsiColor::BrightRed.into())),
                CrosstermColor::Green => clap_style.fg_color(Some(AnsiColor::BrightGreen.into())),
                CrosstermColor::Yellow => clap_style.fg_color(Some(AnsiColor::BrightYellow.into())),
                CrosstermColor::Blue => clap_style.fg_color(Some(AnsiColor::BrightBlue.into())),
                CrosstermColor::Magenta => {
                    clap_style.fg_color(Some(AnsiColor::BrightMagenta.into()))
                }
                CrosstermColor::Cyan => clap_style.fg_color(Some(AnsiColor::BrightCyan.into())),
                CrosstermColor::White => clap_style.fg_color(Some(AnsiColor::BrightWhite.into())),
                CrosstermColor::Rgb { r, g, b } => {
                    return Err(format!(
                        "Hex color #{r:02x}{g:02x}{b:02x} is not supported for help text. Only \
                         ANSI colors are supported."
                    ));
                }
                CrosstermColor::AnsiValue(n) => {
                    return Err(format!(
                        "ANSI 256 color (ansi-color-{n}) is not supported for help text. Only \
                         ANSI colors are supported."
                    ));
                }
                _ => clap_style,
            };
        }

        // Set attributes
        if style.bold == Some(true) {
            clap_style = clap_style.bold();
        }
        if style.italic == Some(true) {
            clap_style = clap_style.italic();
        }
        if style.underline == Some(true) {
            clap_style = clap_style.underline();
        }

        Ok(clap_style)
    };

    // Helper to parse a style from config
    let parse_style_config = |key: &str| -> Result<Style, ConfigGetError> {
        config.get_value_with(["colors", key], |value| {
            if value.is_str() {
                Ok(Style {
                    fg: Some(deserialize_color(value.into_deserializer())?),
                    bg: None,
                    bold: None,
                    italic: None,
                    underline: None,
                    reverse: None,
                })
            } else if value.is_inline_table() {
                Style::deserialize(value.into_deserializer())
            } else {
                Err(serde::de::Error::custom(format!(
                    "invalid type: {}, expected a color name or a table of styles",
                    value.type_name()
                )))
            }
        })
    };

    // Helper to apply a style or fall back to default
    let apply_style = |styles: Styles,
                       config_key: &str,
                       default: ClapStyle,
                       setter: fn(Styles, ClapStyle) -> Styles|
     -> Styles {
        let style = if let Ok(style) = parse_style_config(config_key) {
            match to_clap_style(&style) {
                Ok(clap_style) => clap_style,
                Err(err) => {
                    _ = writeln!(ui.warning_default(), "{err}");
                    default
                }
            }
        } else {
            default
        };
        setter(styles, style)
    };

    styles = apply_style(styles, "help heading", default_header, Styles::header);
    styles = apply_style(styles, "help usage", default_usage, Styles::usage);
    styles = apply_style(styles, "help literal", default_literal, Styles::literal);
    styles = apply_style(
        styles,
        "help placeholder",
        default_placeholder,
        Styles::placeholder,
    );

    styles
}

#[derive(clap::Parser, Clone, Debug)]
#[command(disable_help_subcommand = true)]
#[command(after_long_help = help::show_keyword_hint_after_help())]
#[command(add = SubcommandCandidates::new(complete::aliases))]
enum Command {
    Abandon(abandon::AbandonArgs),
    Absorb(absorb::AbsorbArgs),
    // TODO: Remove in jj 0.34+
    Backout(backout::BackoutArgs),
    #[cfg(feature = "bench")]
    #[command(subcommand)]
    Bench(bench::BenchCommand),
    #[command(subcommand)]
    Bookmark(bookmark::BookmarkCommand),
    Commit(commit::CommitArgs),
    #[command(subcommand)]
    Config(config::ConfigCommand),
    #[command(subcommand)]
    Debug(debug::DebugCommand),
    Describe(describe::DescribeArgs),
    Diff(diff::DiffArgs),
    Diffedit(diffedit::DiffeditArgs),
    Duplicate(duplicate::DuplicateArgs),
    Edit(edit::EditArgs),
    #[command(alias = "obslog", visible_alias = "evolution-log")]
    Evolog(evolog::EvologArgs),
    #[command(subcommand)]
    File(file::FileCommand),
    Fix(fix::FixArgs),
    #[cfg(feature = "git")]
    #[command(subcommand)]
    Git(git::GitCommand),
    Help(help::HelpArgs),
    Interdiff(interdiff::InterdiffArgs),
    Log(log::LogArgs),
    New(new::NewArgs),
    Next(next::NextArgs),
    #[command(subcommand)]
    #[command(visible_alias = "op")]
    Operation(operation::OperationCommand),
    Parallelize(parallelize::ParallelizeArgs),
    Prev(prev::PrevArgs),
    Rebase(rebase::RebaseArgs),
    Resolve(resolve::ResolveArgs),
    Restore(restore::RestoreArgs),
    Revert(revert::RevertArgs),
    Root(root::RootArgs),
    #[command(hide = true)]
    // TODO: Flesh out.
    Run(run::RunArgs),
    Show(show::ShowArgs),
    Sign(sign::SignArgs),
    SimplifyParents(simplify_parents::SimplifyParentsArgs),
    #[command(subcommand)]
    Sparse(sparse::SparseCommand),
    Split(split::SplitArgs),
    Squash(squash::SquashArgs),
    Status(status::StatusArgs),
    #[command(subcommand)]
    Tag(tag::TagCommand),
    /// Undo an operation (shortcut for `jj op undo`)
    Undo(operation::undo::OperationUndoArgs),
    Unsign(unsign::UnsignArgs),
    #[command(subcommand)]
    Util(util::UtilCommand),
    Version(version::VersionArgs),
    #[command(subcommand)]
    Workspace(workspace::WorkspaceCommand),
}

pub fn default_app() -> clap::Command {
    Command::augment_subcommands(Args::command())
}

pub fn app_with_styles(app: clap::Command, ui: &Ui, config: &StackedConfig) -> clap::Command {
    let styles = get_help_styles(ui, config);
    app.styles(styles)
}

#[instrument(skip_all)]
pub fn run_command(ui: &mut Ui, command_helper: &CommandHelper) -> Result<(), CommandError> {
    let subcommand = Command::from_arg_matches(command_helper.matches()).unwrap();
    match &subcommand {
        Command::Abandon(args) => abandon::cmd_abandon(ui, command_helper, args),
        Command::Absorb(args) => absorb::cmd_absorb(ui, command_helper, args),
        Command::Backout(args) => backout::cmd_backout(ui, command_helper, args),
        #[cfg(feature = "bench")]
        Command::Bench(args) => bench::cmd_bench(ui, command_helper, args),
        Command::Bookmark(args) => bookmark::cmd_bookmark(ui, command_helper, args),
        Command::Commit(args) => commit::cmd_commit(ui, command_helper, args),
        Command::Config(args) => config::cmd_config(ui, command_helper, args),
        Command::Debug(args) => debug::cmd_debug(ui, command_helper, args),
        Command::Describe(args) => describe::cmd_describe(ui, command_helper, args),
        Command::Diff(args) => diff::cmd_diff(ui, command_helper, args),
        Command::Diffedit(args) => diffedit::cmd_diffedit(ui, command_helper, args),
        Command::Duplicate(args) => duplicate::cmd_duplicate(ui, command_helper, args),
        Command::Edit(args) => edit::cmd_edit(ui, command_helper, args),
        Command::File(args) => file::cmd_file(ui, command_helper, args),
        Command::Fix(args) => fix::cmd_fix(ui, command_helper, args),
        #[cfg(feature = "git")]
        Command::Git(args) => git::cmd_git(ui, command_helper, args),
        Command::Help(args) => help::cmd_help(ui, command_helper, args),
        Command::Interdiff(args) => interdiff::cmd_interdiff(ui, command_helper, args),
        Command::Log(args) => log::cmd_log(ui, command_helper, args),
        Command::New(args) => new::cmd_new(ui, command_helper, args),
        Command::Next(args) => next::cmd_next(ui, command_helper, args),
        Command::Evolog(args) => evolog::cmd_evolog(ui, command_helper, args),
        Command::Operation(args) => operation::cmd_operation(ui, command_helper, args),
        Command::Parallelize(args) => parallelize::cmd_parallelize(ui, command_helper, args),
        Command::Prev(args) => prev::cmd_prev(ui, command_helper, args),
        Command::Rebase(args) => rebase::cmd_rebase(ui, command_helper, args),
        Command::Resolve(args) => resolve::cmd_resolve(ui, command_helper, args),
        Command::Restore(args) => restore::cmd_restore(ui, command_helper, args),
        Command::Revert(args) => revert::cmd_revert(ui, command_helper, args),
        Command::Root(args) => root::cmd_root(ui, command_helper, args),
        Command::Run(args) => run::cmd_run(ui, command_helper, args),
        Command::SimplifyParents(args) => {
            simplify_parents::cmd_simplify_parents(ui, command_helper, args)
        }
        Command::Show(args) => show::cmd_show(ui, command_helper, args),
        Command::Sign(args) => sign::cmd_sign(ui, command_helper, args),
        Command::Sparse(args) => sparse::cmd_sparse(ui, command_helper, args),
        Command::Split(args) => split::cmd_split(ui, command_helper, args),
        Command::Squash(args) => squash::cmd_squash(ui, command_helper, args),
        Command::Status(args) => status::cmd_status(ui, command_helper, args),
        Command::Tag(args) => tag::cmd_tag(ui, command_helper, args),
        Command::Undo(args) => operation::undo::cmd_op_undo(ui, command_helper, args),
        Command::Unsign(args) => unsign::cmd_unsign(ui, command_helper, args),
        Command::Util(args) => util::cmd_util(ui, command_helper, args),
        Command::Version(args) => version::cmd_version(ui, command_helper, args),
        Command::Workspace(args) => workspace::cmd_workspace(ui, command_helper, args),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_app() {
        default_app().debug_assert();
    }
}
