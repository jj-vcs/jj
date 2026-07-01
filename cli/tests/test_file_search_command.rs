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

    // Searches all files in the current revision by default and prints each
    // matched line prefixed by the file path
    let output = work_dir.run_jj(["file", "search", "--pattern=glob:*foo*"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    dir/file3:-foobar-
    file1:-foo-
    [EOF]
    ");

    // --name-only restores path-only output
    let output = work_dir.run_jj(["file", "search", "--name-only", "--pattern=glob:*foo*"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    dir/file3
    file1
    [EOF]
    ");

    // Matches only the whole line for glob pattern
    let output = work_dir.run_jj(["file", "search", "--pattern=glob:foo"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"");

    // Can search files in another revision
    let output = work_dir.run_jj(["file", "search", "--pattern=glob:*foo*", "-r=@-"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    file1:-foo-
    [EOF]
    ");

    // Can filter by path
    let output = work_dir.run_jj(["file", "search", "--pattern=glob:*foo*", "dir"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    dir/file3:-foobar-
    [EOF]
    ");

    // Warning if path doesn't exist
    let output = work_dir.run_jj(["file", "search", "--pattern=glob:*foo*", "file9"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    ------- stderr -------
    Warning: No matching entries for paths: file9
    [EOF]
    ");

    // The default is regex
    let output = work_dir.run_jj(["file", "search", "--pattern=f.o"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    dir/file3:-foobar-
    file1:-foo-
    [EOF]
    ");

    // Can specify the kind
    let output = work_dir.run_jj(["file", "search", "--pattern=glob-i:*foo*"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    dir/file3:-foobar-
    file1:-foo-
    [EOF]
    ");

    // The part before the first colon is treated as the kind.
    let output = work_dir.run_jj(["file", "search", "--pattern=foo:bar"]);
    insta::assert_snapshot!(output.normalize_stderr_exit_status(), @"
    ------- stderr -------
    Error: Invalid string pattern kind `foo:`
    [EOF]
    [exit status: 2]
    ");

    // Colons can be in the pattern part.
    let output = work_dir.run_jj(["file", "search", "--pattern=regex-i:foo:bar"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"");

    // Prints every matched line, not just the first
    work_dir.write_file("multi", "hit-one\nmiss\nhit-two\nhit-three\n");
    let output = work_dir.run_jj(["file", "search", "--pattern=hit", "multi"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    multi:hit-one
    multi:hit-two
    multi:hit-three
    [EOF]
    ");
    // --name-only collapses the same file to a single line
    let output = work_dir.run_jj(["file", "search", "--name-only", "--pattern=hit", "multi"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    multi
    [EOF]
    ");

    // -n prefixes each match with its 1-based line number
    let output = work_dir.run_jj(["file", "search", "-n", "--pattern=hit", "multi"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    multi:1:hit-one
    multi:3:hit-two
    multi:4:hit-three
    [EOF]
    ");
}

#[test]
fn test_file_search_conflicts() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "-foo-\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "-bar-\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("file1", "-baz-\n");
    work_dir.run_jj(["rebase", "-r=@", "-B=@-"]).success();

    // Test the setup
    insta::assert_snapshot!(work_dir.read_file("file1"), @r"
    <<<<<<< conflict 1 of 1
    %%%%%%% diff from: rlvkpnrz 60901f47 (parents of rebased revision)
    \\\\\\\        to: qpvuntsm fae24a95 (rebase destination)
    --bar-
    +-foo-
    +++++++ kkmpptxz 51957a05 (rebased revision)
    -baz-
    >>>>>>> conflict 1 of 1 ends
    ");

    // Matches positive terms (one match per matching add side)
    let output = work_dir.run_jj(["file", "search", "--pattern=glob:*foo*"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    file1:-foo-
    [EOF]
    ");
    // --name-only collapses per-file even with a conflict
    let output = work_dir.run_jj(["file", "search", "--name-only", "--pattern=glob:*foo*"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    file1
    [EOF]
    ");
    let output = work_dir.run_jj(["file", "search", "--pattern=glob:*bar*"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"");
    let output = work_dir.run_jj(["file", "search", "--pattern=glob:*baz*"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    file1:-baz-
    [EOF]
    ");

    // A pattern that matches on multiple add sides: each matching side emits
    // one line; with --name-only the path is deduped to a single output.
    let output = work_dir.run_jj(["file", "search", "--pattern=regex:-(foo|baz)-"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    file1:-foo-
    file1:-baz-
    [EOF]
    ");
    let output = work_dir.run_jj([
        "file",
        "search",
        "--name-only",
        "--pattern=regex:-(foo|baz)-",
    ]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    file1
    [EOF]
    ");

    // -n numbers lines within each conflict side independently, so matches on
    // different sides can share a line number.
    let output = work_dir.run_jj(["file", "search", "-n", "--pattern=regex:-(foo|baz)-"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
    file1:1:-foo-
    file1:1:-baz-
    [EOF]
    ");

    // Doesn't match the conflict markers
    let output = work_dir.run_jj(["file", "search", "--pattern=glob:*%%%*"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"");

    // Doesn't list file if the pattern doesn't match
    let output = work_dir.run_jj(["file", "search", "--pattern=glob:*qux*"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"");
}
