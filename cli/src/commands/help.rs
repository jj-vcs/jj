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

use std::fmt::Write as _;
use std::io::Write as _;

use clap::builder::PossibleValue;
use clap::builder::StyledStr;
use clap::error::ContextKind;
use crossterm::style::Stylize as _;
use itertools::Itertools as _;
use tracing::instrument;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::cli_error;
use crate::ui::Ui;

/// Print this message or the help of the given subcommand(s)
#[derive(clap::Args, Clone, Debug)]
pub(crate) struct HelpArgs {
    /// Print help for the subcommand(s)
    pub(crate) command: Vec<String>,

    /// Show help for keywords instead of commands
    #[arg(
        long,
        short = 'k',
        conflicts_with = "command",
        value_parser = KEYWORDS
            .iter()
            .map(|k| PossibleValue::new(k.name).help(k.description))
            .collect_vec()
    )]
    pub(crate) keyword: Option<String>,
}

#[instrument(skip_all)]
pub(crate) async fn cmd_help(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &HelpArgs,
) -> Result<(), CommandError> {
    if let Some(name) = &args.keyword {
        let keyword = find_keyword(name).expect("clap should check this with `value_parser`");
        ui.request_pager();
        write!(ui.stdout(), "{}", strip_html_comments(keyword.content))?;

        return Ok(());
    }

    let bin_name = command
        .string_args()
        .first()
        .map_or(command.app().get_name(), |name| name.as_ref());
    let mut args_to_get_command = vec![bin_name];
    args_to_get_command.extend(args.command.iter().map(|s| s.as_str()));

    let mut app = command.app().clone();
    // This propagates global arguments to subcommand, and generates error if
    // the subcommand doesn't exist.
    if let Err(err) = app.try_get_matches_from_mut(args_to_get_command) {
        if err.get(ContextKind::InvalidSubcommand).is_some() {
            return Err(err.into());
        } else {
            // `help log -- -r`, etc. shouldn't generate an argument error.
        }
    }
    let command = args
        .command
        .iter()
        .try_fold(&mut app, |cmd, name| cmd.find_subcommand_mut(name))
        .ok_or_else(|| cli_error(format!("Unknown command: {}", args.command.join(" "))))?;

    ui.request_pager();
    let help_text = command.render_long_help();
    if ui.color() {
        write!(ui.stdout(), "{}", help_text.ansi())?;
    } else {
        write!(ui.stdout(), "{help_text}")?;
    }
    Ok(())
}

#[derive(Clone)]
struct Keyword {
    name: &'static str,
    description: &'static str,
    content: &'static str,
}

// TODO: Add all documentation to keywords
//
// Maybe adding some code to build.rs to find all the docs files and build the
// `KEYWORDS` at compile time.
//
// It would be cool to follow the docs hierarchy somehow.
//
// One of the problems would be `config.md`, as it has the same name as a
// subcommand.
//
// TODO: Find a way to render markdown using ANSI escape codes.
//
// Maybe we can steal some ideas from https://github.com/jj-vcs/jj/pull/3130
const KEYWORDS: &[Keyword] = &[
    Keyword {
        name: "bookmarks",
        description: "Named pointers to revisions (similar to Git's branches)",
        content: include_str!(concat!("../../", env!("JJ_DOCS_DIR"), "bookmarks.md")),
    },
    Keyword {
        name: "config",
        description: "How and where to set configuration options",
        content: include_str!(concat!("../../", env!("JJ_DOCS_DIR"), "config.md")),
    },
    Keyword {
        name: "filesets",
        description: "A functional language for selecting a set of files",
        content: include_str!(concat!("../../", env!("JJ_DOCS_DIR"), "filesets.md")),
    },
    Keyword {
        name: "glossary",
        description: "Definitions of various terms",
        content: include_str!(concat!("../../", env!("JJ_DOCS_DIR"), "glossary.md")),
    },
    Keyword {
        name: "revsets",
        description: "A functional language for selecting a set of revision",
        content: include_str!(concat!("../../", env!("JJ_DOCS_DIR"), "revsets.md")),
    },
    Keyword {
        name: "templates",
        description: "A functional language to customize command output",
        content: include_str!(concat!("../../", env!("JJ_DOCS_DIR"), "templates.md")),
    },
    Keyword {
        name: "tutorial",
        description: "Show a tutorial to get started with jj",
        content: include_str!(concat!("../../", env!("JJ_DOCS_DIR"), "tutorial.md")),
    },
];

