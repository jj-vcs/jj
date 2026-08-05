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

//! Contains some basic methods around Revsets.
// TODO: consider moving the optimizer into this.

use crate::dsl_util;
use crate::revset_parser;

/// Formats a string as symbol by quoting and escaping it if necessary.
///
/// Note that symbols may be substituted to user aliases. Use
/// [`format_string()`] to ensure that the provided string is resolved as a
/// tag/bookmark name, commit/change ID prefix, etc.
pub fn format_symbol(literal: &str) -> String {
    if revset_parser::is_identifier(literal) {
        literal.to_string()
    } else {
        format_string(literal)
    }
}

/// Formats a string by quoting and escaping it.
pub fn format_string(literal: &str) -> String {
    format!(r#""{}""#, dsl_util::escape_string(literal))
}

/// Formats a `name@remote` symbol, applies quoting and escaping if necessary.
pub fn format_remote_symbol(name: &str, remote: &str) -> String {
    let name = format_symbol(name);
    let remote = format_symbol(remote);
    format!("{name}@{remote}")
}
