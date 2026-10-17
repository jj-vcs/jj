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

//! Utility for parsing and evaluating user-provided revset expressions.

use std::collections::HashMap;
use std::io;

use futures::StreamExt as _;
use futures::TryStreamExt as _;
use itertools::Itertools as _;
use jj_frontend::config_util;
use jj_frontend::config_util::RemoteSettingDiagnostics;
use jj_frontend::revset_util::RevsetExpressionEvaluator;
use jj_frontend::revset_util::UserRevsetEvaluationError;
use jj_lib::commit::Commit;
use jj_lib::ref_name::RefNameBuf;
use jj_lib::ref_name::RemoteName;
use jj_lib::ref_name::RemoteNameBuf;
use jj_lib::ref_name::RemoteRefSymbolBuf;
use jj_lib::revset;
use jj_lib::revset::RevsetDiagnostics;
use jj_lib::revset::RevsetParseError;
use jj_lib::settings::RemoteSettingsMap;
use jj_lib::str_util::StringExpression;
use jj_lib::str_util::StringMatcher;
use thiserror::Error;

use crate::command_error::CommandError;
use crate::command_error::print_parse_diagnostics;
use crate::command_error::revset_parse_error_hint;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::formatter::Formatter;
use crate::templater::TemplateRenderer;
use crate::ui::Ui;

/// Error when evaluating a revset into a single commit.
#[derive(Debug)]
pub enum RevsetEvaluationSizeError {
    /// The revset evaluated to no commits.
    Empty,
    /// The revset evaluated to multiple commits. The vector only contains a few
    /// commits (enough for an error message), and the bool indicates whether
    /// there were more; do not expect this to be the entire evaluated revset.
    Multiple(Vec<Commit>, bool),
    /// An error occurred during revset evaluation; size unknown.
    Other(UserRevsetEvaluationError),
}

impl RevsetEvaluationSizeError {
    pub fn to_command_error(
        self,
        revision_str: &str,
        commit_summary_template: &TemplateRenderer<'_, Commit>,
    ) -> CommandError {
        match self {
            Self::Empty => user_error(format!(
                "Revset `{revision_str}` didn't resolve to any revisions"
            )),
            Self::Multiple(commits, has_more) => format_multiple_revisions_error(
                revision_str,
                &commits,
                has_more,
                commit_summary_template,
            ),
            Self::Other(error) => error.into(),
        }
    }
}

pub(super) async fn evaluate_revset_to_single_commit(
    expression: &RevsetExpressionEvaluator<'_>,
) -> Result<Commit, RevsetEvaluationSizeError> {
    // The number of commits to pass to the error (will be shown in the error
    // message).
    let max_commits = 5;
    let mut commits: Vec<_> = expression
        .evaluate_to_commits()
        .map_err(RevsetEvaluationSizeError::Other)?
        .take(max_commits + 1)
        .try_collect()
        .await
        .map_err(UserRevsetEvaluationError::Evaluation)
        .map_err(RevsetEvaluationSizeError::Other)?;
    match commits.as_slice() {
        [commit] => Ok(commit.clone()),
        [] => Err(RevsetEvaluationSizeError::Empty),
        _ => {
            let has_more = commits.len() > max_commits;
            commits.truncate(max_commits);
            Err(RevsetEvaluationSizeError::Multiple(commits, has_more))
        }
    }
}

fn format_multiple_revisions_error(
    revision_str: &str,
    commits: &[Commit],
    elided: bool,
    template: &TemplateRenderer<'_, Commit>,
) -> CommandError {
    assert!(commits.len() >= 2);
    let mut cmd_err = user_error(format!(
        "Revset `{revision_str}` resolved to more than one revision"
    ));
    let write_commits_summary = |formatter: &mut dyn Formatter| {
        for commit in commits {
            write!(formatter, "  ")?;
            template.format(commit, formatter)?;
            writeln!(formatter)?;
        }
        if elided {
            writeln!(formatter, "  ...")?;
        }
        Ok(())
    };
    cmd_err.add_formatted_hint_with(|formatter| {
        writeln!(
            formatter,
            "The revset `{revision_str}` resolved to these revisions:"
        )?;
        write_commits_summary(formatter)
    });
    cmd_err
}

#[derive(Debug, Error)]
#[error("Failed to parse bookmark name: {}", source.kind())]
pub struct BookmarkNameParseError {
    pub input: String,
    pub source: RevsetParseError,
}

/// Parses bookmark name specified in revset syntax.
pub fn parse_bookmark_name(text: &str) -> Result<RefNameBuf, BookmarkNameParseError> {
    revset::parse_symbol(text)
        .map(Into::into)
        .map_err(|source| BookmarkNameParseError {
            input: text.to_owned(),
            source,
        })
}

#[derive(Debug, Error)]
#[error("Failed to parse tag name: {}", source.kind())]
pub struct TagNameParseError {
    pub source: RevsetParseError,
}

/// Parses tag name specified in revset syntax.
pub fn parse_tag_name(text: &str) -> Result<RefNameBuf, TagNameParseError> {
    revset::parse_symbol(text)
        .map(Into::into)
        .map_err(|source| TagNameParseError { source })
}

