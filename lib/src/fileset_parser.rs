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

//! Parser for the fileset language.

// Allow unused imports here, because all symbols moved to the core crate and we need to
// continue to reexport the same APIs.
#![expect(unused_imports)]

pub use jj_core::fileset_parser::BinaryOp;
pub use jj_core::fileset_parser::ExpressionKind;
pub use jj_core::fileset_parser::ExpressionNode;
pub use jj_core::fileset_parser::FilesetAliasParser;
pub use jj_core::fileset_parser::FilesetAliasesMap;
pub use jj_core::fileset_parser::FilesetDiagnostics;
pub use jj_core::fileset_parser::FilesetParseError;
pub use jj_core::fileset_parser::FilesetParseErrorKind;
pub use jj_core::fileset_parser::FilesetParseResult;
pub use jj_core::fileset_parser::FunctionCallNode;
pub use jj_core::fileset_parser::PatternNode;
pub use jj_core::fileset_parser::Rule;
pub use jj_core::fileset_parser::UnaryOp;
pub use jj_core::fileset_parser::catch_aliases;
pub use jj_core::fileset_parser::expand_aliases;
pub use jj_core::fileset_parser::expect_string_literal;
pub use jj_core::fileset_parser::parse_program;
pub use jj_core::fileset_parser::parse_program_or_bare_string;
