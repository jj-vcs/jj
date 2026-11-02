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

/// Parses user-configured expression defining the heads of the set of commits
/// that are immutable from any workspace (`immutable_heads()`). Includes the
/// root commit.
pub fn parse_immutable_heads_expression(
    diagnostics: &mut RevsetDiagnostics,
    context: &RevsetParseContext,
) -> Result<Arc<UserRevsetExpression>, RevsetParseError> {
    let heads = parse_alias_body(diagnostics, "immutable_heads", context)?;
    Ok(heads.union(&RevsetExpression::root()))
}

/// Parses user-configured expression defining the heads of the set of commits
/// that cannot be rewritten from this workspace
/// (`workspace_immutable_heads()`). Includes the root commit.
///
/// The ancestors of this expression must match the default definition of the
/// `immutable()` alias.
pub fn parse_workspace_immutable_heads_expression(
    diagnostics: &mut RevsetDiagnostics,
    context: &RevsetParseContext,
) -> Result<Arc<UserRevsetExpression>, RevsetParseError> {
    let heads = parse_alias_body(diagnostics, "workspace_immutable_heads", context)?;
    Ok(heads.union(&RevsetExpression::root()))
}

fn parse_alias_body(
    diagnostics: &mut RevsetDiagnostics,
    name: &str,
    context: &RevsetParseContext,
) -> Result<Arc<UserRevsetExpression>, RevsetParseError> {
    let (_, _, body, _) = context
        .aliases_map
        .get_function(name, 0)
        .expect("alias should be defined by default");
    revset::parse(diagnostics, body, context)
}

/// Parses and resolves `trunk()` alias to detect name resolution error in it.
///
/// Returns `None` if the alias couldn't be parsed. Returns `Err` if the parsed
/// expression had name resolution error.
pub fn try_resolve_trunk_alias(
    repo: &dyn Repo,
    context: &RevsetParseContext,
) -> Result<Option<Arc<ResolvedRevsetExpression>>, RevsetResolutionError> {
    let (_, _, revset_str, _) = context
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
    let (expression, _other_repos) = resolved.into_inner();
    Ok(Some(expression))
}
