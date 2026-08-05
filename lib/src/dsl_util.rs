// Copyright 2020-2024 The Jujutsu Authors
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

//! Domain-specific language helpers.

pub use jj_core::dsl_util::AliasDeclaration;
pub use jj_core::dsl_util::AliasDeclarationParser;
pub use jj_core::dsl_util::AliasDefinitionParser;
pub use jj_core::dsl_util::AliasExpandError;
pub use jj_core::dsl_util::AliasExpandableExpression;
pub use jj_core::dsl_util::AliasId;
pub use jj_core::dsl_util::AliasesMap;
pub use jj_core::dsl_util::Diagnostics;
pub use jj_core::dsl_util::ExpressionFolder;
pub use jj_core::dsl_util::ExpressionNode;
pub use jj_core::dsl_util::FoldableExpression;
pub use jj_core::dsl_util::FunctionCallNode;
pub use jj_core::dsl_util::FunctionCallParser;
pub use jj_core::dsl_util::InvalidArguments;
pub use jj_core::dsl_util::KeywordArgument;
pub use jj_core::dsl_util::PatternNode;
pub use jj_core::dsl_util::StringLiteralParser;
pub use jj_core::dsl_util::collect_similar;
pub use jj_core::dsl_util::escape_string;
pub use jj_core::dsl_util::expand_aliases;
pub use jj_core::dsl_util::expand_aliases_with_locals;
pub use jj_core::dsl_util::fold_expression_nodes;
pub use jj_core::dsl_util::fold_function_call_args;
