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

use std::rc::Rc;
use std::sync::Arc;

use itertools::Itertools as _;
use jj_lib::backend::BackendResult;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::id_prefix::IdPrefixContext;
use jj_lib::repo::Repo;
use jj_lib::revset;
use jj_lib::revset::DefaultSymbolResolver;
use jj_lib::revset::Revset;
use jj_lib::revset::RevsetAliasesMap;
use jj_lib::revset::RevsetCommitRef;
use jj_lib::revset::RevsetDiagnostics;
use jj_lib::revset::RevsetEvaluationError;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::RevsetExtensions;
use jj_lib::revset::RevsetIteratorExt as _;
use jj_lib::revset::RevsetParseContext;
use jj_lib::revset::RevsetParseError;
use jj_lib::revset::RevsetResolutionError;
use jj_lib::revset::SymbolResolverExtension;
use jj_lib::settings::ConfigResultExt as _;
use thiserror::Error;

use crate::command_error::user_error;
use crate::command_error::CommandError;
use crate::config::ConfigSource;
use crate::config::LayeredConfigs;
use crate::formatter::Formatter;
use crate::templater::TemplateRenderer;
use crate::ui::Ui;

const USER_IMMUTABLE_HEADS: &str = "immutable_heads";

#[derive(Debug, Error)]
pub enum UserRevsetEvaluationError {
    #[error(transparent)]
    Resolution(RevsetResolutionError),
    #[error(transparent)]
    Evaluation(RevsetEvaluationError),
}

/// Wrapper around `RevsetExpression` to provide convenient methods.
pub struct RevsetExpressionEvaluator<'repo> {
    repo: &'repo dyn Repo,
    extensions: Arc<RevsetExtensions>,
    id_prefix_context: &'repo IdPrefixContext,
    expression: Rc<RevsetExpression>,
}

impl<'repo> RevsetExpressionEvaluator<'repo> {
    pub fn new(
        repo: &'repo dyn Repo,
        extensions: Arc<RevsetExtensions>,
        id_prefix_context: &'repo IdPrefixContext,
        expression: Rc<RevsetExpression>,
    ) -> Self {
        RevsetExpressionEvaluator {
            repo,
            extensions,
            id_prefix_context,
            expression,
        }
    }

    /// Returns the underlying expression.
    pub fn expression(&self) -> &Rc<RevsetExpression> {
        &self.expression
    }

    /// Intersects the underlying expression with the `other` expression.
    pub fn intersect_with(&mut self, other: &Rc<RevsetExpression>) {
        self.expression = self.expression.intersection(other);
    }

    /// Evaluates the expression.
    pub fn evaluate(&self) -> Result<Box<dyn Revset + 'repo>, UserRevsetEvaluationError> {
        let symbol_resolver = default_symbol_resolver(
            self.repo,
            self.extensions.symbol_resolvers(),
            self.id_prefix_context,
        );
        evaluate(self.repo, &symbol_resolver, self.expression.clone())
    }

    /// Evaluates the expression to an iterator over commit ids. Entries are
    /// sorted in reverse topological order.
    pub fn evaluate_to_commit_ids(
        &self,
    ) -> Result<Box<dyn Iterator<Item = CommitId> + 'repo>, UserRevsetEvaluationError> {
        Ok(self.evaluate()?.iter())
    }

    /// Evaluates the expression to an iterator over commit objects. Entries are
    /// sorted in reverse topological order.
    pub fn evaluate_to_commits(
        &self,
    ) -> Result<impl Iterator<Item = BackendResult<Commit>> + 'repo, UserRevsetEvaluationError>
    {
        Ok(self.evaluate()?.iter().commits(self.repo.store()))
    }
}

fn warn_user_redefined_builtin(
    ui: &Ui,
    source: &ConfigSource,
    name: &str,
) -> Result<(), CommandError> {
    match source {
        ConfigSource::Default => (),
        ConfigSource::Env | ConfigSource::User | ConfigSource::Repo | ConfigSource::CommandArg => {
            let checked_mutability_builtins =
                ["mutable()", "immutable()", "builtin_immutable_heads()"];

            if checked_mutability_builtins.contains(&name) {
                writeln!(
                    ui.warning_default(),
                    "Redefining `revset-aliases.{name}` is not recommended; redefine \
                     `immutable_heads()` instead",
                )?;
            }
        }
    }

    Ok(())
}

pub fn load_revset_aliases(
    ui: &Ui,
    layered_configs: &LayeredConfigs,
) -> Result<RevsetAliasesMap, CommandError> {
    const TABLE_KEY: &str = "revset-aliases";
    let mut aliases_map = RevsetAliasesMap::new();
    // Load from all config layers in order. 'f(x)' in default layer should be
    // overridden by 'f(a)' in user.
    for (source, config) in layered_configs.sources() {
        let table = if let Some(table) = config.get_table(TABLE_KEY).optional()? {
            table
        } else {
            continue;
        };
        for (decl, value) in table.into_iter().sorted_by(|a, b| a.0.cmp(&b.0)) {
            warn_user_redefined_builtin(ui, &source, &decl)?;

            let r = value
                .into_string()
                .map_err(|e| e.to_string())
                .and_then(|v| aliases_map.insert(&decl, v).map_err(|e| e.to_string()));

            if let Err(s) = r {
                writeln!(
                    ui.warning_default(),
                    r#"Failed to load "{TABLE_KEY}.{decl}": {s}"#
                )?;
            }
        }
    }
    Ok(aliases_map)
}

