// Copyright 2025 The Jujutsu Authors
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

use std::io::Write as _;

fn main() {
    let mut args_os = std::env::args_os().skip(1).peekable();
    while let Some(arg_os) = args_os.next() {
        std::io::stdout()
            .write_all(arg_os.as_encoded_bytes())
            .unwrap_or_else(|e| {
                panic!(
                    "Failed to write the argument \"{}\" to stdout: {e}",
                    arg_os.display()
                )
            });
        if args_os.peek().is_some() {
            write!(std::io::stdout(), " ").unwrap_or_else(|e| {
                panic!("Failed to write the space separator to stdout: {e}");
            });
        }
    }
    writeln!(std::io::stdout())
        .unwrap_or_else(|e| panic!("Failed to write the terminating newline to stdout: {e}"));
}
