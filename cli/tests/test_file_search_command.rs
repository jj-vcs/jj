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
fn test_file_search() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "-foo-");
    work_dir.write_file("file2", "-bar-");
    work_dir.run_jj(["new"]).success();
    work_dir.create_dir("dir");
    work_dir.write_file("dir/file3", "-foobar-");

    // Searches all files in the current revision by default
    let output = work_dir.run_jj(["file", "search", "--pattern=*foo*"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    dir/file3
    file1
    [EOF]
    ");

    // Matches only the whole line
    let output = work_dir.run_jj(["file", "search", "--pattern=foo"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"");

    // Can search files in another revision
    let output = work_dir.run_jj(["file", "search", "--pattern=*foo*", "-r=@-"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    file1
    [EOF]
    ");

    // Can filter by path
    let output = work_dir.run_jj(["file", "search", "--pattern=*foo*", "dir"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    dir/file3
    [EOF]
    ");

    // Warning if path doesn't exist
    let output = work_dir.run_jj(["file", "search", "--pattern=*foo*", "file9"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    ------- stderr -------
    Warning: No matching entries for paths: file9
    [EOF]
    ");
}

#[test]
fn test_file_search_conflicts() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "-foo-");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "-bar-");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "-baz-");
    work_dir.run_jj(["rebase", "-r=@", "-B=@-"]).success();

    // Test the setup
    insta::assert_snapshot!(work_dir.read_file("file1"), @r"
    <<<<<<< conflict 1 of 1
    %%%%%%% diff from base to side #1 (no terminating newline)
    --bar-
    +-foo-
    +++++++ side #2 (no terminating newline)
    -baz-
    >>>>>>> conflict 1 of 1 ends
    ");

    // Matches positive terms
    let output = work_dir.run_jj(["file", "search", "--pattern=*foo*"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    file1
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "search", "--pattern=*bar*"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"");
    let output = work_dir.run_jj(["file", "search", "--pattern=*baz*"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    file1
    [EOF]
    ");

    // Doesn't match the conflict markers
    let output = work_dir.run_jj(["file", "search", "--pattern=*%%%*"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"");

    // Doesn't list file if the pattern doesn't match
    let output = work_dir.run_jj(["file", "search", "--pattern=*qux*"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"");
}
