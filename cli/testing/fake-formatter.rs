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

use std::fs::OpenOptions;
use std::io::Read as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use bstr::ByteSlice as _;
use clap::Parser;
use itertools::Itertools as _;
#[cfg(unix)]
use nix::sys::signal;

/// A fake code formatter, useful for testing
///
/// `fake-formatter` is similar to `cat`.
/// `fake-formatter --reverse` is similar to `rev` (not `tac`).
/// `fake-formatter --stdout foo` is similar to `echo foo`.
/// `fake-formatter --stdout foo --stderr bar --fail` is similar to
///   `echo foo; echo bar >&2; false`.
/// `fake-formatter --tee foo` is similar to `tee foo`).
///
/// This program acts as a portable alternative to that class of shell commands.
#[derive(Parser, Debug)]
struct Args {
    /// Exit with non-successful status.
    #[arg(long, default_value_t = false)]
    fail: bool,

    /// Abort instead of exiting
    #[arg(long, default_value_t = false, conflicts_with = "fail")]
    abort: bool,

    /// Reverse the characters in each line when reading stdin.
    #[arg(long, default_value_t = false)]
    reverse: bool,

    /// Convert all characters to uppercase when reading stdin.
    #[arg(long, default_value_t = false)]
    uppercase: bool,

    /// Convert all characters to lowercase when reading stdin.
    #[arg(long, default_value_t = false)]
    lowercase: bool,

    /// Adds a line to the end of the file
    #[arg(long)]
    append: Option<String>,

    /// Write this string to stdout, and ignore stdin.
    #[arg(long)]
    stdout: Option<String>,

    /// Write this string to stderr.
    #[arg(long)]
    stderr: Option<String>,

    /// Duplicate stdout into this file.
    #[arg(long)]
    tee: Option<PathBuf>,

    /// Read one byte at a time, and send one byte to the stdout before reading
    /// the next byte.
    #[arg(long, default_value_t = false)]
    byte_mode: bool,

    /// Format lines from first to last inclusive [f, l] (1-indexed)
    /// For example `--line-ranges 1-2,4-5` or `--line-ranges 1-2 --line-ranges
    /// 4-5` formats lines 1 and 2, and lines 4 and 5.
    #[arg(long, value_delimiter = ',')]
    line_ranges: Vec<String>,

    /// Split even lines into two lines. Used only with `--line-ranges`.
    /// Replicating formatters that expand beyond the line ranges.
    /// For example, "abcd" becomes "ab\ncd".
    #[arg(long, default_value_t = false, requires = "line_ranges")]
    split_even_lines: bool,
}

/// Represents an inclusive range of lines.
#[derive(Debug)]
struct LineRange {
    first: usize,
    last: usize,
}

impl LineRange {
    /// Creates a new line range from a string slice.
    fn from_str(s: &str) -> Self {
        let (first, last) = s.split_once('-').unwrap();
        Self {
            first: first.parse::<usize>().unwrap(),
            last: last.parse::<usize>().unwrap(),
        }
    }

    /// Checks if the line range contains the given line number (1-indexed and
    /// inclusive).
    fn contains(&self, line_num: usize) -> bool {
        line_num >= self.first && line_num <= self.last
    }
}

fn split_even_line(line: &str) -> String {
    let line_len = if line.ends_with('\n') {
        line.len() - 1
    } else {
        line.len()
    };
    if line_len % 2 == 0 {
        let (first, second) = line.split_at(line_len / 2);
        format!("{first}\n{second}")
    } else {
        line.to_owned()
    }
}

fn main() -> ExitCode {
    let args: Args = Args::parse();
    // Code formatters tend to print errors before printing the result.
    if let Some(data) = args.stderr {
        eprint!("{data}");
    }
    let stdout = if let Some(data) = args.stdout {
        // Other content-altering flags don't apply to --stdout.
        assert!(!args.reverse);
        assert!(!args.uppercase);
        assert!(!args.lowercase);
        assert!(args.append.is_none());
        print!("{data}");
        data
    } else if args.byte_mode {
        assert!(!args.reverse);
        assert!(args.append.is_none());

        let mut stdout = vec![];
        #[expect(clippy::unbuffered_bytes)]
        for byte in std::io::stdin().bytes() {
            let byte = byte.expect("Failed to read from stdin");
            let output = if args.uppercase {
                byte.to_ascii_uppercase()
            } else if args.lowercase {
                assert!(!args.uppercase);
                byte.to_ascii_lowercase()
            } else {
                byte
            };
            stdout.push(output);
            std::io::stdout()
                .write_all(&[output])
                .expect("Failed to write to stdout");
        }
        stdout
            .to_str()
            .expect("Output is not a valid UTF-8 string")
            .to_owned()
    } else {
        let line_ranges: Vec<LineRange> = if !args.line_ranges.is_empty() {
            args.line_ranges
                .iter()
                .map(|rng| LineRange::from_str(rng))
                .collect_vec()
        } else {
            vec![LineRange {
                first: 1,
                last: usize::MAX,
            }]
        };

        let mut input = vec![];
        std::io::stdin()
            .read_to_end(&mut input)
            .expect("Failed to read from stdin");
        let mut stdout = input
            .lines_with_terminator()
            .enumerate()
            .map(|(i, line)| {
                let line = line
                    .to_str()
                    .expect("The input is not valid UTF-8 string")
                    .to_owned();

                let line_num = i + 1;
                let in_range = line_ranges
                    .iter()
                    .any(|line_range| line_range.contains(line_num));
                if !in_range {
                    return line;
                }

                let line = if args.reverse {
                    line.chars().rev().collect()
                } else {
                    line
                };
                let line = if args.split_even_lines {
                    split_even_line(&line)
                } else {
                    line
                };
                if args.uppercase {
                    assert!(!args.lowercase);
                    line.to_uppercase()
                } else if args.lowercase {
                    assert!(!args.uppercase);
                    line.to_lowercase()
                } else {
                    line
                }
            })
            .join("");
        if let Some(line) = args.append {
            stdout.push_str(&line);
        }
        print!("{stdout}");
        stdout
    };
    if let Some(path) = args.tee {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        write!(file, "{stdout}").unwrap();
    }
    if args.abort {
        // Coredump generation varies by UNIX and is irrelevant to tests
        // Prefer raising SIGTERM to crash without dumping core
        #[cfg(unix)]
        let _ = signal::raise(signal::Signal::SIGTERM);
        std::process::abort()
    } else if args.fail {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
