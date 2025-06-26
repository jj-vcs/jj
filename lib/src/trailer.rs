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
use thiserror::Error;

/// A key-value pair representing a trailer in a commit message, of the
/// form `Key: Value`.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Trailer {
    /// trailer key
    pub key: String,
    /// trailer value
    ///
    /// It is trimmed at the start and the end but includes new line characters
    /// (\n) and multi-line escape chars ( ) for multi line values.
    pub value: String,
}

#[expect(missing_docs)]
#[derive(Error, Debug)]
pub enum TrailerParseError {
    #[error("The trailer paragraph can't contain a blank line")]
    BlankLine,
    #[error("Invalid trailer line: {line}")]
    NonTrailerLine { line: String },
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
pub fn parse_description_trailers(body: &str, separators: &str) -> Vec<Trailer> {
    let (trailers, blank, found_git_trailer, non_trailer) = parse_trailers_impl(body, separators);
    if !blank {
        // no blank found, this means there was a single paragraph, so whatever
        // was found can't come from the trailer
        vec![]
    } else if non_trailer.is_some() && !found_git_trailer {
        // at least one non trailer line was found in the trailers paragraph
        // the trailers are considered as trailers only if there is a predefined
        // trailers from git
        vec![]
    } else {
        trailers
    }
}

/// Parse the trailers from a trailer paragraph. This function behaves like
/// `parse_description_trailer`, but will return an error if a blank or
/// non trailer line is found.
pub fn parse_trailers(body: &str, separators: &str) -> Result<Vec<Trailer>, TrailerParseError> {
    let (trailers, blank, _, non_trailer) = parse_trailers_impl(body, separators);
    if blank {
        return Err(TrailerParseError::BlankLine);
    }
    if let Some(line) = non_trailer {
        return Err(TrailerParseError::NonTrailerLine { line });
    }
    Ok(trailers)
}

fn parse_trailers_impl(body: &str, separators: &str) -> (Vec<Trailer>, bool, bool, Option<String>) {
    // a trailer always comes at the end of a message; we can split the message
    // by newline, but we need to immediately reverse the order of the lines
    // to ensure we parse the trailer in an unambiguous manner; this avoids cases
    // where a colon in the body of the message is mistaken for a trailer
    let lines = body.trim_ascii_end().lines().rev();
    let separators_re = regex::escape(separators);
    let trailer_re = regex::Regex::new(&format!("^([a-zA-Z0-9-]+?) *[{separators_re}] *(.*)$"))
        .expect("Trailer regex should be valid");
    let mut trailers: Vec<Trailer> = Vec::new();
    let mut multiline_value = vec![];
    let mut found_blank = false;
    let mut found_git_trailer = false;
    let mut non_trailer_line = None;
    for line in lines {
        if line.starts_with(' ') {
            multiline_value.push(line);
        } else if let Some(groups) = trailer_re.captures(line) {
            let key = groups[1].to_string();
            multiline_value.push(groups.get(2).unwrap().as_str());
            // trim the end of the multiline value
            // the start is already trimmed with the regex
            multiline_value[0] = multiline_value[0].trim_ascii_end();
            let value = multiline_value.iter().rev().join("\n");
            multiline_value.clear();
            if key == "Signed-off-by" {
                found_git_trailer = true;
            }
            trailers.push(Trailer { key, value });
        } else if line.starts_with("(cherry picked from commit ") {
            found_git_trailer = true;
            non_trailer_line = Some(line.to_owned());
            multiline_value.clear();
        } else if line.trim_ascii().is_empty() {
            // end of the trailer
            found_blank = true;
            break;
        } else {
            // a non trailer in the trailer paragraph
            // the line is ignored, as well as the multiline value that may
            // have previously been accumulated
            multiline_value.clear();
            non_trailer_line = Some(line.to_owned());
        }
    }
    // reverse the insert order, since we parsed the trailer in reverse
    trailers.reverse();
    (trailers, found_blank, found_git_trailer, non_trailer_line)
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_simple_trailers() {
        let descriptions = indoc! {r#"
            chore: update itertools to version 0.14.0

            Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed
            do eiusmod tempor incididunt ut labore et dolore magna aliqua.

            Co-authored-by: Alice <alice@example.com>
            Co-authored-by: Bob <bob@example.com>
            Reviewed-by: Charlie <charlie@example.com>
            Change-Id: I1234567890abcdef1234567890abcdef12345678
        "#};

        let trailers = parse_description_trailers(descriptions, ":");
        assert_eq!(trailers.len(), 4);

        assert_eq!(trailers[0].key, "Co-authored-by");
        assert_eq!(trailers[0].value, "Alice <alice@example.com>");

        assert_eq!(trailers[1].key, "Co-authored-by");
        assert_eq!(trailers[1].value, "Bob <bob@example.com>");

        assert_eq!(trailers[2].key, "Reviewed-by");
        assert_eq!(trailers[2].value, "Charlie <charlie@example.com>");

        assert_eq!(trailers[3].key, "Change-Id");
        assert_eq!(
            trailers[3].value,
            "I1234567890abcdef1234567890abcdef12345678"
        );
    }

    #[test]
    fn test_trailers_with_colon_in_body() {
        let descriptions = indoc! {r#"
            chore: update itertools to version 0.14.0

            Summary: Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod
            tempor incididunt ut labore et dolore magna aliqua.

            Change-Id: I1234567890abcdef1234567890abcdef12345678
        "#};

        let trailers = parse_description_trailers(descriptions, ":");

        // should only have Change-Id
        assert_eq!(trailers.len(), 1);
        assert_eq!(trailers[0].key, "Change-Id");
    }

    #[test]
    fn test_multiline_trailer() {
        let description = indoc! {r#"
            chore: update itertools to version 0.14.0

            key: This is a very long value, with spaces and
              newlines in it.
        "#};

        let trailers = parse_description_trailers(description, ":");

        // should only have Change-Id
        assert_eq!(trailers.len(), 1);
        assert_eq!(trailers[0].key, "key");
        assert_eq!(
            trailers[0].value,
            indoc! {r"
                This is a very long value, with spaces and
                  newlines in it."}
        );
    }

    #[test]
    fn test_ignore_line_in_trailer() {
        let description = indoc! {r#"
            chore: update itertools to version 0.14.0

            Signed-off-by: Random J Developer <random@developer.example.org>
            [lucky@maintainer.example.org: struct foo moved from foo.c to foo.h]
            Signed-off-by: Lucky K Maintainer <lucky@maintainer.example.org>
        "#};

        let trailers = parse_description_trailers(description, ":");
        assert_eq!(trailers.len(), 2);
    }

    #[test]
    fn test_trailers_with_single_line_description() {
        let description = r#"chore: update itertools to version 0.14.0"#;
        let trailers = parse_description_trailers(description, ":");
        assert_eq!(trailers.len(), 0);
    }

    #[test]
    fn test_parse_trailers() {
        let trailers_txt = indoc! {r#"
            foo: 1
            bar: 2
        "#};
        let res = parse_trailers(trailers_txt, ":");
        let trailers = res.expect("trailers to be valid");
        assert_eq!(trailers.len(), 2);
        assert_eq!(trailers[0].key, "foo");
        assert_eq!(trailers[0].value, "1");
        assert_eq!(trailers[1].key, "bar");
        assert_eq!(trailers[1].value, "2");
    }

    #[test]
    fn test_blank_line_in_trailers() {
        let trailers = indoc! {r#"
            foo: 1

            foo: 2
        "#};
        let res = parse_trailers(trailers, ":");
        assert!(matches!(res, Err(TrailerParseError::BlankLine)));
    }

    #[test]
    fn test_non_trailer_line_in_trailers() {
        let trailers = indoc! {r#"
            bar
            foo: 1
        "#};
        let res = parse_trailers(trailers, ":");
        assert!(matches!(
            res,
            Err(TrailerParseError::NonTrailerLine { line: _ })
        ));
    }

    #[test]
    fn test_blank_line_after_trailer() {
        let description = indoc! {r#"
            subject

            foo: 1

        "#};
        let trailers = parse_description_trailers(description, ":");
        assert_eq!(trailers.len(), 1);
    }

    #[test]
    fn test_blank_line_inbetween() {
        let description = indoc! {r#"
            subject

            foo: 1

            bar: 2
        "#};
        let trailers = parse_description_trailers(description, ":");
        assert_eq!(trailers.len(), 1);
    }

    #[test]
    fn test_no_blank_line() {
        let description = indoc! {r#"
            subject: whatever
            foo: 1
        "#};
        let trailers = parse_description_trailers(description, ":");
        assert_eq!(trailers.len(), 0);
    }

    #[test]
    fn test_whitespace_before_key() {
        let description = indoc! {r#"
            subject

             foo: 1
        "#};
        let trailers = parse_description_trailers(description, ":");
        assert_eq!(trailers.len(), 0);
    }

    #[test]
    fn test_whitespace_after_key() {
        let description = indoc! {r#"
            subject

            foo : 1
        "#};
        let trailers = parse_description_trailers(description, ":");
        assert_eq!(trailers.len(), 1);
        assert_eq!(trailers[0].key, "foo");
    }

    #[test]
    fn test_whitespace_around_value() {
        let description = indoc! {"
            subject

            foo:  1\x20
        "};
        let trailers = parse_description_trailers(description, ":");
        assert_eq!(trailers.len(), 1);
        assert_eq!(trailers[0].value, "1");
    }

    #[test]
    fn test_whitespace_around_multiline_value() {
        let description = indoc! {"
            subject

            foo:  1\x20
             2\x20
        "};
        let trailers = parse_description_trailers(description, ":");
        assert_eq!(trailers.len(), 1);
        assert_eq!(trailers[0].value, "1 \n 2");
    }

    #[test]
    fn test_whitespace_around_multiliple_trailers() {
        let description = indoc! {"
            subject

            foo:  1\x20
            bar:  2\x20
        "};
        let trailers = parse_description_trailers(description, ":");
        assert_eq!(trailers.len(), 2);
        assert_eq!(trailers[0].value, "1");
        assert_eq!(trailers[1].value, "2");
    }

    #[test]
    fn test_no_whitespace_before_value() {
        let description = indoc! {r#"
            subject

            foo:1
        "#};
        let trailers = parse_description_trailers(description, ":");
        assert_eq!(trailers.len(), 1);
    }

    #[test]
    fn test_empty_value() {
        let description = indoc! {r#"
            subject

            foo:
        "#};
        let trailers = parse_description_trailers(description, ":");
        assert_eq!(trailers.len(), 1);
    }

    #[test]
    fn test_invalid_key() {
        let description = indoc! {r#"
            subject

            f_o_o: bar
        "#};
        let trailers = parse_description_trailers(description, ":");
        assert_eq!(trailers.len(), 0);
    }

    #[test]
    fn test_content_after_trailer() {
        let description = indoc! {r#"
            subject

            foo: bar
            baz
        "#};
        let trailers = parse_description_trailers(description, ":");
        assert_eq!(trailers.len(), 0);
    }

    #[test]
    fn test_invalid_content_after_trailer() {
        let description = indoc! {r#"
            subject

            foo: bar

            baz
        "#};
        let trailers = parse_description_trailers(description, ":");
        assert_eq!(trailers.len(), 0);
    }

    #[test]
    fn test_empty_description() {
        let description = "";
        let trailers = parse_description_trailers(description, ":");
        assert_eq!(trailers.len(), 0);
    }

    #[test]
    fn test_cherry_pick_trailer() {
        let description = indoc! {r#"
            subject

            some non-trailer text
            foo: bar
            (cherry picked from commit 72bb9f9cf4bbb6bbb11da9cda4499c55c44e87b9)
        "#};
        let trailers = parse_description_trailers(description, ":");
        assert_eq!(trailers.len(), 1);
        assert_eq!(trailers[0].key, "foo");
        assert_eq!(trailers[0].value, "bar");
    }

    #[test]
    fn test_trailer_separators() {
        let description = indoc! {r#"
            subject

            BUG-13
            key|value
            foo: bar
            toto > titi
        "#};
        let trailers = parse_description_trailers(description, ":-|>");
        assert_eq!(trailers.len(), 4);
        assert_eq!(trailers[0].key, "BUG");
        assert_eq!(trailers[0].value, "13");
        assert_eq!(trailers[1].key, "key");
        assert_eq!(trailers[1].value, "value");
        assert_eq!(trailers[2].key, "foo");
        assert_eq!(trailers[2].value, "bar");
        assert_eq!(trailers[3].key, "toto");
        assert_eq!(trailers[3].value, "titi");
    }
}
