// Copyright 2022-2024 The Jujutsu Authors
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

use std::error;
use std::error::Error as _;
use std::io;
use std::io::Write as _;
use std::iter;

use jj_lib::dsl_util::Diagnostics;
use jj_lib::formatter::FormatRecorder;
use jj_lib::formatter::Formatter;
use jj_lib::formatter::FormatterExt as _;
use jj_lib::revset;
use jj_lib::user_error::ErrorHint;
use jj_lib::user_error::UserError;
pub use jj_lib::user_error::UserErrorKind as CommandErrorKind;
use jj_lib::user_error::format_similarity_hint;

use crate::description_util::ParseBulkEditMessageError;
use crate::description_util::TempTextEditError;
use crate::description_util::TextEditError;
use crate::diff_util::DiffRenderError;
use crate::merge_tools::ConflictResolveError;
use crate::merge_tools::DiffEditError;
use crate::merge_tools::MergeToolConfigError;
use crate::merge_tools::MergeToolPartialResolutionError;
use crate::revset_util::BookmarkNameParseError;
use crate::revset_util::TagNameParseError;
use crate::revset_util::UserRevsetEvaluationError;
use crate::template_parser::TemplateParseError;
use crate::template_parser::TemplateParseErrorKind;
use crate::ui::Ui;

#[derive(Clone, Debug)]
pub struct CommandError(pub UserError);

impl CommandError {
    /// Returns error with the given plain-text `hint` attached.
    pub fn hinted(mut self, hint: impl Into<String>) -> Self {
        self.0.add_hint(hint);
        self
    }

    /// Appends plain-text `hint` to the error.
    pub fn add_hint(&mut self, hint: impl Into<String>) {
        self.0.add_hint(hint);
    }

    /// Appends formatted `hint` to the error.
    pub fn add_formatted_hint(&mut self, hint: FormatRecorder) {
        self.0.add_formatted_hint(hint);
    }

    /// Constructs formatted hint and appends it to the error.
    pub fn add_formatted_hint_with(
        &mut self,
        write: impl FnOnce(&mut dyn Formatter) -> io::Result<()>,
    ) {
        self.0.add_formatted_hint_with(write);
    }

    /// Appends 0 or more plain-text `hints` to the error.
    pub fn extend_hints(&mut self, hints: impl IntoIterator<Item = String>) {
        self.0.extend_hints(hints);
    }
}

impl<T> From<T> for CommandError
where
    UserError: From<T>,
{
    fn from(value: T) -> Self {
        Self(value.into())
    }
}

pub fn user_error(err: impl Into<Box<dyn error::Error + Send + Sync>>) -> CommandError {
    CommandError(UserError::new(CommandErrorKind::User, err))
}

pub fn user_error_with_message(
    message: impl Into<String>,
    source: impl Into<Box<dyn error::Error + Send + Sync>>,
) -> CommandError {
    CommandError(UserError::with_message(
        CommandErrorKind::User,
        message,
        source,
    ))
}

pub fn config_error(err: impl Into<Box<dyn error::Error + Send + Sync>>) -> CommandError {
    CommandError(UserError::new(CommandErrorKind::Config, err))
}

pub fn config_error_with_message(
    message: impl Into<String>,
    source: impl Into<Box<dyn error::Error + Send + Sync>>,
) -> CommandError {
    CommandError(UserError::with_message(
        CommandErrorKind::Config,
        message,
        source,
    ))
}

pub fn internal_error(err: impl Into<Box<dyn error::Error + Send + Sync>>) -> CommandError {
    CommandError(UserError::new(CommandErrorKind::Internal, err))
}

pub fn internal_error_with_message(
    message: impl Into<String>,
    source: impl Into<Box<dyn error::Error + Send + Sync>>,
) -> CommandError {
    CommandError(UserError::with_message(
        CommandErrorKind::Internal,
        message,
        source,
    ))
}

pub fn cli_error(err: impl Into<Box<dyn error::Error + Send + Sync>>) -> CommandError {
    CommandError(UserError::new(CommandErrorKind::Cli, err))
}

pub fn cli_error_with_message(
    message: impl Into<String>,
    source: impl Into<Box<dyn error::Error + Send + Sync>>,
) -> CommandError {
    CommandError(UserError::with_message(
        CommandErrorKind::Cli,
        message,
        source,
    ))
}

pub fn clap_error(err: clap::Error) -> CommandError {
    let hint = find_source_parse_error_hint(&err);
    let mut cmd_err = cli_error(err);
    cmd_err.extend_hints(hint);
    cmd_err
}

impl From<DiffEditError> for CommandError {
    fn from(err: DiffEditError) -> Self {
        user_error_with_message("Failed to edit diff", err)
    }
}

impl From<DiffRenderError> for CommandError {
    fn from(err: DiffRenderError) -> Self {
        match err {
            DiffRenderError::DiffGenerate(_) => user_error(err),
            DiffRenderError::Backend(err) => err.into(),
            DiffRenderError::AccessDenied { .. } => user_error(err),
            DiffRenderError::InvalidRepoPath(_) => user_error(err),
            DiffRenderError::Io(err) => err.into(),
        }
    }
}

