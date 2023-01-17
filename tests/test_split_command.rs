// Copyright 2022 The Jujutsu Authors
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

use crate::common::TestEnvironment;

pub mod common;

#[test]
fn test_split_by_paths() {
    let mut test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file1"), "foo").unwrap();
    std::fs::write(repo_path.join("file2"), "foo").unwrap();
    std::fs::write(repo_path.join("file3"), "foo").unwrap();

    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "commit_id.short()"]);
    insta::assert_snapshot!(stdout, @r###"
    @ 9d08ea8cac40
    o 000000000000
    "###);

    let edit_script = test_env.set_up_fake_editor();
    std::fs::write(
        edit_script,
        ["dump editor0", "next invocation\n", "dump editor1"].join("\0"),
    )
    .unwrap();
    let stdout = test_env.jj_cmd_success(&repo_path, &["split", "file2"]);
    insta::assert_snapshot!(stdout, @r###"
    First part: 5eebce1de3b0 (no description set)
    Second part: 45833353d94e (no description set)
    Working copy now at: 45833353d94e (no description set)
    "###);
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor0")).unwrap(), @r###"
    JJ: Enter commit description for the first part (parent).


    JJ: This commit contains the following changes:
    JJ:     A file2

    JJ: Lines starting with "JJ: " (like this one) will be removed.
    "###);
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor1")).unwrap(), @r###"
    JJ: Enter commit description for the second part (child).


    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:     A file3

    JJ: Lines starting with "JJ: " (like this one) will be removed.
    "###);

    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "commit_id.short()"]);
    insta::assert_snapshot!(stdout, @r###"
    @ 45833353d94e
    o 5eebce1de3b0
    o 000000000000
    "###);

    let stdout = test_env.jj_cmd_success(&repo_path, &["diff", "-s", "-r", "@-"]);
    insta::assert_snapshot!(stdout, @r###"
    A file2
    "###);
    let stdout = test_env.jj_cmd_success(&repo_path, &["diff", "-s"]);
    insta::assert_snapshot!(stdout, @r###"
    A file1
    A file3
    "###);
}
