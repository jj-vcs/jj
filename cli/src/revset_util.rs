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
use std::sync::Arc;

use futures::StreamExt as _;
use futures::TryStreamExt as _;
use itertools::Itertools as _;
use jj_lib::commit::Commit;
use jj_lib::formatter::Formatter;
use jj_lib::ref_name::RefNameBuf;
use jj_lib::ref_name::RemoteName;
use jj_lib::ref_name::RemoteNameBuf;
use jj_lib::repo::Repo;
use jj_lib::revset;
use jj_lib::revset::ResolvedRevsetExpression;
use jj_lib::revset::RevsetDiagnostics;
use jj_lib::revset::RevsetParseContext;
use jj_lib::revset::RevsetParseError;
use jj_lib::revset::RevsetResolutionError;
use jj_lib::revset::SymbolResolver;
use jj_lib::revset_util::RevsetExpressionEvaluator;
use jj_lib::settings::RemoteSettingsMap;
use jj_lib::str_util::StringExpression;
use jj_lib::str_util::StringMatcher;
use jj_lib::user_error::revset_parse_error_hint;
use thiserror::Error;

use crate::command_error::CommandError;
use crate::command_error::config_error_with_message;
use crate::command_error::print_parse_diagnostics;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;
use crate::templater::TemplateRenderer;
use crate::ui::Ui;

/// Parses and resolves `trunk()` alias to detect name resolution error in it.
///
/// Returns `None` if the alias couldn't be parsed. Returns `Err` if the parsed
/// expression had name resolution error.
pub fn try_resolve_trunk_alias(
    repo: &dyn Repo,
    context: &RevsetParseContext,
) -> Result<Option<Arc<ResolvedRevsetExpression>>, RevsetResolutionError> {
    let (_, _, revset_str) = context
        .aliases_map
        .get_function("trunk", 0)
        .expect("trunk() should be defined by default");
    let Ok(expression) = revset::parse(&mut RevsetDiagnostics::new(), revset_str, context) else {
        return Ok(None);
    };
    // Not using IdPrefixContext since trunk() revset shouldn't contain short
    // prefixes.
    let symbol_resolver = SymbolResolver::new(repo, context.extensions.symbol_resolvers());
    let resolved = expression.resolve_user_expression(repo, &symbol_resolver)?;
    Ok(Some(resolved))
}

pub(super) async fn evaluate_revset_to_single_commit<'a>(
    revision_str: &str,
    expression: &RevsetExpressionEvaluator<'_>,
    commit_summary_template: impl FnOnce() -> TemplateRenderer<'a, Commit>,
) -> Result<Commit, CommandError> {
    let commits: Vec<_> = expression
        .evaluate_to_commits()?
        .take(6)
        .try_collect()
        .await?;
    match commits.as_slice() {
        [commit] => Ok(commit.clone()),
        [] => Err(user_error(format!(
            "Revset `{revision_str}` didn't resolve to any revisions"
        ))),
        _ => {
            let elided = commits.len() > 5;
            Err(format_multiple_revisions_error(
                revision_str,
                &commits[..std::cmp::min(5, commits.len())],
                elided,
                &commit_summary_template(),
            ))
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

/// Parses the given `remotes.<name>.auto-track-bookmarks` settings into a map
/// of string matchers.
pub fn parse_remote_auto_track_bookmarks_map(
    ui: &Ui,
    remote_settings: &RemoteSettingsMap,
) -> Result<HashMap<RemoteNameBuf, StringMatcher>, CommandError> {
    let mut matchers = HashMap::new();
    for (name, settings) in remote_settings {
        let Some(text) = &settings.auto_track_bookmarks else {
            continue;
        };
        let expr = parse_remote_string_expression(ui, name, text, "auto-track-bookmarks")?;
        matchers.insert(name.clone(), expr.to_matcher());
    }
    Ok(matchers)
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
    field_name: &str,
) -> Result<StringExpression, CommandError> {
    let mut diagnostics = RevsetDiagnostics::new();
    let expr = revset::parse_string_expression(&mut diagnostics, text).map_err(|err| {
        // From<RevsetParseError>, but with different message and error kind
        let hint = revset_parse_error_hint(&err);
        let message = format!(
            "Invalid `remotes.{}.{field_name}`: {}",
            name.as_symbol(),
            err.kind()
        );
        let mut cmd_err = config_error_with_message(message, err);
        cmd_err.extend_hints(hint);
        cmd_err
    })?;
    print_parse_diagnostics(
        ui,
        &format!("In `remotes.{}.{field_name}`", name.as_symbol()),
        &diagnostics,
    )?;
    Ok(expr)
}
