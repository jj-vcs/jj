// Copyright 2022-2026 The Jujutsu Authors
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

use std::fmt;
use std::io;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::stream::LocalBoxStream;
use thiserror::Error;

use crate::backend::CommitId;
use crate::commit::Commit;
use crate::config::ConfigNamePathBuf;
use crate::config::ConfigSource;
use crate::config::StackedConfig;
use crate::id_prefix::IdPrefixContext;
use crate::repo::Repo;
use crate::revset;
use crate::revset::ResolvedRevsetExpression;
use crate::revset::Revset;
use crate::revset::RevsetDiagnostics;
use crate::revset::RevsetEvaluationError;
use crate::revset::RevsetExpression;
use crate::revset::RevsetExtensions;
use crate::revset::RevsetParseContext;
use crate::revset::RevsetParseError;
use crate::revset::RevsetResolutionError;
use crate::revset::RevsetStreamExt as _;
use crate::revset::SymbolResolver;
use crate::revset::SymbolResolverExtension;
use crate::revset::UserRevsetExpression;

const USER_IMMUTABLE_HEADS: &str = "immutable_heads";

/// An error which can occur when evaluating user provided revsets.
#[derive(Debug, Error)]
pub enum UserRevsetEvaluationError {
    /// The revset resolved to nothing, i.e the empty set.
    #[error(transparent)]
    Resolution(RevsetResolutionError),
    /// The revset failed to evaluate because there was a syntactical error or
    /// otherwise.
    #[error(transparent)]
    Evaluation(RevsetEvaluationError),
}

/// Wrapper around `UserRevsetExpression` to provide convenient methods.
pub struct RevsetExpressionEvaluator<'repo> {
    /// The repo the expressions get evaluated in.
    repo: &'repo dyn Repo,
    /// The registered extensions with this Evaluator, usually provided by the
    /// environment.
    extensions: Arc<RevsetExtensions>,
    /// The `IdPrefixContext` used during the evaluation.
    id_prefix_context: &'repo IdPrefixContext,
    /// The user-provided `RevsetExpression`.
    expression: Arc<UserRevsetExpression>,
}

impl<'repo> RevsetExpressionEvaluator<'repo> {
    /// Creates a new `RevsetExpressionEvaluator` for the given `repo` and
    /// `expression`, respecting any `extensions` provided
    /// during the evaluation.
    pub fn new(
        repo: &'repo dyn Repo,
        extensions: Arc<RevsetExtensions>,
        id_prefix_context: &'repo IdPrefixContext,
        expression: Arc<UserRevsetExpression>,
    ) -> Self {
        Self {
            repo,
            extensions,
            id_prefix_context,
            expression,
        }
    }

    /// Returns the underlying expression.
    pub fn expression(&self) -> &Arc<UserRevsetExpression> {
        &self.expression
    }

    /// Intersects the underlying expression with the `other` expression.
    pub fn intersect_with(&mut self, other: &Arc<UserRevsetExpression>) {
        self.expression = self.expression.intersection(other);
    }

    /// Resolves user symbols in the expression, returns new expression.
    pub fn resolve(&self) -> Result<Arc<ResolvedRevsetExpression>, RevsetResolutionError> {
        let symbol_resolver = default_symbol_resolver(
            self.repo,
            self.extensions.symbol_resolvers(),
            self.id_prefix_context,
        );
        self.expression
            .resolve_user_expression(self.repo, &symbol_resolver)
    }

    /// Evaluates the expression.
    pub fn evaluate(&self) -> Result<Box<dyn Revset + 'repo>, UserRevsetEvaluationError> {
        self.resolve()
            .map_err(UserRevsetEvaluationError::Resolution)?
            .evaluate(self.repo)
            .map_err(UserRevsetEvaluationError::Evaluation)
    }

    /// Evaluates the expression to an iterator over commit ids. Entries are
    /// sorted in reverse topological order.
    pub fn evaluate_to_commit_ids(
        &self,
    ) -> Result<
        LocalBoxStream<'repo, Result<CommitId, RevsetEvaluationError>>,
        UserRevsetEvaluationError,
    > {
        Ok(self.evaluate()?.stream())
    }

    /// Evaluates the expression to an iterator over commit objects. Entries are
    /// sorted in reverse topological order.
    pub fn evaluate_to_commits(
        &self,
    ) -> Result<
        LocalBoxStream<'repo, Result<Commit, RevsetEvaluationError>>,
        UserRevsetEvaluationError,
    > {
        Ok(self
            .evaluate()?
            .stream()
            .commits(self.repo.store())
            .boxed_local())
    }
}

/// Warn if the `config` contains a user-provided name which matches the
/// built-in expressions. All output is written to `warn`.
pub fn warn_user_redefined_builtin(
    config: &StackedConfig,
    table_name: &ConfigNamePathBuf,
    mut warn: impl FnMut(fmt::Arguments<'_>) -> io::Result<()>,
) -> io::Result<()> {
    let checked_mutability_builtins = ["mutable()", "immutable()", "builtin_immutable_heads()"];
    for layer in config
        .layers()
        .iter()
        .skip_while(|layer| layer.source == ConfigSource::Default)
    {
        let Ok(Some(table)) = layer.look_up_table(table_name) else {
            continue;
        };
        for decl in checked_mutability_builtins
            .iter()
            .filter(|decl| table.contains_key(decl))
        {
            warn(format_args!(
                "Redefining `{table_name}.{decl}` is not recommended; redefine \
                 `immutable_heads()` instead",
            ))?;
        }
    }
    Ok(())
}

/// Parses user-configured expression defining the heads of the immutable set.
/// Includes the root commit.
pub fn parse_immutable_heads_expression(
    diagnostics: &mut RevsetDiagnostics,
    context: &RevsetParseContext,
) -> Result<Arc<UserRevsetExpression>, RevsetParseError> {
    let (_, _, immutable_heads_str) = context
        .aliases_map
        .get_function(USER_IMMUTABLE_HEADS, 0)
        .unwrap();
    let heads = revset::parse(diagnostics, immutable_heads_str, context)?;
    Ok(heads.union(&RevsetExpression::root()))
}

/// Wraps the given `IdPrefixContext` in `SymbolResolver` to be passed in to
/// `evaluate()`.
pub fn default_symbol_resolver<'a>(
    repo: &'a dyn Repo,
    extensions: &[impl AsRef<dyn SymbolResolverExtension>],
    id_prefix_context: &'a IdPrefixContext,
) -> SymbolResolver<'a> {
    SymbolResolver::new(repo, extensions).with_id_prefix_context(id_prefix_context)
}
