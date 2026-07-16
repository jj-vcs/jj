// Copyright 2021-2024 The Jujutsu Authors
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

// This is needed so we export the same interface as usual without people
// noticing that we moved everything to the jj-core crate.
#![expect(unused_imports)]

pub use jj_core::revset_parser::BinaryOp;
pub use jj_core::revset_parser::ExpressionKind;
pub use jj_core::revset_parser::ExpressionNode;
pub use jj_core::revset_parser::FunctionCallNode;
pub use jj_core::revset_parser::PatternNode;
pub use jj_core::revset_parser::RevsetAliasParser;
pub use jj_core::revset_parser::RevsetAliasesMap;
pub use jj_core::revset_parser::RevsetDiagnostics;
pub use jj_core::revset_parser::RevsetParseError;
pub use jj_core::revset_parser::RevsetParseErrorKind;
pub use jj_core::revset_parser::Rule;
pub use jj_core::revset_parser::UnaryOp;
pub use jj_core::revset_parser::catch_aliases;
pub use jj_core::revset_parser::expect_literal;
pub use jj_core::revset_parser::expect_string_literal;
pub use jj_core::revset_parser::expect_string_pattern;
pub use jj_core::revset_parser::is_identifier;
pub use jj_core::revset_parser::parse_program;
pub use jj_core::revset_parser::parse_symbol;