fn find_keyword(name: &str) -> Option<&Keyword> {
    KEYWORDS.iter().find(|keyword| keyword.name == name)
}

/// Removes HTML comments (`<!-- ... -->`) from Markdown docs, since they are
/// notes for editors and tools rather than for readers. Comments inside fenced
/// code blocks are kept. A line that only contained a comment is dropped along
/// with a following blank line, so paragraphs stay separated by one blank line.
/// An unclosed comment is kept as is, along with everything after it.
fn strip_html_comments(content: &str) -> String {
    let mut output = String::with_capacity(content.len());
    let mut fence = None;
    let mut after_blank_line = true;
    let mut rest = content;
    while !rest.is_empty() {
        let (line, after_line) = split_first_line(rest);
        if let Some(marker) = fence {
            if fence_marker(line) == Some(marker) {
                fence = None;
            }
        } else if let Some(marker) = fence_marker(line) {
            fence = Some(marker);
        } else if line.contains("<!--") {
            let Some((kept, after_comments)) = strip_comments(rest) else {
                output.push_str(rest);
                break;
            };
            rest = after_comments;
            if !kept.trim().is_empty() {
                output.push_str(&kept);
                after_blank_line = false;
            } else if after_blank_line {
                let (next, after_next) = split_first_line(rest);
                if next.trim().is_empty() {
                    rest = after_next;
                }
            }
            continue;
        }
        output.push_str(line);
        after_blank_line = line.trim().is_empty();
        rest = after_line;
    }
    output
}

/// Returns the marker if `line` opens or closes a fenced code block.
fn fence_marker(line: &str) -> Option<&'static str> {
    let trimmed = line.trim_start();
    ["```", "~~~"]
        .into_iter()
        .find(|marker| trimmed.starts_with(marker))
}

/// Splits `text` after its first line break, or at the end if there is none.
fn split_first_line(text: &str) -> (&str, &str) {
    text.split_at(text.find('\n').map_or(text.len(), |i| i + 1))
}

/// Strips comments from the first line of `text`, continuing onto later lines
/// while a comment is open. Returns the kept text and the text after the line
/// where the last comment ends, or `None` if a comment is never closed.
fn strip_comments(text: &str) -> Option<(String, &str)> {
    let mut kept = String::new();
    let mut rest = text;
    loop {
        let (line, after_line) = split_first_line(rest);
        let Some(start) = line.find("<!--") else {
            kept.push_str(line);
            return Some((kept, after_line));
        };
        kept.push_str(&line[..start]);
        let comment = &rest[start + "<!--".len()..];
        let end = comment.find("-->")?;
        rest = &comment[end + "-->".len()..];
    }
}

pub fn show_keyword_hint_after_help() -> StyledStr {
    let mut ret = StyledStr::new();
    writeln!(
        ret,
        "{} lists available keywords. Use {} to show help for one of these keywords.",
        "'jj help --help'".bold(),
        "'jj help -k'".bold(),
    )
    .unwrap();
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_html_comments() {
        let input = indoc::indoc! {"
            <!-- A comment on the first line. -->

            Before.

            <!-- A standalone comment. -->
            <!-- Another one. -->

            Inline <!-- comment --> text.
            <!-- Between lines. -->
            Same paragraph.

            <!--- A comment that
             spans lines. -->

            Inline <!-- comment
            over lines --> kept.
            Next line.

            <!-- Before a code block. -->
            ```
            code
            ```

            Paragraph after code.

            ```
            <!-- kept in code -->
            ```

            ~~~
            ```
            <!-- kept in code -->
            ~~~
            After.
        "};
        insta::assert_snapshot!(strip_html_comments(input), @"
        Before.

        Inline  text.
        Same paragraph.

        Inline  kept.
        Next line.

        ```
        code
        ```

        Paragraph after code.

        ```
        <!-- kept in code -->
        ```

        ~~~
        ```
        <!-- kept in code -->
        ~~~
        After.
        ");
    }

    #[test]
    fn test_strip_html_comments_crlf() {
        let input = "Before.\r\n\r\n<!-- comment -->\r\n\r\nAfter.\r\n";
        assert_eq!(strip_html_comments(input), "Before.\r\n\r\nAfter.\r\n");
    }

    #[test]
    fn test_strip_html_comments_unclosed() {
        let input = "Before. <!-- unclosed\n\nAfter.\n";
        assert_eq!(strip_html_comments(input), input);
    }
}
