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

use insta::assert_snapshot;
use regex::Regex;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;

#[test]
fn test_debug_fileset() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["debug", "fileset", "all()"]);
    assert_snapshot!(output, @r"
    -- Parsed:
    All

    -- Matcher:
    EverythingMatcher
    [EOF]
    ");

    let output = work_dir.run_jj(["debug", "fileset", "cwd:.."]);
    assert_snapshot!(output.normalize_backslash(), @r#"
    ------- stderr -------
    Error: Failed to parse fileset: Invalid file pattern
    Caused by:
    1:  --> 1:1
      |
    1 | cwd:..
      | ^----^
      |
      = Invalid file pattern
    2: Path ".." is not in the repo "."
    3: Invalid component ".." in repo-relative path "../"
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_debug_revset() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let mut insta_settings = insta::Settings::clone_current();
    insta_settings.add_filter(r"(?m)(^    .*\n)+", "    ..\n");
    let _guard = insta_settings.bind_to_scope();

    let output = work_dir.run_jj(["debug", "revset", "root()"]);
    assert_snapshot!(output, @r"
    -- Parsed:
    Root

    -- Resolved:
    Root

    -- Optimized:
    Root

    -- Backend:
    Commits(
        ..
    )

    -- Evaluated:
    RevsetImpl {
        ..
    }

    -- Commit IDs:
    0000000000000000000000000000000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["debug", "revset", "--no-optimize", "root() & ~@"]);
    assert_snapshot!(output, @r"
    -- Parsed:
    Intersection(
        ..
    )

    -- Resolved:
    Intersection(
        ..
    )

    -- Backend:
    Intersection(
        ..
    )

    -- Evaluated:
    RevsetImpl {
        ..
    }

    -- Commit IDs:
    0000000000000000000000000000000000000000
    [EOF]
    ");

    let output = work_dir.run_jj(["debug", "revset", "--no-resolve", "foo & ~bar"]);
    assert_snapshot!(output, @r"
    -- Parsed:
    Intersection(
        ..
    )

    -- Optimized:
    Difference(
        ..
    )

    [EOF]
    ");

    let output = work_dir.run_jj([
        "debug",
        "revset",
        "--no-resolve",
        "--no-optimize",
        "foo & ~bar",
    ]);
    assert_snapshot!(output, @r"
    -- Parsed:
    Intersection(
        ..
    )

    [EOF]
    ");
}

#[test]
fn test_debug_index() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let output = work_dir.run_jj(["debug", "index"]);
    assert_snapshot!(filter_index_stats(output), @r"
    === Commits ===
    Number of commits: 2
    Number of merges: 0
    Max generation number: 1
    Number of heads: 1
    Number of changes: 2
    Stats per level:
      Level 0:
        Number of commits: 2
        Name: [hash]
    === Changed paths ===
    Indexed commits: none
    Stats per level:
    [EOF]
    ");

    // Enable changed-path index, index one commit
    let output = work_dir.run_jj(["debug", "index-changed-paths", "-n1"]);
    assert_snapshot!(output, @r"
    ------- stderr -------
    Finished indexing 1..2 commits.
    [EOF]
    ");
    let output = work_dir.run_jj(["debug", "index"]);
    assert_snapshot!(filter_index_stats(output), @r"
    === Commits ===
    Number of commits: 2
    Number of merges: 0
    Max generation number: 1
    Number of heads: 1
    Number of changes: 2
    Stats per level:
      Level 0:
        Number of commits: 2
        Name: [hash]
    === Changed paths ===
    Indexed commits: 1..2
    Stats per level:
      Level 0:
        Number of commits: 1
        Number of changed paths: 0
        Number of paths: 0
        Name: [hash]
    [EOF]
    ");
}

#[test]
fn test_debug_reindex() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["new"]).success();
    let output = work_dir.run_jj(["debug", "index"]);
    assert_snapshot!(filter_index_stats(output), @r"
    === Commits ===
    Number of commits: 4
    Number of merges: 0
    Max generation number: 3
    Number of heads: 1
    Number of changes: 4
    Stats per level:
      Level 0:
        Number of commits: 3
        Name: [hash]
      Level 1:
        Number of commits: 1
        Name: [hash]
    === Changed paths ===
    Indexed commits: none
    Stats per level:
    [EOF]
    ");
    let output = work_dir.run_jj(["debug", "reindex"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Finished indexing 4 commits.
    [EOF]
    ");
    let output = work_dir.run_jj(["debug", "index"]);
    assert_snapshot!(filter_index_stats(output), @r"
    === Commits ===
    Number of commits: 4
    Number of merges: 0
    Max generation number: 3
    Number of heads: 1
    Number of changes: 4
    Stats per level:
      Level 0:
        Number of commits: 4
        Name: [hash]
    === Changed paths ===
    Indexed commits: none
    Stats per level:
    [EOF]
    ");
}

#[test]
fn test_debug_stacked_table() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["new"]).success();

    let output = work_dir.run_jj([
        "debug",
        "stacked-table",
        ".jj/repo/store/extra",
        "--key-size=20", // HASH_LENGTH
    ]);
    assert_snapshot!(filter_index_stats(output), @r"
    Number of entries: 4
    Stats per level:
      Level 0:
        Number of entries: 3
        Name: [hash]
      Level 1:
        Number of entries: 1
        Name: [hash]
    [EOF]
    ");
}

#[test]
fn test_debug_tree() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let sub_dir = work_dir.create_dir_all("dir/subdir");
    sub_dir.write_file("file1", "contents 1");
    work_dir.run_jj(["new"]).success();
    sub_dir.write_file("file2", "contents 2");

    // Defaults to showing the tree at the current commit
    let output = work_dir.run_jj(["debug", "tree"]);
    assert_snapshot!(output.normalize_backslash(), @r#"
    dir/subdir/file1: Ok(Resolved(Some(File { id: FileId("498e9b01d79cb8d31cdf0df1a663cc1fcefd9de3"), executable: false, copy_id: CopyId("") })))
    dir/subdir/file2: Ok(Resolved(Some(File { id: FileId("b2496eaffe394cd50a9db4de5787f45f09fd9722"), executable: false, copy_id: CopyId("") })))
    [EOF]
    "#
    );

    // Can show the tree at another commit
    let output = work_dir.run_jj(["debug", "tree", "-r@-"]);
    assert_snapshot!(output.normalize_backslash(), @r#"
    dir/subdir/file1: Ok(Resolved(Some(File { id: FileId("498e9b01d79cb8d31cdf0df1a663cc1fcefd9de3"), executable: false, copy_id: CopyId("") })))
    [EOF]
    "#
    );

    // Can filter by paths
    let output = work_dir.run_jj(["debug", "tree", "dir/subdir/file2"]);
    assert_snapshot!(output.normalize_backslash(), @r#"
    dir/subdir/file2: Ok(Resolved(Some(File { id: FileId("b2496eaffe394cd50a9db4de5787f45f09fd9722"), executable: false, copy_id: CopyId("") })))
    [EOF]
    "#
    );

    // Can a show the root tree by id
    let output = work_dir.run_jj([
        "debug",
        "tree",
        "--id=0958358e3f80e794f032b25ed2be96cf5825da6c",
    ]);
    assert_snapshot!(output.normalize_backslash(), @r#"
    dir/subdir/file1: Ok(Resolved(Some(File { id: FileId("498e9b01d79cb8d31cdf0df1a663cc1fcefd9de3"), executable: false, copy_id: CopyId("") })))
    dir/subdir/file2: Ok(Resolved(Some(File { id: FileId("b2496eaffe394cd50a9db4de5787f45f09fd9722"), executable: false, copy_id: CopyId("") })))
    [EOF]
    "#
    );

    // Can a show non-root tree by id
    let output = work_dir.run_jj([
        "debug",
        "tree",
        "--dir=dir",
        "--id=6ac232efa713535ae518a1a898b77e76c0478184",
    ]);
    assert_snapshot!(output.normalize_backslash(), @r#"
    dir/subdir/file1: Ok(Resolved(Some(File { id: FileId("498e9b01d79cb8d31cdf0df1a663cc1fcefd9de3"), executable: false, copy_id: CopyId("") })))
    dir/subdir/file2: Ok(Resolved(Some(File { id: FileId("b2496eaffe394cd50a9db4de5787f45f09fd9722"), executable: false, copy_id: CopyId("") })))
    [EOF]
    "#
    );

    // Can filter by paths when showing non-root tree (matcher applies from root)
    let output = work_dir.run_jj([
        "debug",
        "tree",
        "--dir=dir",
        "--id=6ac232efa713535ae518a1a898b77e76c0478184",
        "dir/subdir/file2",
    ]);
    assert_snapshot!(output.normalize_backslash(), @r#"
    dir/subdir/file2: Ok(Resolved(Some(File { id: FileId("b2496eaffe394cd50a9db4de5787f45f09fd9722"), executable: false, copy_id: CopyId("") })))
    [EOF]
    "#
    );
}

fn filter_index_stats(output: CommandOutput) -> CommandOutput {
    let regex = Regex::new(r"    Name: [0-9a-z]+").unwrap();
    output.normalize_stdout_with(|text| regex.replace_all(&text, "    Name: [hash]").into_owned())
}
