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

use indoc::indoc;

use crate::common::TestEnvironment;

#[test]
fn test_file_list() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.create_dir("dir");
    work_dir.write_file("dir/file", "content1");
    work_dir.write_file("exec-file", "content1");
    work_dir.write_file("conflict-exec-file", "content1");
    work_dir.write_file("conflict-file", "content1");
    work_dir
        .run_jj(["file", "chmod", "x", "exec-file", "conflict-exec-file"])
        .success();

    work_dir.run_jj(["new", "root()"]).success();
    work_dir.write_file("conflict-exec-file", "content2");
    work_dir.write_file("conflict-file", "content2");
    work_dir
        .run_jj(["file", "chmod", "x", "conflict-exec-file"])
        .success();

    work_dir.run_jj(["new", "visible_heads()"]).success();

    // Lists all files in the current revision by default
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    conflict-exec-file
    conflict-file
    dir/file
    exec-file
    [EOF]
    ");

    // Can list with templates
    let template = indoc! {r#"
        separate(" ",
          path,
          "[" ++ file_type ++ "]",
          "conflict=" ++ conflict,
          "executable=" ++ executable,
        ) ++ "\n"
    "#};
    let output = work_dir.run_jj(["file", "list", "-T", template]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    conflict-exec-file [conflict] conflict=true executable=true
    conflict-file [conflict] conflict=true executable=false
    dir/file [file] conflict=false executable=false
    exec-file [file] conflict=false executable=true
    [EOF]
    ");

    // Can list files in another revision
    let output = work_dir.run_jj(["file", "list", "-r=first_parent(@)"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    conflict-exec-file
    conflict-file
    [EOF]
    ");

    // Can filter by path
    let output = work_dir.run_jj(["file", "list", "dir"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    dir/file
    [EOF]
    ");

    // Warning if path doesn't exist
    let output = work_dir.run_jj(["file", "list", "dir", "file3"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    dir/file
    [EOF]
    ------- stderr -------
    Warning: No matching entries for paths: file3
    [EOF]
    ");
}
