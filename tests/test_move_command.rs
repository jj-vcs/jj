// Copyright 2022 Google LLC
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

use itertools::Itertools;

use crate::common::TestEnvironment;

pub mod common;

#[test]
fn test_move() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    // Create history like this:
    // F
    // |
    // E C
    // | |
    // D B
    // |/
    // A
    //
    // When moving changes between e.g. C and F, we should not get unrelated changes
    // from B and D.
    test_env.jj_cmd_success(&repo_path, &["branch", "a"]);
    std::fs::write(repo_path.join("file1"), "a\n").unwrap();
    std::fs::write(repo_path.join("file2"), "a\n").unwrap();
    std::fs::write(repo_path.join("file3"), "a\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["new"]);
    test_env.jj_cmd_success(&repo_path, &["branch", "b"]);
    std::fs::write(repo_path.join("file3"), "b\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["new"]);
    test_env.jj_cmd_success(&repo_path, &["branch", "c"]);
    std::fs::write(repo_path.join("file1"), "c\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["co", "a"]);
    test_env.jj_cmd_success(&repo_path, &["new"]);
    test_env.jj_cmd_success(&repo_path, &["branch", "d"]);
    std::fs::write(repo_path.join("file3"), "d\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["new"]);
    test_env.jj_cmd_success(&repo_path, &["branch", "e"]);
    std::fs::write(repo_path.join("file2"), "e\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["new"]);
    test_env.jj_cmd_success(&repo_path, &["branch", "f"]);
    std::fs::write(repo_path.join("file2"), "f\n").unwrap();
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @ 0d7353584003 f
    o e9515f21068c e
    o bdd835cae844 d
    | o caa4d0b23201 c
    | o 55171e33db26 b
    |/  
    o 3db0a2f5b535 a
    o 000000000000 
    "###);

    // Errors out without arguments
    let stderr = test_env.jj_cmd_failure(&repo_path, &["move"]);
    insta::assert_snapshot!(stderr.lines().take(2).join("\n"), @r###"
    error: The following required arguments were not provided:
        <--from <FROM>|--to <TO>>
    "###);
    // Errors out if source and destination are the same
    let stderr = test_env.jj_cmd_failure(&repo_path, &["move", "--to", "@"]);
    insta::assert_snapshot!(stderr, @"Error: Source and destination cannot be the same.
");

    // Can move from sibling, which results in the source being abandoned
    let stdout = test_env.jj_cmd_success(&repo_path, &["move", "--from", "c"]);
    insta::assert_snapshot!(stdout, @r###"
    Working copy now at: 1c03e3d3c63f 
    Added 0 files, modified 1 files, removed 0 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @ 1c03e3d3c63f f
    o e9515f21068c e
    o bdd835cae844 d
    | o 55171e33db26 b c
    |/  
    o 3db0a2f5b535 a
    o 000000000000 
    "###);
    // The change from the source has been applied
    let stdout = test_env.jj_cmd_success(&repo_path, &["print", "file1"]);
    insta::assert_snapshot!(stdout, @"c
");
    // File `file2`, which was not changed in source, is unchanged
    let stdout = test_env.jj_cmd_success(&repo_path, &["print", "file2"]);
    insta::assert_snapshot!(stdout, @"f
");

    // Can move from ancestor
    test_env.jj_cmd_success(&repo_path, &["undo"]);
    let stdout = test_env.jj_cmd_success(&repo_path, &["move", "--from", "@--"]);
    insta::assert_snapshot!(stdout, @"Working copy now at: c8d83075e8c2 
");
    // The change has been removed from the source (the change pointed to by 'd'
    // became empty and was abandoned)
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @ c8d83075e8c2 f
    o 2c50bfc59c68 e
    | o caa4d0b23201 c
    | o 55171e33db26 b
    |/  
    o 3db0a2f5b535 a d
    o 000000000000 
    "###);
    // The change from the source has been applied (the file contents were already
    // "f", as is typically the case when moving changes from an ancestor)
    let stdout = test_env.jj_cmd_success(&repo_path, &["print", "file2"]);
    insta::assert_snapshot!(stdout, @"f
");

    // Can move from descendant
    test_env.jj_cmd_success(&repo_path, &["undo"]);
    let stdout = test_env.jj_cmd_success(&repo_path, &["move", "--from", "e", "--to", "d"]);
    insta::assert_snapshot!(stdout, @r###"
    Rebased 1 descendant commits
    Working copy now at: 2b723b1d6033 
    "###);
    // The change has been removed from the source (the change pointed to by 'e'
    // became empty and was abandoned)
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @ 2b723b1d6033 f
    o 4293930d6333 d e
    | o caa4d0b23201 c
    | o 55171e33db26 b
    |/  
    o 3db0a2f5b535 a
    o 000000000000 
    "###);
    // The change from the source has been applied
    let stdout = test_env.jj_cmd_success(&repo_path, &["print", "file2", "-r", "d"]);
    insta::assert_snapshot!(stdout, @"e
");
}

#[test]
fn test_move_partial() {
    let mut test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let repo_path = test_env.env_root().join("repo");

    // Create history like this:
    //   C
    //   |
    // D B
    // |/
    // A
    test_env.jj_cmd_success(&repo_path, &["branch", "a"]);
    std::fs::write(repo_path.join("file1"), "a\n").unwrap();
    std::fs::write(repo_path.join("file2"), "a\n").unwrap();
    std::fs::write(repo_path.join("file3"), "a\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["new"]);
    test_env.jj_cmd_success(&repo_path, &["branch", "b"]);
    std::fs::write(repo_path.join("file3"), "b\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["new"]);
    test_env.jj_cmd_success(&repo_path, &["branch", "c"]);
    std::fs::write(repo_path.join("file1"), "c\n").unwrap();
    std::fs::write(repo_path.join("file2"), "c\n").unwrap();
    test_env.jj_cmd_success(&repo_path, &["co", "a"]);
    test_env.jj_cmd_success(&repo_path, &["new"]);
    test_env.jj_cmd_success(&repo_path, &["branch", "d"]);
    std::fs::write(repo_path.join("file3"), "d\n").unwrap();
    // Test the setup
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @ bdd835cae844 d
    | o 5028db694b6b c
    | o 55171e33db26 b
    |/  
    o 3db0a2f5b535 a
    o 000000000000 
    "###);

    let edit_script = test_env.set_up_fake_diff_editor();

    // If we don't make any changes in the diff-editor, the whole change is moved
    std::fs::write(&edit_script, "").unwrap();
    let stdout = test_env.jj_cmd_success(&repo_path, &["move", "-i", "--from", "c"]);
    insta::assert_snapshot!(stdout, @r###"
    Working copy now at: 71b69e433fbc 
    Added 0 files, modified 2 files, removed 0 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @ 71b69e433fbc d
    | o 55171e33db26 b c
    |/  
    o 3db0a2f5b535 a
    o 000000000000 
    "###);
    // The changes from the source has been applied
    let stdout = test_env.jj_cmd_success(&repo_path, &["print", "file1"]);
    insta::assert_snapshot!(stdout, @"c
");
    let stdout = test_env.jj_cmd_success(&repo_path, &["print", "file2"]);
    insta::assert_snapshot!(stdout, @"c
");
    // File `file3`, which was not changed in source, is unchanged
    let stdout = test_env.jj_cmd_success(&repo_path, &["print", "file3"]);
    insta::assert_snapshot!(stdout, @"d
");

    // Can move only part of the change in interactive mode
    test_env.jj_cmd_success(&repo_path, &["undo"]);
    std::fs::write(&edit_script, "reset file2").unwrap();
    let stdout = test_env.jj_cmd_success(&repo_path, &["move", "-i", "--from", "c"]);
    insta::assert_snapshot!(stdout, @r###"
    Working copy now at: 63f1a6e96edb 
    Added 0 files, modified 1 files, removed 0 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @ 63f1a6e96edb d
    | o d027c6e3e6bc c
    | o 55171e33db26 b
    |/  
    o 3db0a2f5b535 a
    o 000000000000 
    "###);
    // The selected change from the source has been applied
    let stdout = test_env.jj_cmd_success(&repo_path, &["print", "file1"]);
    insta::assert_snapshot!(stdout, @"c
");
    // The unselected change from the source has not been applied
    let stdout = test_env.jj_cmd_success(&repo_path, &["print", "file2"]);
    insta::assert_snapshot!(stdout, @"a
");
    // File `file3`, which was changed in source's parent, is unchanged
    let stdout = test_env.jj_cmd_success(&repo_path, &["print", "file3"]);
    insta::assert_snapshot!(stdout, @"d
");

    // Can move only part of the change from a sibling in non-interactive mode
    test_env.jj_cmd_success(&repo_path, &["undo"]);
    // Clear the script so we know it won't be used
    std::fs::write(&edit_script, "").unwrap();
    let stdout = test_env.jj_cmd_success(&repo_path, &["move", "--from", "c", "file1"]);
    insta::assert_snapshot!(stdout, @r###"
    Working copy now at: 17c2e6632cc5 
    Added 0 files, modified 1 files, removed 0 files
    "###);
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    @ 17c2e6632cc5 d
    | o 6a3ae047a03e c
    | o 55171e33db26 b
    |/  
    o 3db0a2f5b535 a
    o 000000000000 
    "###);
    // The selected change from the source has been applied
    let stdout = test_env.jj_cmd_success(&repo_path, &["print", "file1"]);
    insta::assert_snapshot!(stdout, @"c
");
    // The unselected change from the source has not been applied
    let stdout = test_env.jj_cmd_success(&repo_path, &["print", "file2"]);
    insta::assert_snapshot!(stdout, @"a
");
    // File `file3`, which was changed in source's parent, is unchanged
    let stdout = test_env.jj_cmd_success(&repo_path, &["print", "file3"]);
    insta::assert_snapshot!(stdout, @"d
");

    // Can move only part of the change from a descendant in non-interactive mode
    test_env.jj_cmd_success(&repo_path, &["undo"]);
    // Clear the script so we know it won't be used
    std::fs::write(&edit_script, "").unwrap();
    let stdout =
        test_env.jj_cmd_success(&repo_path, &["move", "--from", "c", "--to", "b", "file1"]);
    insta::assert_snapshot!(stdout, @"Rebased 1 descendant commits
");
    insta::assert_snapshot!(get_log_output(&test_env, &repo_path), @r###"
    o 21253406d416 c
    o e1cf08aae711 b
    | @ bdd835cae844 d
    |/  
    o 3db0a2f5b535 a
    o 000000000000 
    "###);
    // The selected change from the source has been applied
    let stdout = test_env.jj_cmd_success(&repo_path, &["print", "file1", "-r", "b"]);
    insta::assert_snapshot!(stdout, @"c
");
    // The unselected change from the source has not been applied
    let stdout = test_env.jj_cmd_success(&repo_path, &["print", "file2", "-r", "b"]);
    insta::assert_snapshot!(stdout, @"a
");
}

fn get_log_output(test_env: &TestEnvironment, cwd: &Path) -> String {
    test_env.jj_cmd_success(cwd, &["log", "-T", r#"commit_id.short() " " branches"#])
}
