use std::ffi::OsString;
use std::io::Write as _;

use clap_complete::CompletionCandidate;
use clap_complete::engine::complete;
use serde::Serialize;

use crate::cli_util::CommandHelper;
use crate::cli_util::expand_args_for_completion;
use crate::cli_util::hide_short_subcommand_aliases;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::ui::Ui;

/// Compute machine-readable completion candidates
///
/// This command is intended for editors and other tools which need structured
/// completion results instead of shell-specific text output.
///
/// Provide the full command line after `--`, including the leading binary name
/// (usually `jj`). `--index` identifies the argument to complete. To complete
/// after whitespace, append an empty string as the final argument and point the
/// index at that empty argument.
///
/// Example:
///
/// ```shell
/// jj util complete --index 3 -- jj diff --from ""
/// ```
#[derive(clap::Args, Clone, Debug)]
#[command(verbatim_doc_comment)]
pub struct UtilCompleteArgs {
    /// Zero-based index of the argument being completed
    ///
    /// Defaults to the last provided argument.
    #[arg(long)]
    index: Option<usize>,

    /// Completion output format
    #[arg(long, default_value = "json")]
    format: MachineCompletionFormat,

    /// Full command line to complete, including the leading `jj` binary name
    #[arg(required = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, Eq, PartialEq)]
enum MachineCompletionFormat {
    Json,
}

#[derive(Serialize)]
struct MachineCompletionCandidate {
    value: String,
    help: Option<String>,
    id: Option<String>,
    tag: Option<String>,
    display_order: Option<usize>,
    hidden: bool,
}

impl From<CompletionCandidate> for MachineCompletionCandidate {
    fn from(candidate: CompletionCandidate) -> Self {
        Self {
            value: candidate.get_value().to_string_lossy().into_owned(),
            help: candidate.get_help().map(ToString::to_string),
            id: candidate.get_id().cloned(),
            tag: candidate.get_tag().map(ToString::to_string),
            display_order: candidate.get_display_order(),
            hidden: candidate.is_hide_set(),
        }
    }
}

pub async fn cmd_util_complete(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &UtilCompleteArgs,
) -> Result<(), CommandError> {
    let complete_index = args
        .index
        .unwrap_or_else(|| args.args.len().saturating_sub(1));
    if args.args.is_empty() {
        return Err(user_error("No command line provided to complete"));
    }
    if complete_index >= args.args.len() {
        return Err(user_error(format!(
            "Completion index {} is out of bounds for {} argument(s)",
            complete_index,
            args.args.len()
        )));
    }

    let left_of_cursor = args.args[..=complete_index]
        .iter()
        .cloned()
        .map(OsString::from);
    let mut resolved_args = expand_args_for_completion(
        &Ui::null(),
        command.app(),
        left_of_cursor,
        command.settings().config(),
    )?;
    let resolved_index = resolved_args.len().saturating_sub(1);
    resolved_args.extend(args.args[complete_index + 1..].iter().cloned());

    let mut app = command.app().clone();
    hide_short_subcommand_aliases(&mut app);
    app = app.allow_external_subcommands(true);

    let candidates = complete(
        &mut app,
        resolved_args.into_iter().map(OsString::from).collect(),
        resolved_index,
        Some(command.cwd()),
    )
    .map_err(user_error)?;
    let candidates: Vec<_> = candidates
        .into_iter()
        .map(MachineCompletionCandidate::from)
        .collect();

    match args.format {
        MachineCompletionFormat::Json => {
            serde_json::to_writer_pretty(ui.stdout(), &candidates).map_err(user_error)?;
            ui.stdout().write_all(b"\n")?;
        }
    }
    Ok(())
}
