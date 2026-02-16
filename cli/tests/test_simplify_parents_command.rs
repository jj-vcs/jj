// Copyright 2024 The Jujutsu Authors
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

use test_case::test_case;

use crate::common::TestEnvironment;
use crate::common::create_commit;

#[test]
fn test_simplify_parents_no_commits() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["simplify-parents", "-r", "root() ~ root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");
}

#[test]
fn test_simplify_parents_immutable() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["simplify-parents", "-r", "root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: The root commit 000000000000 is immutable
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_simplify_parents_no_change() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &["root()"]);
    create_commit(&work_dir, "b", &["a"]);
    let output = work_dir.run_jj(["log", "-r", "all()", "-T", "description"]);
    insta::assert_snapshot!(output, @"
    @  b
    ○  a
    ◆
    [EOF]
    ");

    let output = work_dir.run_jj(["simplify-parents", "-s", "@-"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-r", "all()", "-T", "description"]);
    insta::assert_snapshot!(output, @"
    @  b
    ○  a
    ◆
    [EOF]
    ");
}

#[test]
fn test_simplify_parents_no_change_diamond() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &["root()"]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["a"]);
    create_commit(&work_dir, "d", &["b", "c"]);
    let output = work_dir.run_jj(["log", "-r", "all()", "-T", "description"]);
    insta::assert_snapshot!(output, @"
    @    d
    ├─╮
    │ ○  c
    ○ │  b
    ├─╯
    ○  a
    ◆
    [EOF]
    ");

    let output = work_dir.run_jj(["simplify-parents", "-r", "all() ~ root()"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Nothing changed.
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-r", "all()", "-T", "description"]);
    insta::assert_snapshot!(output, @"
    @    d
    ├─╮
    │ ○  c
    ○ │  b
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
}

#[test_case(&["simplify-parents", "-r", "@", "-r", "@-"] ; "revisions")]
#[test_case(&["simplify-parents", "-s", "@-"] ; "sources")]
fn test_simplify_parents_redundant_parent(args: &[&str]) {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &["root()"]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["a", "b"]);
    let output = work_dir.run_jj(["log", "-r", "all()", "-T", "description"]);
    insta::allow_duplicates! {
        insta::assert_snapshot!(output, @"
        @    c
        ├─╮
        │ ○  b
        ├─╯
        ○  a
        ◆
        [EOF]
        ");
    }
    let output = work_dir.run_jj(args);
    insta::allow_duplicates! {
        insta::assert_snapshot!(output, @"
        ------- stderr -------
        Removed 1 edges from 1 out of 3 commits.
        Working copy  (@) now at: royxmykx 265f0407 c | c
        Parent commit (@-)      : zsuskuln 123b4d91 b | b
        [EOF]
        ");
    }

    let output = work_dir.run_jj(["log", "-r", "all()", "-T", "description"]);
    insta::allow_duplicates! {
        insta::assert_snapshot!(output, @"
        @  c
        ○  b
        ○  a
        ◆
        [EOF]
        ");
    }
}

#[test]
fn test_simplify_parents_multiple_redundant_parents() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &["root()"]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["a", "b"]);
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["d"]);
    create_commit(&work_dir, "f", &["d", "e"]);
    let output = work_dir.run_jj(["log", "-r", "all()", "-T", "description"]);
    insta::assert_snapshot!(output, @"
    @    f
    ├─╮
    │ ○  e
    ├─╯
    ○  d
    ○    c
    ├─╮
    │ ○  b
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    // Test with `-r`.
    let output = work_dir.run_jj(["simplify-parents", "-r", "c", "-r", "f"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Removed 2 edges from 2 out of 2 commits.
    Rebased 2 descendant commits
    Working copy  (@) now at: kmkuslsw 5ad764e9 f | f
    Parent commit (@-)      : znkkpsqq 9102487c e | e
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-r", "all()", "-T", "description"]);
    insta::assert_snapshot!(output, @"
    @  f
    ○  e
    ○  d
    ○  c
    ○  b
    ○  a
    ◆
    [EOF]
    ");

    // Test with `-s`.
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    let output = work_dir.run_jj(["simplify-parents", "-s", "c"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Removed 2 edges from 2 out of 4 commits.
    Rebased 2 descendant commits
    Working copy  (@) now at: kmkuslsw 2b2c1c63 f | f
    Parent commit (@-)      : znkkpsqq 9142e3bb e | e
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-r", "all()", "-T", "description"]);
    insta::assert_snapshot!(output, @"
    @  f
    ○  e
    ○  d
    ○  c
    ○  b
    ○  a
    ◆
    [EOF]
    ");
}

#[test]
fn test_simplify_parents_no_args() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &["root()"]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["a", "b"]);
    create_commit(&work_dir, "d", &["c"]);
    create_commit(&work_dir, "e", &["d"]);
    create_commit(&work_dir, "f", &["d", "e"]);
    let output = work_dir.run_jj(["log", "-r", "all()", "-T", "description"]);
    insta::assert_snapshot!(output, @"
    @    f
    ├─╮
    │ ○  e
    ├─╯
    ○  d
    ○    c
    ├─╮
    │ ○  b
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
    let setup_opid = work_dir.current_operation_id();

    let output = work_dir.run_jj(["simplify-parents"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Removed 2 edges from 2 out of 6 commits.
    Rebased 2 descendant commits
    Working copy  (@) now at: kmkuslsw 5ad764e9 f | f
    Parent commit (@-)      : znkkpsqq 9102487c e | e
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-r", "all()", "-T", "description"]);
    insta::assert_snapshot!(output, @"
    @  f
    ○  e
    ○  d
    ○  c
    ○  b
    ○  a
    ◆
    [EOF]
    ");

    // Test with custom `revsets.simplify-parents`.
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    test_env.add_config(r#"revsets.simplify-parents = "d::""#);
    let output = work_dir.run_jj(["simplify-parents"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Removed 1 edges from 1 out of 3 commits.
    Working copy  (@) now at: kmkuslsw 1180d0f5 f | f
    Parent commit (@-)      : znkkpsqq 009aef72 e | e
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "-r", "all()", "-T", "description"]);
    insta::assert_snapshot!(output, @"
    @  f
    ○  e
    ○  d
    ○    c
    ├─╮
    │ ○  b
    ├─╯
    ○  a
    ◆
    [EOF]
    ");
}