impl From<ConflictResolveError> for CommandError {
    fn from(err: ConflictResolveError) -> Self {
        match err {
            ConflictResolveError::Backend(err) => err.into(),
            ConflictResolveError::Io(err) => err.into(),
            _ => {
                let hint = match &err {
                    ConflictResolveError::ConflictTooComplicated { .. } => {
                        Some("Edit the conflict markers manually to resolve this.".to_owned())
                    }
                    ConflictResolveError::ExecutableConflict { .. } => {
                        Some("Use `jj file chmod` to update the executable bit.".to_owned())
                    }
                    _ => None,
                };
                let mut cmd_err = user_error_with_message("Failed to resolve conflicts", err);
                cmd_err.extend_hints(hint);
                cmd_err
            }
        }
    }
}

impl From<MergeToolPartialResolutionError> for CommandError {
    fn from(err: MergeToolPartialResolutionError) -> Self {
        user_error(err)
    }
}

impl From<MergeToolConfigError> for CommandError {
    fn from(err: MergeToolConfigError) -> Self {
        match &err {
            MergeToolConfigError::MergeArgsNotConfigured { tool_name } => {
                let tool_name = tool_name.clone();
                user_error(err).hinted(format!(
                    "To use `{tool_name}` as a merge tool, the config \
                     `merge-tools.{tool_name}.merge-args` must be defined (see docs for details)"
                ))
            }
            _ => user_error_with_message("Failed to load tool configuration", err),
        }
    }
}

impl From<TextEditError> for CommandError {
    fn from(err: TextEditError) -> Self {
        user_error(err)
    }
}

impl From<TempTextEditError> for CommandError {
    fn from(err: TempTextEditError) -> Self {
        let hint = err.path.as_ref().map(|path| {
            let name = err.name.as_deref().unwrap_or("file");
            format!("Edited {name} is left in {path}", path = path.display())
        });
        let mut cmd_err = user_error(err);
        cmd_err.extend_hints(hint);
        cmd_err
    }
}

impl From<UserRevsetEvaluationError> for CommandError {
    fn from(err: UserRevsetEvaluationError) -> Self {
        match err {
            UserRevsetEvaluationError::Resolution(err) => err.into(),
            UserRevsetEvaluationError::Evaluation(err) => err.into(),
        }
    }
}

impl From<TemplateParseError> for CommandError {
    fn from(err: TemplateParseError) -> Self {
        let hint = template_parse_error_hint(&err);
        let mut cmd_err =
            user_error_with_message(format!("Failed to parse template: {}", err.kind()), err);
        cmd_err.extend_hints(hint);
        cmd_err
    }
}

impl From<ParseBulkEditMessageError> for CommandError {
    fn from(err: ParseBulkEditMessageError) -> Self {
        user_error(err)
    }
}

/// handles cli-level error, falling back to lib-level types if the source is
/// more general
fn find_source_parse_error_hint(err: &dyn error::Error) -> Option<String> {
    let source = err.source()?;
    if let Some(source) = source.downcast_ref() {
        bookmark_name_parse_error_hint(source)
    } else if let Some(UserRevsetEvaluationError::Resolution(source)) = source.downcast_ref() {
        // TODO: propagate all hints?
        jj_lib::user_error::revset_resolution_error_hints(source)
            .into_iter()
            .next()
    } else if let Some(source) = source.downcast_ref() {
        tag_name_parse_error_hint(source)
    } else if let Some(source) = source.downcast_ref() {
        template_parse_error_hint(source)
    } else {
        jj_lib::user_error::find_source_parse_error_hint(err)
    }
}

const REVSET_SYMBOL_HINT: &str = "See https://docs.jj-vcs.dev/latest/revsets/ or use `jj help -k \
                                  revsets` for how to quote symbols.";

fn bookmark_name_parse_error_hint(err: &BookmarkNameParseError) -> Option<String> {
    use revset::ExpressionKind;
    match revset::parse_program(&err.input).map(|node| node.kind) {
        Ok(ExpressionKind::RemoteSymbol(symbol)) => Some(format!(
            "Looks like remote bookmark. Run `jj bookmark track {name} --remote={remote}` to \
             track it.",
            name = symbol.name.as_symbol(),
            remote = symbol.remote.as_symbol()
        )),
        _ => Some(REVSET_SYMBOL_HINT.to_owned()),
    }
}

fn tag_name_parse_error_hint(_: &TagNameParseError) -> Option<String> {
    Some(REVSET_SYMBOL_HINT.to_owned())
}

fn template_parse_error_hint(err: &TemplateParseError) -> Option<String> {
    // Only for the bottom error, which is usually the root cause
    let bottom_err = iter::successors(Some(err), |e| e.origin()).last().unwrap();
    match bottom_err.kind() {
        TemplateParseErrorKind::NoSuchKeyword { candidates, .. }
        | TemplateParseErrorKind::NoSuchFunction { candidates, .. }
        | TemplateParseErrorKind::NoSuchMethod { candidates, .. } => {
            format_similarity_hint(candidates)
        }
        TemplateParseErrorKind::InvalidArguments { .. } | TemplateParseErrorKind::Expression(_) => {
            find_source_parse_error_hint(bottom_err)
        }
        _ => None,
    }
}

