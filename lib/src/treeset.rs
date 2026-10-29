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

//! Expression language for selecting a tree.
//!
//! The language is a superset of the revset language. A plain revset selects
//! the tree of the single revision it resolves to. In addition, there are
//! functions that compute trees that don't necessarily belong to any commit,
//! such as the merge of several commits' trees, or one side of a conflicted
//! tree.

use std::sync::Arc;

use futures::StreamExt as _;

use crate::backend::BackendResult;
use crate::dsl_util;
use crate::merged_tree::MergedTree;
use crate::merged_tree_builder::MergedTreeBuilder;
use crate::revset;
use crate::revset::ExpressionKind;
use crate::revset::ExpressionNode;
use crate::revset::RevsetDiagnostics;
use crate::revset::RevsetParseContext;
use crate::revset::RevsetParseError;
use crate::revset::RevsetParseErrorKind;
use crate::revset::UserRevsetExpression;
use crate::revset_parser;

/// Names of the functions that are specific to the treeset language.
pub const FUNCTION_NAMES: [&str; 2] = ["conflict_side", "tree_merge"];

/// Lowered treeset expression.
#[derive(Clone, Debug)]
pub enum TreesetExpression {
    /// The tree of the single revision the revset resolves to.
    Revision {
        /// Revset that should resolve to a single revision.
        expression: Arc<UserRevsetExpression>,
        /// Source text of the revset, for use in error messages.
        source: String,
    },
    /// The merge of the trees of all revisions the revset resolves to.
    TreeMerge {
        /// Revset that should resolve to at least one revision.
        expression: Arc<UserRevsetExpression>,
        /// Source text of the revset, for use in error messages.
        source: String,
    },
    /// One side of the conflicts in the tree. Paths without conflicts keep
    /// their content.
    ConflictSide {
        /// Tree to select the side from.
        tree: Box<Self>,
        /// 0-based index of the side.
        index: usize,
    },
}

/// Parses text into a treeset expression.
pub fn parse(
    diagnostics: &mut RevsetDiagnostics,
    text: &str,
    context: &RevsetParseContext,
) -> Result<TreesetExpression, RevsetParseError> {
    let node = revset::parse_program(text)?;
    let node =
        dsl_util::expand_aliases_with_locals(node, context.aliases_map, &context.local_variables)?;
    lower_expression(diagnostics, &node, text, text, context).map_err(|err| {
        if is_misplaced_tree_function(&err) {
            // The function exists, so it would be suggested as its own
            // replacement
            err
        } else {
            err.extend_function_candidates(
                context.aliases_map.function_names().chain(FUNCTION_NAMES),
            )
        }
    })
}

/// Whether the error is because a treeset function was used within a revset.
pub fn is_misplaced_tree_function(err: &RevsetParseError) -> bool {
    matches!(
        err.kind(),
        RevsetParseErrorKind::NoSuchFunction { name, .. } if FUNCTION_NAMES.contains(&name.as_str())
    )
}

/// Lowers `node` parsed from `input`. The `outer_source` is the closest
/// enclosing expression in `input`, which is reported in place of revsets that
/// come from alias definitions.
fn lower_expression(
    diagnostics: &mut RevsetDiagnostics,
    node: &ExpressionNode,
    input: &str,
    outer_source: &str,
    context: &RevsetParseContext,
) -> Result<TreesetExpression, RevsetParseError> {
    let source = user_source(node, input, outer_source);
    revset_parser::catch_aliases(diagnostics, node, |diagnostics, node| {
        if let ExpressionKind::FunctionCall(function) = &node.kind {
            match function.name {
                "tree_merge" => {
                    let [arg] = function.expect_exact_arguments()?;
                    let expression = lower_revset(diagnostics, arg, context)?;
                    return Ok(TreesetExpression::TreeMerge {
                        expression,
                        source: user_source(arg, input, source).to_owned(),
                    });
                }
                "conflict_side" => {
                    let [tree_arg, side_arg] = function.expect_exact_arguments()?;
                    let tree = lower_expression(diagnostics, tree_arg, input, source, context)?;
                    let side: usize = revset::expect_literal("integer", side_arg)?;
                    let Some(index) = side.checked_sub(1) else {
                        return Err(RevsetParseError::expression(
                            "Conflict sides are numbered from 1",
                            side_arg.span,
                        ));
                    };
                    return Ok(TreesetExpression::ConflictSide {
                        tree: Box::new(tree),
                        index,
                    });
                }
                _ => {}
            }
        }
        let expression = lower_revset(diagnostics, node, context)?;
        Ok(TreesetExpression::Revision {
            expression,
            source: source.to_owned(),
        })
    })
}

/// Returns the source text of `node` if it was parsed from `input`, or
/// `outer_source` if it came from an alias definition.
fn user_source<'a>(node: &ExpressionNode<'a>, input: &str, outer_source: &'a str) -> &'a str {
    if std::ptr::eq(node.span.get_input(), input) {
        node.span.as_str()
    } else {
        outer_source
    }
}

fn lower_revset(
    diagnostics: &mut RevsetDiagnostics,
    node: &ExpressionNode,
    context: &RevsetParseContext,
) -> Result<Arc<UserRevsetExpression>, RevsetParseError> {
    revset::lower_expression(diagnostics, node, &context.to_lowering_context())
}

