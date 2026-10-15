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

//! Revset helpers for the workspace configuration.

use std::sync::Arc;

use jj_lib::ref_name::RemoteName;
use jj_lib::repo::Repo;
use jj_lib::revset;
use jj_lib::revset::ResolvedRevsetExpression;
use jj_lib::revset::RevsetDiagnostics;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::RevsetParseContext;
use jj_lib::revset::RevsetParseError;
use jj_lib::revset::RevsetResolutionError;
use jj_lib::revset::SymbolResolver;
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