const BROKEN_PIPE_EXIT_CODE: u8 = 3;

pub(crate) fn handle_command_result(ui: &mut Ui, result: Result<(), CommandError>) -> u8 {
    try_handle_command_result(ui, result).unwrap_or(BROKEN_PIPE_EXIT_CODE)
}

fn try_handle_command_result(ui: &mut Ui, result: Result<(), CommandError>) -> io::Result<u8> {
    let Err(cmd_err) = &result else {
        return Ok(0);
    };
    let err = &cmd_err.0.error;
    let hints = &cmd_err.0.hints;
    match cmd_err.0.kind {
        CommandErrorKind::User => {
            print_error(ui, "Error: ", err, hints)?;
            Ok(1)
        }
        CommandErrorKind::Config => {
            print_error(ui, "Config error: ", err, hints)?;
            writeln!(
                ui.stderr_formatter().labeled("hint"),
                "For help, see https://docs.jj-vcs.dev/latest/config/ or use `jj help -k config`."
            )?;
            Ok(1)
        }
        CommandErrorKind::Cli => {
            if let Some(err) = err.downcast_ref::<clap::Error>() {
                handle_clap_error(ui, err, hints)
            } else {
                print_error(ui, "Error: ", err, hints)?;
                Ok(2)
            }
        }
        CommandErrorKind::BrokenPipe => {
            // A broken pipe is not an error, but a signal to exit gracefully.
            Ok(BROKEN_PIPE_EXIT_CODE)
        }
        CommandErrorKind::Internal => {
            print_error(ui, "Internal error: ", err, hints)?;
            Ok(255)
        }
    }
}

fn print_error(
    ui: &Ui,
    heading: &str,
    err: &dyn error::Error,
    hints: &[ErrorHint],
) -> io::Result<()> {
    writeln!(ui.error_with_heading(heading), "{err}")?;
    print_error_sources(ui, err.source())?;
    print_error_hints(ui, hints)?;
    Ok(())
}

/// Prints error sources one by one from the given `source` inclusive.
pub fn print_error_sources(ui: &Ui, source: Option<&dyn error::Error>) -> io::Result<()> {
    let Some(err) = source else {
        return Ok(());
    };
    let mut formatter = ui.stderr_formatter().into_labeled("error_source");
    if err.source().is_none() {
        write!(formatter.labeled("heading"), "Caused by: ")?;
        writeln!(formatter, "{err}")?;
    } else {
        writeln!(formatter.labeled("heading"), "Caused by:")?;
        for (i, err) in iter::successors(Some(err), |&err| err.source()).enumerate() {
            write!(formatter.labeled("heading"), "{}: ", i + 1)?;
            writeln!(formatter, "{err}")?;
        }
    }
    Ok(())
}

fn print_error_hints(ui: &Ui, hints: &[ErrorHint]) -> io::Result<()> {
    let mut formatter = ui.stderr_formatter().into_labeled("hint");
    for hint in hints {
        write!(formatter.labeled("heading"), "Hint: ")?;
        match hint {
            ErrorHint::PlainText(message) => {
                writeln!(formatter, "{message}")?;
            }
            ErrorHint::Formatted(recorded) => {
                recorded.replay(formatter.as_mut())?;
                // Formatted hint is usually multi-line text, and it's
                // convenient if trailing "\n" doesn't have to be omitted.
                if !recorded.data().ends_with(b"\n") {
                    writeln!(formatter)?;
                }
            }
        }
    }
    Ok(())
}

fn handle_clap_error(ui: &mut Ui, err: &clap::Error, hints: &[ErrorHint]) -> io::Result<u8> {
    let clap_str = if ui.color() {
        err.render().ansi().to_string()
    } else {
        err.render().to_string()
    };

    match err.kind() {
        clap::error::ErrorKind::DisplayHelp
        | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => ui.request_pager(),
        _ => {}
    }
    // Definitions for exit codes and streams come from
    // https://github.com/clap-rs/clap/blob/master/src/error/mod.rs
    match err.kind() {
        clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
            write!(ui.stdout(), "{clap_str}")?;
            return Ok(0);
        }
        _ => {}
    }
    write!(ui.stderr(), "{clap_str}")?;
    // Skip the first source error, which should be printed inline.
    print_error_sources(ui, err.source().and_then(|err| err.source()))?;
    print_error_hints(ui, hints)?;
    Ok(2)
}

/// Prints diagnostic messages emitted during parsing.
pub fn print_parse_diagnostics<T: error::Error>(
    ui: &Ui,
    context_message: &str,
    diagnostics: &Diagnostics<T>,
) -> io::Result<()> {
    for diag in diagnostics {
        writeln!(ui.warning_default(), "{context_message}")?;
        for err in iter::successors(Some(diag as &dyn error::Error), |&err| err.source()) {
            writeln!(ui.stderr(), "{err}")?;
        }
        // If we add support for multiple error diagnostics, we might have to do
        // find_source_parse_error_hint() and print it here.
    }
    Ok(())
}
