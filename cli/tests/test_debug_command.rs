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
    let workspace_path = test_env.env_root().join("repo");

    let output = test_env.run_jj_in(&workspace_path, ["debug", "fileset", "all()"]);
    assert_snapshot!(output, @r"
    -- Parsed:
    All

    -- Matcher:
    EverythingMatcher
    [EOF]
    ");

    let output = test_env.run_jj_in(&workspace_path, ["debug", "fileset", "cwd:.."]);
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
    let workspace_path = test_env.env_root().join("repo");

    let output = test_env.run_jj_in(&workspace_path, ["debug", "revset", "root()"]);
    insta::with_settings!({filters => vec![
        (r"(?m)(^    .*\n)+", "    ..\n"),
    ]}, {
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
    });
}

#[test]
fn test_debug_index() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let workspace_path = test_env.env_root().join("repo");
    let output = test_env.run_jj_in(&workspace_path, ["debug", "index"]);
    assert_snapshot!(filter_index_stats(output), @r"
    Number of commits: 2
    Number of merges: 0
    Max generation number: 1
    Number of heads: 1
    Number of changes: 2
    Stats per level:
      Level 0:
        Number of commits: 2
        Name: [hash]
    [EOF]
    ");
}

#[test]
fn test_debug_reindex() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let workspace_path = test_env.env_root().join("repo");
    test_env.run_jj_in(&workspace_path, ["new"]).success();
    test_env.run_jj_in(&workspace_path, ["new"]).success();
    let output = test_env.run_jj_in(&workspace_path, ["debug", "index"]);
    assert_snapshot!(filter_index_stats(output), @r"
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
    [EOF]
    ");
    let output = test_env.run_jj_in(&workspace_path, ["debug", "reindex"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Finished indexing 4 commits.
    [EOF]
    ");
    let output = test_env.run_jj_in(&workspace_path, ["debug", "index"]);
    assert_snapshot!(filter_index_stats(output), @r"
    Number of commits: 4
    Number of merges: 0
    Max generation number: 3
    Number of heads: 1
    Number of changes: 4
    Stats per level:
      Level 0:
        Number of commits: 4
        Name: [hash]
    [EOF]
    ");
}

#[test]
fn test_debug_tree() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let workspace_path = test_env.env_root().join("repo");
    let subdir = workspace_path.join("dir").join("subdir");
    std::fs::create_dir_all(&subdir).unwrap();
    std::fs::write(subdir.join("file1"), "contents 1").unwrap();
    test_env.run_jj_in(&workspace_path, ["new"]).success();
    std::fs::write(subdir.join("file2"), "contents 2").unwrap();

    // Defaults to showing the tree at the current commit
    let output = test_env.run_jj_in(&workspace_path, ["debug", "tree"]);
    assert_snapshot!(output.normalize_backslash(), @r#"
    dir/subdir/file1: Ok(Resolved(Some(File { id: FileId("498e9b01d79cb8d31cdf0df1a663cc1fcefd9de3"), executable: false })))
    dir/subdir/file2: Ok(Resolved(Some(File { id: FileId("b2496eaffe394cd50a9db4de5787f45f09fd9722"), executable: false })))
    [EOF]
    "#
    );

    // Can show the tree at another commit
    let output = test_env.run_jj_in(&workspace_path, ["debug", "tree", "-r@-"]);
    assert_snapshot!(output.normalize_backslash(), @r#"
    dir/subdir/file1: Ok(Resolved(Some(File { id: FileId("498e9b01d79cb8d31cdf0df1a663cc1fcefd9de3"), executable: false })))
    [EOF]
    "#
    );

    // Can filter by paths
    let output = test_env.run_jj_in(&workspace_path, ["debug", "tree", "dir/subdir/file2"]);
    assert_snapshot!(output.normalize_backslash(), @r#"
    dir/subdir/file2: Ok(Resolved(Some(File { id: FileId("b2496eaffe394cd50a9db4de5787f45f09fd9722"), executable: false })))
    [EOF]
    "#
    );

    // Can a show the root tree by id
    let output = test_env.run_jj_in(
        &workspace_path,
        [
            "debug",
            "tree",
            "--id=0958358e3f80e794f032b25ed2be96cf5825da6c",
        ],
    );
    assert_snapshot!(output.normalize_backslash(), @r#"
    dir/subdir/file1: Ok(Resolved(Some(File { id: FileId("498e9b01d79cb8d31cdf0df1a663cc1fcefd9de3"), executable: false })))
    dir/subdir/file2: Ok(Resolved(Some(File { id: FileId("b2496eaffe394cd50a9db4de5787f45f09fd9722"), executable: false })))
    [EOF]
    "#
    );

    // Can a show non-root tree by id
    let output = test_env.run_jj_in(
        &workspace_path,
        [
            "debug",
            "tree",
            "--dir=dir",
            "--id=6ac232efa713535ae518a1a898b77e76c0478184",
        ],
    );
    assert_snapshot!(output.normalize_backslash(), @r#"
    dir/subdir/file1: Ok(Resolved(Some(File { id: FileId("498e9b01d79cb8d31cdf0df1a663cc1fcefd9de3"), executable: false })))
    dir/subdir/file2: Ok(Resolved(Some(File { id: FileId("b2496eaffe394cd50a9db4de5787f45f09fd9722"), executable: false })))
    [EOF]
    "#
    );

    // Can filter by paths when showing non-root tree (matcher applies from root)
    let output = test_env.run_jj_in(
        &workspace_path,
        [
            "debug",
            "tree",
            "--dir=dir",
            "--id=6ac232efa713535ae518a1a898b77e76c0478184",
            "dir/subdir/file2",
        ],
    );
    assert_snapshot!(output.normalize_backslash(), @r#"
    dir/subdir/file2: Ok(Resolved(Some(File { id: FileId("b2496eaffe394cd50a9db4de5787f45f09fd9722"), executable: false })))
    [EOF]
    "#
    );
}

#[test]
fn test_debug_operation_id() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let workspace_path = test_env.env_root().join("repo");
    let output = test_env.run_jj_in(&workspace_path, ["debug", "operation", "--display", "id"]);
    assert_snapshot!(filter_index_stats(output), @r"
    eac759b9ab75793fd3da96e60939fb48f2cd2b2a9c1f13ffe723cf620f3005b8d3e7e923634a07ea39513e4f2f360c87b9ad5d331cf90d7a844864b83b72eba1
    [EOF]
    ");
}

fn filter_index_stats(output: CommandOutput) -> CommandOutput {
    let regex = Regex::new(r"    Name: [0-9a-z]+").unwrap();
    output.normalize_stdout_with(|text| regex.replace_all(&text, "    Name: [hash]").into_owned())
}
