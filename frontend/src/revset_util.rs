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

//! Helpers for parsing and evaluating revsets in a workspace.

use std::sync::Arc;

use futures::StreamExt as _;
use futures::TryStreamExt as _;
use futures::stream::LocalBoxStream;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::id_prefix::IdPrefixContext;
use jj_lib::ref_name::RemoteName;
use jj_lib::repo::Repo;
use jj_lib::revset;
use jj_lib::revset::ResolvedRevset;
use jj_lib::revset::ResolvedRevsetExpression;
use jj_lib::revset::Revset;
use jj_lib::revset::RevsetDiagnostics;
use jj_lib::revset::RevsetEvaluationError;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::RevsetExtensions;
use jj_lib::revset::RevsetParseContext;
use jj_lib::revset::RevsetParseError;
use jj_lib::revset::RevsetResolutionError;
use jj_lib::revset::RevsetStreamExt as _;
use jj_lib::revset::SymbolResolver;
use jj_lib::revset::SymbolResolverExtension;
use jj_lib::revset::UserRevsetExpression;
use jj_lib::store::Store;
use thiserror::Error;

const USER_IMMUTABLE_HEADS: &str = "immutable_heads";

/// Returns the special remote name that should be ignored by default.
#[cfg_attr(not(feature = "git"), expect(unused_variables))]
pub fn default_ignored_remote_name(store: &Store) -> Option<&'static RemoteName> {
    #[cfg(feature = "git")]
    {
        use jj_lib::git;
        if git::get_git_backend(store).is_ok() {
            return Some(git::REMOTE_NAME_FOR_LOCAL_GIT_REPO);
        }
    }
    None
}

/// Error from [`parse_immutable_heads_expression()`].
#[derive(Debug, Error)]
pub enum ImmutableHeadsError {
    /// The `immutable_heads()` alias isn't defined.
    #[error("The `revset-aliases.immutable_heads()` alias is not defined")]
    NotDefined,
    /// The `immutable_heads()` alias couldn't be parsed.
    #[error(transparent)]
    Parse(#[from] RevsetParseError),
}

/// Parses user-configured expression defining the heads of the immutable set.
/// Includes the root commit.
pub fn parse_immutable_heads_expression(
    diagnostics: &mut RevsetDiagnostics,
    context: &RevsetParseContext,
) -> Result<Arc<UserRevsetExpression>, ImmutableHeadsError> {
    let (_, _, immutable_heads_str, _) = context
        .aliases_map
        .get_function(USER_IMMUTABLE_HEADS, 0)
        .ok_or(ImmutableHeadsError::NotDefined)?;
    let heads = revset::parse(diagnostics, immutable_heads_str, context)?;
    Ok(heads.union(&RevsetExpression::root()))
}

/// Parses and resolves `trunk()` alias to detect name resolution error in it.
///
/// Returns `None` if the alias isn't defined or couldn't be parsed. Returns
/// `Err` if the parsed expression had name resolution error.
pub fn try_resolve_trunk_alias(
    repo: &dyn Repo,
    context: &RevsetParseContext,
) -> Result<Option<Arc<ResolvedRevsetExpression>>, RevsetResolutionError> {
    let Some((_, _, revset_str, _)) = context.aliases_map.get_function("trunk", 0) else {
        return Ok(None);
    };
    let Ok(expression) = revset::parse(&mut RevsetDiagnostics::new(), revset_str, context) else {
        return Ok(None);
    };
    // Not using IdPrefixContext since trunk() revset shouldn't contain short
    // prefixes.
    let symbol_resolver = SymbolResolver::new(repo, context.extensions.symbol_resolvers());
    let resolved = expression.resolve_user_expression(repo, &symbol_resolver)?;
    let (expression, _other_repos) = resolved.into_inner();
    Ok(Some(expression))
}

/// Error from evaluating a user-provided revset expression.
#[derive(Debug, Error)]
pub enum UserRevsetEvaluationError {
    /// A symbol in the expression couldn't be resolved.
    #[error(transparent)]
    Resolution(RevsetResolutionError),
    /// The resolved expression couldn't be evaluated.
    #[error(transparent)]
    Evaluation(RevsetEvaluationError),
}

/// Wrapper around `UserRevsetExpression` to provide convenient methods.
pub struct RevsetExpressionEvaluator<'repo> {
    repo: &'repo dyn Repo,
    extensions: Arc<RevsetExtensions>,
    id_prefix_context: &'repo IdPrefixContext,
    expression: Arc<UserRevsetExpression>,
}

impl<'repo> RevsetExpressionEvaluator<'repo> {
    /// Creates an evaluator of `expression` in `repo`. Short commit and change
    /// ID prefixes are resolved within `id_prefix_context`.
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
    ///
    /// Note that this discards any extra index data needed to evaluate
    /// `at_operation()` expressions pointing to operations whose commits
    /// aren't indexed in the current repo. Use [`Self::evaluate()`] if the
    /// expression is to be evaluated as is.
    // TODO: migrate most callers to `evaluate()` or to `resolve_revset`, so
    // callers don't drop the merged_index on the floor when the expression
    // contains an `at_operation(OP, x)`. This can be done over time.
    pub fn resolve(&self) -> Result<Arc<ResolvedRevsetExpression>, RevsetResolutionError> {
        let (expression, _other_repos) = self.resolve_revset()?.into_inner();
        Ok(expression)
    }

    /// Resolves user symbols in the expression.
    pub fn resolve_revset(&self) -> Result<ResolvedRevset<'_>, RevsetResolutionError> {
        let symbol_resolver = default_symbol_resolver(
            self.repo,
            self.extensions.symbol_resolvers(),
            self.id_prefix_context,
        );
        self.expression
            .resolve_user_expression(self.repo, &symbol_resolver)
    }

    /// Evaluates the expression.
    pub fn evaluate(&self) -> Result<Box<dyn Revset>, UserRevsetEvaluationError> {
        self.resolve_revset()
            .map_err(UserRevsetEvaluationError::Resolution)?
            .evaluate()
            .map_err(UserRevsetEvaluationError::Evaluation)
    }

    /// Evaluates the expression to an iterator over commit ids. Entries are
    /// sorted in reverse topological order.
    pub fn evaluate_to_commit_ids(
        &self,
    ) -> Result<
        LocalBoxStream<'static, Result<CommitId, RevsetEvaluationError>>,
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

    /// Evaluates the expression to a single commit. Returns an error if the
    /// expression evaluates to no commits or to multiple commits.
    pub async fn evaluate_to_single_commit(&self) -> Result<Commit, RevsetEvaluationSizeError> {
        // The number of commits to pass to the error (will be shown in the error
        // message).
        let max_commits = 5;
        let mut commits: Vec<_> = self
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
                Err(RevsetEvaluationSizeError::Multiple { commits, has_more })
            }
        }
    }
}

/// Error when evaluating a revset into a single commit.
#[derive(Debug, Error)]
pub enum RevsetEvaluationSizeError {
    /// The revset evaluated to no commits.
    #[error("The revset didn't resolve to any revisions")]
    Empty,
    /// The revset evaluated to multiple commits.
    #[error("The revset resolved to more than one revision")]
    Multiple {
        /// A few of the commits (enough for an error message); do not expect
        /// this to be the entire evaluated revset.
        commits: Vec<Commit>,
        /// Whether there were more commits than those in `commits`.
        has_more: bool,
    },
    /// An error occurred during revset evaluation; size unknown.
    #[error(transparent)]
    Other(UserRevsetEvaluationError),
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
