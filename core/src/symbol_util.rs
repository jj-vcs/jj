// Copyright 2020-2026 The Jujutsu Authors
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

//! Symbol and string formatting and parsing utilities for our DSL.

use std::ascii;

/// Escapes special characters in the input.
pub fn escape_string(unescaped: &str) -> String {
    let mut escaped = String::with_capacity(unescaped.len());
    escape_string_to_buf(&mut escaped, unescaped);
    escaped
}

/// Formats a string by quoting and escaping it.
pub fn format_string(unescaped: &str) -> String {
    let mut escaped = String::with_capacity(unescaped.len() + 2);
    escaped.push('"');
    escape_string_to_buf(&mut escaped, unescaped);
    escaped.push('"');
    escaped
}

fn escape_string_to_buf(escaped: &mut String, unescaped: &str) {
    for c in unescaped.chars() {
        match c {
            '"' => escaped.push_str(r#"\""#),
            '\\' => escaped.push_str(r#"\\"#),
            '\t' => escaped.push_str(r#"\t"#),
            '\r' => escaped.push_str(r#"\r"#),
            '\n' => escaped.push_str(r#"\n"#),
            '\0' => escaped.push_str(r#"\0"#),
            c if c.is_ascii_control() => {
                for b in ascii::escape_default(c as u8) {
                    escaped.push(b as char);
                }
            }
            c => escaped.push(c),
        }
    }
}

/// Parses an escape sequence into a character.
pub fn unescape_char(escaped: &str) -> char {
    assert!(escaped.starts_with('\\'));
    match &escaped[1..] {
        "\"" => '"',
        "\\" => '\\',
        "t" => '\t',
        "r" => '\r',
        "n" => '\n',
        "0" => '\0',
        "e" => '\x1b',
        hex if hex.starts_with('x') => {
            char::from(u8::from_str_radix(&hex[1..], 16).expect("hex characters"))
        }
        char => panic!("invalid escape: \\{char:?}"),
    }
}
