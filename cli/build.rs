// Copyright 2023 The Jujutsu Authors
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

use std::path::Path;
use std::process::Command;

const GIT_HEAD_PATH: &str = "../.git/HEAD";
const JJ_OP_HEADS_PATH: &str = "../.jj/repo/op_heads/heads";

fn main() {
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();

    println!("cargo:rerun-if-env-changed=NIX_JJ_GIT_HASH");
    let git_hash = get_git_hash_from_nix().or_else(|| {
        if Path::new(GIT_HEAD_PATH).exists() {
            // In colocated workspace, .git/HEAD should reflect the working-copy parent.
            println!("cargo:rerun-if-changed={GIT_HEAD_PATH}");
        } else if Path::new(JJ_OP_HEADS_PATH).exists() {
            // op_heads changes when working-copy files are mutated, which is way more
            // frequent than .git/HEAD.
            println!("cargo:rerun-if-changed={JJ_OP_HEADS_PATH}");
        }
        get_git_hash_from_jj().or_else(get_git_hash_from_git)
    });

    if let Some(git_hash) = git_hash {
        println!("cargo:rustc-env=JJ_VERSION={version}-{git_hash}");
    } else {
        println!("cargo:rustc-env=JJ_VERSION={version}");
    }

    let docs_symlink_path = Path::new("docs");
    println!("cargo:rerun-if-changed={}", docs_symlink_path.display());
    if docs_symlink_path.join("index.md").exists() {
        println!("cargo:rustc-env=JJ_DOCS_DIR=docs/");
    } else {
        println!("cargo:rustc-env=JJ_DOCS_DIR=../docs/");
    }
}

fn get_git_hash_from_nix() -> Option<String> {
    std::env::var("NIX_JJ_GIT_HASH")
        .ok()
        .filter(|s| !s.is_empty())
}

fn get_git_hash_from_jj() -> Option<String> {
    Command::new("jj")
        .args([
            "--ignore-working-copy",
            "--color=never",
            "log",
            "--no-graph",
            "-r=@-",
            "-T=commit_id ++ '-'",
        ])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            let mut parent_commits = String::from_utf8(output.stdout).unwrap();
            // If a development version of `jj` is compiled at a merge commit, this will
            // result in several commit ids separated by `-`s.
            parent_commits.truncate(parent_commits.trim_end_matches('-').len());
            parent_commits
        })
}

fn get_git_hash_from_git() -> Option<String> {
    Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            str::from_utf8(&output.stdout)
                .unwrap()
                .trim_end()
                .to_owned()
        })
}
