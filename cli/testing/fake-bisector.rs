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

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::exit;

use clap::Parser;
use itertools::Itertools as _;

/// A fake diff-editor, useful for testing
#[derive(Parser, Debug)]
#[clap()]
struct Args {
    /// Fail if the given file doesn't exist
    #[arg(long)]
    require_file: Option<String>,
}

fn main() {
    let args: Args = Args::parse();
    let edit_script_path = PathBuf::from(env::var_os("BISECTION_SCRIPT").unwrap());
    let commit_to_test = env::var_os("JJ_BISECT_TARGET")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    println!("fake-bisector testing commit {commit_to_test}");
    let edit_script = fs::read_to_string(&edit_script_path).unwrap();

    if let Some(path) = args.require_file
        && !std::fs::exists(&path).unwrap()
    {
        exit(1)
    }

    let mut instructions = edit_script.split('\0').collect_vec();
    if let Some(pos) = instructions.iter().position(|&i| i == "next invocation\n") {
        // Overwrite the edit script. The next time `fake-bisector` is called, it will
        // only see the part after the `next invocation` command.
        fs::write(&edit_script_path, instructions[pos + 1..].join("\0")).unwrap();
        instructions.truncate(pos);
    }
    for instruction in instructions {
        let (command, payload) = instruction.split_once('\n').unwrap_or((instruction, ""));
        let parts = command.split(' ').collect_vec();
        match parts.as_slice() {
            [""] => {}
            ["abort"] => exit(127),
            ["skip"] => exit(125),
            ["fail"] => exit(1),
            ["fail-if-target-is", bad_target_commit] => {
                if commit_to_test == *bad_target_commit {
                    exit(1)
                }
            }
            ["write", path] => {
                fs::write(path, payload).unwrap_or_else(|_| panic!("Failed to write file {path}"));
            }
            ["jj", args @ ..] => {
                let jj_executable_path = PathBuf::from(env::var_os("JJ_EXECUTABLE_PATH").unwrap());
                let status = std::process::Command::new(&jj_executable_path)
                    .args(args)
                    .status()
                    .unwrap();
                if !status.success() {
                    eprintln!("fake-bisector: failed to run jj: {status:?}");
                    exit(1)
                }
            }
            _ => {
                eprintln!("fake-bisector: unexpected command: {command}");
                exit(1)
            }
        }
    }
}
