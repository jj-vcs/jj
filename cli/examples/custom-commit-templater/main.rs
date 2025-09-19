// Copyright 2024 The Jujutsu Authors
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

use std::any::Any;
use std::sync::Arc;

use jj_cli::cli_util::CliRunner;
use jj_cli::commit_templater::CommitTemplateBuildFnTable;
use jj_cli::commit_templater::CommitTemplateLanguageExtension;
use jj_cli::template_parser;
use jj_cli::template_parser::TemplateParseError;
use jj_cli::templater::TemplatePropertyExt as _;
use jj_lib::backend::CommitId;
use jj_lib::commit::Commit;
use jj_lib::extensions_map::ExtensionsMap;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo::Repo;
use jj_lib::revset::FunctionCallNode;
use jj_lib::revset::LoweringContext;
use jj_lib::revset::PartialSymbolResolver;
use jj_lib::revset::RevsetDiagnostics;
use jj_lib::revset::RevsetExpression;
use jj_lib::revset::RevsetFilterExtension;
use jj_lib::revset::RevsetFilterPredicate;
use jj_lib::revset::RevsetParseError;
use jj_lib::revset::RevsetResolutionError;
use jj_lib::revset::SymbolResolverExtension;
use jj_lib::revset::UserRevsetExpression;
use once_cell::sync::OnceCell;

struct HexCounter;

fn num_digits_in_id(id: &CommitId) -> i64 {
    let mut count = 0;
    for ch in id.hex().chars() {
        if ch.is_ascii_digit() {
            count += 1;
        }
    }
    count
}

fn num_char_in_id(commit: Commit, ch_match: char) -> i64 {
    let mut count = 0;
    for ch in commit.id().hex().chars() {
        if ch == ch_match {
            count += 1;
        }
    }
    count
}

#[derive(Default)]
struct MostDigitsInId {
    count: OnceCell<i64>,
}

impl MostDigitsInId {
    fn count(&self, repo: &dyn Repo) -> i64 {
        *self.count.get_or_init(|| {
            RevsetExpression::all()
                .evaluate(repo)
                .unwrap()
                .iter()
                .map(Result::unwrap)
                .map(|id| num_digits_in_id(&id))
                .max()
                .unwrap_or(0)
        })
    }
}

#[derive(Default)]
struct TheDigitestResolver {
    cache: MostDigitsInId,
}

impl PartialSymbolResolver for TheDigitestResolver {
    fn resolve_symbol(
        &self,
        repo: &dyn Repo,
        symbol: &str,
    ) -> Result<Option<CommitId>, RevsetResolutionError> {
        if symbol != "thedigitest" {
            return Ok(None);
        }

        Ok(RevsetExpression::all()
            .evaluate(repo)
            .map_err(|err| RevsetResolutionError::Other(err.into()))?
            .iter()
            .map(Result::unwrap)
            .find(|id| num_digits_in_id(id) == self.cache.count(repo)))
    }
}

struct TheDigitest;

impl SymbolResolverExtension for TheDigitest {
    fn new_resolvers<'a>(&self, _repo: &'a dyn Repo) -> Vec<Box<dyn PartialSymbolResolver + 'a>> {
        vec![Box::<TheDigitestResolver>::default()]
    }
}

impl CommitTemplateLanguageExtension for HexCounter {
    fn build_fn_table<'repo>(&self) -> CommitTemplateBuildFnTable<'repo> {
        let mut table = CommitTemplateBuildFnTable::empty();
        table.commit_methods.insert(
            "has_most_digits",
            |language, _diagnostics, _build_context, property, call| {
                call.expect_no_arguments()?;
                let most_digits = language
                    .cache_extension::<MostDigitsInId>()
                    .unwrap()
                    .count(language.repo());
                let out_property =
                    property.map(move |commit| num_digits_in_id(commit.id()) == most_digits);
                Ok(out_property.into_dyn_wrapped())
            },
        );
        table.commit_methods.insert(
            "num_digits_in_id",
            |_language, _diagnostics, _build_context, property, call| {
                call.expect_no_arguments()?;
                let out_property = property.map(|commit| num_digits_in_id(commit.id()));
                Ok(out_property.into_dyn_wrapped())
            },
        );
        table.commit_methods.insert(
            "num_char_in_id",
            |_language, diagnostics, _build_context, property, call| {
                let [string_arg] = call.expect_exact_arguments()?;
                let char_arg = template_parser::catch_aliases(
                    diagnostics,
                    string_arg,
                    |_diagnostics, arg| {
                        let string = template_parser::expect_string_literal(arg)?;
                        let chars: Vec<_> = string.chars().collect();
                        match chars[..] {
                            [ch] => Ok(ch),
                            _ => Err(TemplateParseError::expression(
                                "Expected singular character argument",
                                arg.span,
                            )),
                        }
                    },
                )?;

                let out_property = property.map(move |commit| num_char_in_id(commit, char_arg));
                Ok(out_property.into_dyn_wrapped())
            },
        );

        table
    }

    fn build_cache_extensions(&self, extensions: &mut ExtensionsMap) {
        extensions.insert(MostDigitsInId::default());
    }
}

#[derive(Debug)]
struct EvenDigitsFilter;

impl RevsetFilterExtension for EvenDigitsFilter {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn matches_commit(&self, commit: &Commit) -> bool {
        num_digits_in_id(commit.id()) % 2 == 0
    }
}

fn even_digits(
    _diagnostics: &mut RevsetDiagnostics,
    function: &FunctionCallNode,
    _context: &LoweringContext,
) -> Result<Arc<UserRevsetExpression>, RevsetParseError> {
    function.expect_no_arguments()?;
    Ok(RevsetExpression::filter(RevsetFilterPredicate::Extension(
        Arc::new(EvenDigitsFilter),
    )))
}

fn main() -> std::process::ExitCode {
    CliRunner::init()
        .add_symbol_resolver_extension(Box::new(TheDigitest))
        .add_revset_function_extension("even_digits", even_digits)
        .add_commit_template_extension(Box::new(HexCounter))
        .run()
        .into()
}