pub fn evaluate<'a>(
    repo: &'a dyn Repo,
    symbol_resolver: &DefaultSymbolResolver,
    expression: Rc<RevsetExpression>,
) -> Result<Box<dyn Revset + 'a>, UserRevsetEvaluationError> {
    let resolved = revset::optimize(expression)
        .resolve_user_expression(repo, symbol_resolver)
        .map_err(UserRevsetEvaluationError::Resolution)?;
    resolved
        .evaluate(repo)
        .map_err(UserRevsetEvaluationError::Evaluation)
}

/// Wraps the given `IdPrefixContext` in `SymbolResolver` to be passed in to
/// `evaluate()`.
pub fn default_symbol_resolver<'a>(
    repo: &'a dyn Repo,
    extensions: &[impl AsRef<dyn SymbolResolverExtension>],
    id_prefix_context: &'a IdPrefixContext,
) -> DefaultSymbolResolver<'a> {
    DefaultSymbolResolver::new(repo, extensions).with_id_prefix_context(id_prefix_context)
}

/// Parses user-configured expression defining the heads of the immutable set.
/// Includes the root commit.
pub fn parse_immutable_heads_expression(
    diagnostics: &mut RevsetDiagnostics,
    context: &RevsetParseContext,
) -> Result<Rc<RevsetExpression>, RevsetParseError> {
    let (_, _, immutable_heads_str) = context
        .aliases_map()
        .get_function(USER_IMMUTABLE_HEADS, 0)
        .unwrap();
    let heads = revset::parse(diagnostics, immutable_heads_str, context)?;
    Ok(heads.union(&RevsetExpression::root()))
}

pub(super) fn evaluate_revset_to_single_commit<'a>(
    revision_str: &str,
    expression: &RevsetExpressionEvaluator<'_>,
    commit_summary_template: impl FnOnce() -> TemplateRenderer<'a, Commit>,
    should_hint_about_all_prefix: bool,
) -> Result<Commit, CommandError> {
    let mut iter = expression.evaluate_to_commits()?.fuse();
    match (iter.next(), iter.next()) {
        (Some(commit), None) => Ok(commit?),
        (None, _) => Err(user_error(format!(
            r#"Revset "{revision_str}" didn't resolve to any revisions"#
        ))),
        (Some(commit0), Some(commit1)) => {
            let mut iter = [commit0, commit1].into_iter().chain(iter);
            let commits: Vec<_> = iter.by_ref().take(5).try_collect()?;
            let elided = iter.next().is_some();
            Err(format_multiple_revisions_error(
                revision_str,
                expression.expression(),
                &commits,
                elided,
                &commit_summary_template(),
                should_hint_about_all_prefix,
            ))
        }
    }
}

fn format_multiple_revisions_error(
    revision_str: &str,
    expression: &RevsetExpression,
    commits: &[Commit],
    elided: bool,
    template: &TemplateRenderer<'_, Commit>,
    should_hint_about_all_prefix: bool,
) -> CommandError {
    assert!(commits.len() >= 2);
    let mut cmd_err = user_error(format!(
        r#"Revset "{revision_str}" resolved to more than one revision"#
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
    if commits[0].change_id() == commits[1].change_id() {
        // Separate hint if there's commits with same change id
        cmd_err.add_formatted_hint_with(|formatter| {
            writeln!(
                formatter,
                r#"The revset "{revision_str}" resolved to these revisions:"#
            )?;
            write_commits_summary(formatter)
        });
        cmd_err.add_hint(
            "Some of these commits have the same change id. Abandon one of them with `jj abandon \
             -r <REVISION>`.",
        );
    } else if let RevsetExpression::CommitRef(RevsetCommitRef::Symbol(bookmark_name)) = expression {
        // Separate hint if there's a conflicted bookmark
        cmd_err.add_formatted_hint_with(|formatter| {
            writeln!(
                formatter,
                "Bookmark {bookmark_name} resolved to multiple revisions because it's conflicted."
            )?;
            writeln!(formatter, "It resolved to these revisions:")?;
            write_commits_summary(formatter)
        });
        cmd_err.add_hint(format!(
            "Set which revision the bookmark points to with `jj bookmark set {bookmark_name} -r \
             <REVISION>`.",
        ));
    } else {
        cmd_err.add_formatted_hint_with(|formatter| {
            writeln!(
                formatter,
                r#"The revset "{revision_str}" resolved to these revisions:"#
            )?;
            write_commits_summary(formatter)
        });
        if should_hint_about_all_prefix {
            cmd_err.add_hint(format!(
                "Prefix the expression with 'all:' to allow any number of revisions (i.e. \
                 'all:{revision_str}')."
            ));
        }
    };
    cmd_err
}
