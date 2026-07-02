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

//! Revset language parser and shared DSL/name utilities.

// Needed so that proc macros can be used inside jj_lib and by external crates
// that depend on it.
// See:
// - https://github.com/rust-lang/rust/issues/54647#issuecomment-432015102
// - https://github.com/rust-lang/rust/issues/54363
extern crate self as jj_revset_parser;

pub mod content_hash;
pub mod dsl_util;
pub mod fmt;
pub mod hex_util;
pub mod parser;
pub mod ref_name;