/// Selects one side of the conflicts in `tree`.
///
/// Conflicted paths get the content from side `index` (0-based). Other paths
/// keep their content. Returns `None` if `tree` doesn't have that many sides.
pub async fn conflict_side(tree: &MergedTree, index: usize) -> BackendResult<Option<MergedTree>> {
    if index >= tree.tree_ids().num_sides() {
        return Ok(None);
    }
    if !tree.has_conflict() {
        return Ok(Some(tree.clone()));
    }
    let side_tree = MergedTree::resolved(
        tree.store().clone(),
        tree.tree_ids().get_add(index).unwrap().clone(),
    );
    let mut builder = MergedTreeBuilder::new(tree.clone().resolve().await?);
    let mut conflicts = tree.conflicts();
    while let Some((path, value)) = conflicts.next().await {
        value?;
        let side_value = side_tree.path_value(&path).await?.into_resolved().unwrap();
        builder.set_or_remove(path, side_value);
    }
    let new_tree = builder.write_tree().await?;
    // The base tree was resolved where possible and all remaining conflicted
    // paths were replaced by resolved values.
    debug_assert!(!new_tree.has_conflict());
    Ok(Some(new_tree))
}

#[cfg(test)]
mod tests {
    use std::assert_matches;
    use std::collections::HashMap;

    use super::*;
    use crate::fileset::FilesetAliasesMap;
    use crate::revset::RevsetAliasesMap;
    use crate::revset::RevsetExtensions;

    fn parse_with_aliases(
        text: &str,
        aliases: &[(&str, &str)],
    ) -> Result<TreesetExpression, RevsetParseError> {
        let mut aliases_map = RevsetAliasesMap::new();
        for (decl, defn) in aliases {
            aliases_map.insert(decl, *defn, None).unwrap();
        }
        let context = RevsetParseContext {
            aliases_map: &aliases_map,
            local_variables: HashMap::new(),
            user_email: "test.user@example.com",
            current_time: jiff::Timestamp::UNIX_EPOCH.to_zoned(jiff::tz::TimeZone::UTC),
            default_ignored_remote: None,
            fileset_aliases_map: &FilesetAliasesMap::new(),
            extensions: &RevsetExtensions::default(),
            workspace: None,
        };
        super::parse(&mut RevsetDiagnostics::new(), text, &context)
    }

    fn parse(text: &str) -> Result<TreesetExpression, RevsetParseError> {
        parse_with_aliases(text, &[])
    }

    #[test]
    fn test_parse_revision() {
        assert_matches!(
            parse("foo-").unwrap(),
            TreesetExpression::Revision { source, .. } if source == "foo-"
        );
        // Tree functions are only recognized at the top level
        let err = parse("tree_merge(foo) | bar").unwrap_err();
        assert!(is_misplaced_tree_function(&err));
        assert_matches!(
            err.kind(),
            RevsetParseErrorKind::NoSuchFunction { candidates, .. } if candidates.is_empty()
        );
    }

    #[test]
    fn test_parse_tree_merge() {
        assert_matches!(
            parse("tree_merge(foo-)").unwrap(),
            TreesetExpression::TreeMerge { source, .. } if source == "foo-"
        );
        assert_matches!(
            parse("tree_merge()").unwrap_err().kind(),
            RevsetParseErrorKind::InvalidFunctionArguments { .. }
        );
        assert_matches!(
            parse("tree_merge(conflict_side(foo, 1))")
                .unwrap_err()
                .kind(),
            RevsetParseErrorKind::NoSuchFunction { .. }
        );
    }

    #[test]
    fn test_parse_conflict_side() {
        assert_matches!(
            parse("conflict_side(tree_merge(foo-), 2)").unwrap(),
            TreesetExpression::ConflictSide { tree, index: 1 }
                if matches!(*tree, TreesetExpression::TreeMerge { .. })
        );
        assert_matches!(
            parse("conflict_side(foo, 1)").unwrap(),
            TreesetExpression::ConflictSide { tree, index: 0 }
                if matches!(&*tree, TreesetExpression::Revision { source, .. } if source == "foo")
        );
        assert_matches!(
            parse("conflict_side(foo, 0)").unwrap_err().kind(),
            RevsetParseErrorKind::Expression(message)
                if message == "Conflict sides are numbered from 1"
        );
        assert_matches!(
            parse("conflict_side(foo, bar)").unwrap_err().kind(),
            RevsetParseErrorKind::Expression(message) if message == "Expected integer"
        );
        assert_matches!(
            parse("conflict_side(foo)").unwrap_err().kind(),
            RevsetParseErrorKind::InvalidFunctionArguments { .. }
        );
    }

    #[test]
    fn test_parse_aliases() {
        let aliases = [("ours(x)", "conflict_side(x, 1)"), ("base", "foo-")];
        assert_matches!(
            parse_with_aliases("ours(base)", &aliases).unwrap(),
            TreesetExpression::ConflictSide { tree, index: 0 }
                if matches!(&*tree, TreesetExpression::Revision { source, .. } if source == "ours(base)")
        );
        assert_matches!(
            parse_with_aliases("base", &aliases).unwrap(),
            TreesetExpression::Revision { source, .. } if source == "base"
        );
    }

    #[test]
    fn test_parse_function_candidates() {
        assert_matches!(
            parse("tree_merg(foo)").unwrap_err().kind(),
            RevsetParseErrorKind::NoSuchFunction { candidates, .. } if candidates == &["tree_merge"]
        );
    }
}
