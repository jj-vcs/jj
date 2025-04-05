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

//! Parsing trailers from commit messages.

use itertools::Itertools as _;

/// A key-value pair representing a trailer in a commit message, of the
/// form `Key: Value`.
#[derive(Debug, PartialEq, Clone)]
pub struct Trailer {
    /// trailer key
    pub key: String,
    /// normalized trailer value
    pub value: String,
}

/// Parse the trailers from a commit message; these are simple key-value
/// pairs, separated by a colon, describing extra information in a commit
/// message; an example is the following:
///
/// ```text
/// chore: update itertools to version 0.14.0
///
/// Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod
/// tempor incididunt ut labore et dolore magna aliqua.
///
/// Co-authored-by: Alice <alice@example.com>
/// Co-authored-by: Bob <bob@example.com>
/// Reviewed-by: Charlie <charlie@example.com>
/// Change-Id: I1234567890abcdef1234567890abcdef12345678
/// ```
///
/// In this case, there are four trailers: two `Co-authored-by` lines, one
/// `Reviewed-by` line, and one `Change-Id` line.
pub fn parse_description_trailers(body: &str) -> Vec<Trailer> {
    let (trailer, blank) = parse_trailers_and_blank(body);
    if blank {
        trailer
    } else {
        // no blank found, this means there was a single paragraph, so whatever
        // was found can't come from the trailer
        vec![]
    }
}

/// Parse the trailers from a trailer paragraph. This function behaves like
/// `parse_description_trailer`, except that it doesn't expect the body to
/// contain several paragraphs.
pub fn parse_trailers(body: &str) -> Vec<Trailer> {
    let (trailer, _) = parse_trailers_and_blank(body);
    trailer
}

fn parse_trailers_and_blank(body: &str) -> (Vec<Trailer>, bool) {
    // a trailer always comes at the end of a message; we can split the message
    // by newline, but we need to immediately reverse the order of the lines
    // to ensure we parse the trailer in an unambiguous manner; this avoids cases
    // where a colon in the body of the message is mistaken for a trailer
    let lines = body.trim_end().lines().rev();
    let trailer_re =
        regex::Regex::new(r"^([a-zA-Z0-9-]+) *: +(.+) *$").expect("trailer regex should be valid");
    let mut trailer: Vec<Trailer> = Vec::new();
    let mut multiline_value: Vec<&str> = Vec::new();
    let mut found_blank = false;
    for line in lines {
        if line.starts_with(' ') {
            multiline_value.push(line.trim());
        } else if let Some(caps) = trailer_re.captures(line) {
            let key = caps[1].trim().to_string();
            multiline_value.push(caps.get(2).unwrap().as_str());
            let value = multiline_value.iter().rev().join(" ");
            multiline_value.clear();
            trailer.push(Trailer { key, value });
        } else if line.trim().is_empty() {
            // end of the trailer
            found_blank = true;
            break;
        } else {
            // a non trailer in the trailer paragraph
            // the line is ignored, as well as the multiline value that may
            // have previously been accumulated
            multiline_value.clear();
        }
    }
    // reverse the insert order, since we parsed the trailer in reverse
    trailer.reverse();
    (trailer, found_blank)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_simple_trailers() {
        let body = r#"chore: update itertools to version 0.14.0

Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed
do eiusmod tempor incididunt ut labore et dolore magna aliqua.

Co-authored-by: Alice <alice@example.com>
Co-authored-by: Bob <bob@example.com>
Reviewed-by: Charlie <charlie@example.com>
Change-Id: I1234567890abcdef1234567890abcdef12345678"#;

        let trailer = parse_description_trailers(body);
        assert_eq!(trailer.len(), 4);

        assert_eq!(trailer[0].key, "Co-authored-by");
        assert_eq!(trailer[0].value, "Alice <alice@example.com>");

        assert_eq!(trailer[1].key, "Co-authored-by");
        assert_eq!(trailer[1].value, "Bob <bob@example.com>");

        assert_eq!(trailer[2].key, "Reviewed-by");
        assert_eq!(trailer[2].value, "Charlie <charlie@example.com>");

        assert_eq!(trailer[3].key, "Change-Id");
        assert_eq!(
            trailer[3].value,
            "I1234567890abcdef1234567890abcdef12345678"
        );
    }

    #[test]
    fn test_trailers_with_colon_in_body() {
        let body = r#"chore: update itertools to version 0.14.0

Summary: Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod
tempor incididunt ut labore et dolore magna aliqua.

Change-Id: I1234567890abcdef1234567890abcdef12345678"#;

        let trailer = parse_description_trailers(body);

        // should only have Change-Id
        assert_eq!(trailer.len(), 1);
        assert_eq!(trailer[0].key, "Change-Id");
    }

    #[test]
    fn test_multiline_trailer() {
        let body = r#"chore: update itertools to version 0.14.0

key: This is a very long value, with spaces and
  newlines in it."#;

        let trailer = parse_description_trailers(body);

        // should only have Change-Id
        assert_eq!(trailer.len(), 1);
        assert_eq!(trailer[0].key, "key");
        assert!(trailer[0].value.starts_with("This is"));
        assert!(trailer[0].value.ends_with("in it."));
    }

    #[test]
    fn test_ignore_line_in_trailer() {
        let body = r#"chore: update itertools to version 0.14.0

Signed-off-by: Random J Developer <random@developer.example.org>
[lucky@maintainer.example.org: struct foo moved from foo.c to foo.h]
Signed-off-by: Lucky K Maintainer <lucky@maintainer.example.org>
"#;

        let trailer = parse_description_trailers(body);
        assert_eq!(trailer.len(), 2);
    }

    #[test]
    fn test_trailers_with_single_line_description() {
        let body = r#"chore: update itertools to version 0.14.0"#;
        let trailer = parse_description_trailers(body);
        assert_eq!(trailer.len(), 0);
    }

    #[test]
    fn test_blank_line_after_trailer() {
        let body = r#"subject

foo: 1

"#;
        let trailer = parse_description_trailers(body);
        assert_eq!(trailer.len(), 1);
    }

    #[test]
    fn test_blank_line_inbetween() {
        let body = r#"subject

foo: 1

bar: 2
"#;
        let trailer = parse_description_trailers(body);
        assert_eq!(trailer.len(), 1);
    }

    #[test]
    fn test_no_blank_line() {
        let body = r#"subject: whatever
foo: 1
"#;
        let trailer = parse_description_trailers(body);
        assert_eq!(trailer.len(), 0);
    }

    #[test]
    fn test_whitespace_before_key() {
        let body = r#"subject

 foo: 1
"#;
        let trailer = parse_description_trailers(body);
        assert_eq!(trailer.len(), 0);
    }

    #[test]
    fn test_whitespace_after_key() {
        let body = r#"subject

foo : 1
"#;
        let trailer = parse_description_trailers(body);
        assert_eq!(trailer.len(), 1);
        assert_eq!(trailer[0].key, "foo");
    }

    #[test]
    fn test_whitespace_around_value() {
        let body = r#"subject

foo :  1 
"#;
        let trailer = parse_description_trailers(body);
        assert_eq!(trailer.len(), 1);
        assert_eq!(trailer[0].value, "1");
    }

    #[test]
    fn test_invalid_key() {
        let body = r#"subject

f_o_o: bar
"#;
        let trailer = parse_description_trailers(body);
        assert_eq!(trailer.len(), 0);
    }
}
