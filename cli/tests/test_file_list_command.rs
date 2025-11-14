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

use crate::common::TestEnvironment;

#[test]
fn test_file_list() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "");
    work_dir.run_jj(["new"]).success();
    work_dir.create_dir("dir");
    work_dir.write_file("dir/file2", "");

    // Lists all files in the current revision by default
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    dir/file2
    file1
    [EOF]
    ");

    // Can list files in another revision
    let output = work_dir.run_jj(["file", "list", "-r=@-"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    file1
    [EOF]
    ");

    // Can filter by path
    let output = work_dir.run_jj(["file", "list", "dir"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    dir/file2
    [EOF]
    ");

    // Warning if path doesn't exist
    let output = work_dir.run_jj(["file", "list", "dir", "file3"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    dir/file2
    [EOF]
    ------- stderr -------
    Warning: No matching entries for paths: file3
    [EOF]
    ");
}