/// Parses bookmark/tag/remote name patterns and unions them all.
pub fn parse_union_name_patterns<I>(ui: &Ui, texts: I) -> Result<StringExpression, CommandError>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut diagnostics = RevsetDiagnostics::new();
    let expressions = texts
        .into_iter()
        .map(|text| revset::parse_string_expression(&mut diagnostics, text.as_ref()))
        .try_collect()
        .map_err(|err| {
            // From<RevsetParseError>, but with different message
            let hint = revset_parse_error_hint(&err);
            let message = format!("Failed to parse name pattern: {}", err.kind());
            let mut cmd_err = user_error_with_message(message, err);
            cmd_err.extend_hints(hint);
            cmd_err
        })?;
    print_parse_diagnostics(ui, "In name pattern", &diagnostics)?;
    Ok(StringExpression::union_all(expressions))
}

/// Parses bookmark/tag name patterns or remote symbols.
pub fn parse_name_patterns_or_remote_symbols<I>(
    ui: &Ui,
    texts: I,
) -> Result<(Vec<StringExpression>, Vec<RemoteRefSymbolBuf>), CommandError>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let wrap_err = |err| {
        // From<RevsetParseError>, but with different message
        let hint = revset_parse_error_hint(&err);
        let message = format!(
            "Failed to parse name pattern or remote symbol: {}",
            err.kind()
        );
        let mut cmd_err = user_error_with_message(message, err);
        cmd_err.extend_hints(hint);
        cmd_err
    };
    let mut diagnostics = RevsetDiagnostics::new();
    let mut name_expressions = Vec::new();
    let mut remote_symbols = Vec::new();
    for text in texts {
        let node = revset::parse_program(text.as_ref()).map_err(wrap_err)?;
        if let revset::ExpressionKind::RemoteSymbol(symbol) = node.kind {
            remote_symbols.push(symbol);
        } else {
            let expr =
                revset::expect_string_expression(&mut diagnostics, &node).map_err(wrap_err)?;
            name_expressions.push(expr);
        }
    }
    print_parse_diagnostics(ui, "In name pattern", &diagnostics)?;
    Ok((name_expressions, remote_symbols))
}

/// Parses the given `remotes.<name>.auto-track-bookmarks` settings into a map
/// of string matchers.
pub fn parse_remote_auto_track_bookmarks_map(
    ui: &Ui,
    remote_settings: &RemoteSettingsMap,
) -> Result<HashMap<RemoteNameBuf, StringMatcher>, CommandError> {
    let mut diagnostics = vec![];
    let result =
        config_util::parse_remote_auto_track_bookmarks_map(&mut diagnostics, remote_settings);
    print_remote_setting_diagnostics(ui, &diagnostics)?;
    Ok(result?)
}

/// Parses the given `remotes.<name>.auto-track-bookmarks` and
/// `remotes.<name>.auto-track-created-bookmarks` settings into a map of string
/// matchers. If both settings exist for the same remote, the union of the
/// settings will be matched.
pub fn parse_remote_auto_track_bookmarks_map_for_new_bookmarks(
    ui: &Ui,
    remote_settings: &RemoteSettingsMap,
) -> Result<HashMap<RemoteNameBuf, StringMatcher>, CommandError> {
    let mut matchers = HashMap::new();
    for (name, settings) in remote_settings {
        let mut exprs = Vec::new();
        if let Some(text) = &settings.auto_track_bookmarks {
            exprs.push(parse_remote_string_expression(
                ui,
                name,
                text,
                "auto-track-bookmarks",
            )?);
        }
        if let Some(text) = &settings.auto_track_created_bookmarks {
            exprs.push(parse_remote_string_expression(
                ui,
                name,
                text,
                "auto-track-created-bookmarks",
            )?);
        }
        if exprs.is_empty() {
            continue;
        }
        matchers.insert(
            name.clone(),
            StringExpression::union_all(exprs).to_matcher(),
        );
    }
    Ok(matchers)
}

/// Parses the given `remotes.<name>.fetch-bookmarks` setting.
pub fn parse_remote_fetch_bookmarks(
    ui: &Ui,
    remote_settings: &RemoteSettingsMap,
    name: &RemoteName,
) -> Result<Option<StringExpression>, CommandError> {
    remote_settings
        .get(name)
        .and_then(|settings| settings.fetch_bookmarks.as_ref())
        .map(|text| parse_remote_string_expression(ui, name, text, "fetch-bookmarks"))
        .transpose()
}

/// Parses the given `remotes.<name>.fetch-tags` setting.
pub fn parse_remote_fetch_tags(
    ui: &Ui,
    remote_settings: &RemoteSettingsMap,
    name: &RemoteName,
) -> Result<Option<StringExpression>, CommandError> {
    remote_settings
        .get(name)
        .and_then(|settings| settings.fetch_tags.as_ref())
        .map(|text| parse_remote_string_expression(ui, name, text, "fetch-tags"))
        .transpose()
}

fn parse_remote_string_expression(
    ui: &Ui,
    name: &RemoteName,
    text: &str,
    field_name: &'static str,
) -> Result<StringExpression, CommandError> {
    let mut diagnostics = vec![];
    let result =
        config_util::parse_remote_string_expression(&mut diagnostics, name, text, field_name);
    print_remote_setting_diagnostics(ui, &diagnostics)?;
    Ok(result?)
}

/// Prints diagnostics from parsing `remotes.<name>.<field>` settings.
pub fn print_remote_setting_diagnostics(
    ui: &Ui,
    diagnostics: &[RemoteSettingDiagnostics],
) -> io::Result<()> {
    for RemoteSettingDiagnostics {
        remote,
        field_name,
        diagnostics,
    } in diagnostics
    {
        print_parse_diagnostics(
            ui,
            &format!("In `remotes.{}.{field_name}`", remote.as_symbol()),
            diagnostics,
        )?;
    }
    Ok(())
}
