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

//! Formatting of symbols and strings for the revset language.

use crate::dsl_util::escape_string;
use crate::parser::is_identifier;

/// Formats a string as symbol by quoting and escaping it if necessary.
///
/// Note that symbols may be substituted to user aliases. Use
/// [`format_string()`] to ensure that the provided string is resolved as a
/// tag/bookmark name, commit/change ID prefix, etc.
pub fn format_symbol(literal: &str) -> String {
    if is_identifier(literal) {
        literal.to_string()
    } else {
        format_string(literal)
    }
}

/// Formats a string by quoting and escaping it.
pub fn format_string(literal: &str) -> String {
    format!(r#""{}""#, escape_string(literal))
}

/// Formats a `name@remote` symbol, applies quoting and escaping if necessary.
pub fn format_remote_symbol(name: &str, remote: &str) -> String {
    let name = format_symbol(name);
    let remote = format_symbol(remote);
    format!("{name}@{remote}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_string_literal() {
        // Valid identifiers don't need quoting
        assert_eq!(format_symbol("foo"), "foo");
        assert_eq!(format_symbol("foo.bar"), "foo.bar");

        // Invalid identifiers need quoting
        assert_eq!(format_symbol("foo@bar"), r#""foo@bar""#);
        assert_eq!(format_symbol("foo bar"), r#""foo bar""#);
        assert_eq!(format_symbol(" foo "), r#"" foo ""#);
        assert_eq!(format_symbol("(foo)"), r#""(foo)""#);
        assert_eq!(format_symbol("all:foo"), r#""all:foo""#);

        // Some characters also need escaping
        assert_eq!(format_symbol("foo\"bar"), r#""foo\"bar""#);
        assert_eq!(format_symbol("foo\\bar"), r#""foo\\bar""#);
        assert_eq!(format_symbol("foo\\\"bar"), r#""foo\\\"bar""#);
        assert_eq!(format_symbol("foo\nbar"), r#""foo\nbar""#);

        // Some characters don't technically need escaping, but we escape them for
        // clarity
        assert_eq!(format_symbol("foo\"bar"), r#""foo\"bar""#);
        assert_eq!(format_symbol("foo\\bar"), r#""foo\\bar""#);
        assert_eq!(format_symbol("foo\\\"bar"), r#""foo\\\"bar""#);
        assert_eq!(format_symbol("foo \x01 bar"), r#""foo \x01 bar""#);
    }

    #[test]
    fn test_escape_remote_symbol() {
        assert_eq!(format_remote_symbol("foo", "bar"), "foo@bar");
        assert_eq!(
            format_remote_symbol(" foo ", "bar:baz"),
            r#"" foo "@"bar:baz""#
        );
    }
}
