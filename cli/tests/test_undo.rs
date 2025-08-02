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

use testutils::git;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[test]
fn test_undo_root_operation() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // TODO: `jj undo 'root()'` is not a valid command, so use the hardcoded root op
    // id here.
    let output = work_dir.run_jj(["undo", "000000000000"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Cannot undo root operation
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_undo_merge_operation() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["new", "--at-op=@-"]).success();
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    Error: Cannot undo a merge operation
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_undo_rewrite_with_child() {
    // Test that if we undo an operation that rewrote some commit, any descendants
    // after that will be rebased on top of the un-rewritten commit.
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-m", "initial"]).success();
    work_dir.run_jj(["describe", "-m", "modified"]).success();
    work_dir.run_jj(["new", "-m", "child"]).success();
    let output = work_dir.run_jj(["log", "-T", "description"]);
    insta::assert_snapshot!(output, @r"
    @  child
    ○  modified
    ◆
    [EOF]
    ");
    work_dir.run_jj(["undo", "@-"]).success();

    // Since we undid the description-change, the child commit should now be on top
    // of the initial commit
    let output = work_dir.run_jj(["log", "-T", "description"]);
    insta::assert_snapshot!(output, @r"
    @  child
    ○  initial
    ◆
    [EOF]
    ");
}

#[test]
fn test_git_push_undo() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path);
    test_env
        .run_jj_in(".", ["git", "clone", "git-repo", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir
        .run_jj(["bookmark", "create", "-r@", "main"])
        .success();
    work_dir.run_jj(["describe", "-m", "AA"]).success();
    work_dir.run_jj(["git", "push", "--allow-new"]).success();
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir.run_jj(["describe", "-m", "BB"]).success();
    //   Refs at this point look as follows (-- means no ref)
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local `main`     | BB      |   --   | --
    //    remote-tracking  | AA      |   AA   | AA
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm d9a9f6a0 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 3a44d6c5 (empty) AA
    [EOF]
    ");
    let pre_push_opid = work_dir.current_operation_id();
    work_dir.run_jj(["git", "push"]).success();
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local  `main`    | BB      |   --   | --
    //    remote-tracking  | BB      |   BB   | BB
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm d9a9f6a0 (empty) BB
      @origin: qpvuntsm d9a9f6a0 (empty) BB
    [EOF]
    ");

    // Undo the push
    work_dir.run_jj(["op", "restore", &pre_push_opid]).success();
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local  `main`    | BB      |   --   | --
    //    remote-tracking  | AA      |   AA   | BB
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm d9a9f6a0 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 3a44d6c5 (empty) AA
    [EOF]
    ");
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir.run_jj(["describe", "-m", "CC"]).success();
    work_dir.run_jj(["git", "fetch"]).success();
    // TODO: The user would probably not expect a conflict here. It currently is
    // because the undo made us forget that the remote was at v2, so the fetch
    // made us think it updated from v1 to v2 (instead of the no-op it could
    // have been).
    //
    // One option to solve this would be to have undo not restore remote-tracking
    // bookmarks, but that also has undersired consequences: the second fetch in
    // `jj git fetch && jj undo && jj git fetch` would become a no-op.
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main (conflicted):
      - qpvuntsm hidden 3a44d6c5 (empty) AA
      + qpvuntsm divergent 1e742089 (empty) CC
      + qpvuntsm divergent d9a9f6a0 (empty) BB
      @origin (behind by 1 commits): qpvuntsm divergent d9a9f6a0 (empty) BB
    [EOF]
    ");
}

/// This test is identical to the previous one, except for one additional
/// import. It demonstrates that this changes the outcome.
#[test]
fn test_git_push_undo_with_import() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path);
    test_env
        .run_jj_in(".", ["git", "clone", "git-repo", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir
        .run_jj(["bookmark", "create", "-r@", "main"])
        .success();
    work_dir.run_jj(["describe", "-m", "AA"]).success();
    work_dir.run_jj(["git", "push", "--allow-new"]).success();
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir.run_jj(["describe", "-m", "BB"]).success();
    //   Refs at this point look as follows (-- means no ref)
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local `main`     | BB      |   --   | --
    //    remote-tracking  | AA      |   AA   | AA
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm d9a9f6a0 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 3a44d6c5 (empty) AA
    [EOF]
    ");
    let pre_push_opid = work_dir.current_operation_id();
    work_dir.run_jj(["git", "push"]).success();
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local  `main`    | BB      |   --   | --
    //    remote-tracking  | BB      |   BB   | BB
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm d9a9f6a0 (empty) BB
      @origin: qpvuntsm d9a9f6a0 (empty) BB
    [EOF]
    ");

    // Undo the push
    work_dir.run_jj(["op", "restore", &pre_push_opid]).success();
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local  `main`    | BB      |   --   | --
    //    remote-tracking  | AA      |   AA   | BB
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm d9a9f6a0 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 3a44d6c5 (empty) AA
    [EOF]
    ");

    // PROBLEM: inserting this import changes the outcome compared to previous test
    // TODO: decide if this is the better behavior, and whether import of
    // remote-tracking bookmarks should happen on every operation.
    work_dir.run_jj(["git", "import"]).success();
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local  `main`    | BB      |   --   | --
    //    remote-tracking  | BB      |   BB   | BB
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm d9a9f6a0 (empty) BB
      @origin: qpvuntsm d9a9f6a0 (empty) BB
    [EOF]
    ");
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir.run_jj(["describe", "-m", "CC"]).success();
    work_dir.run_jj(["git", "fetch"]).success();
    // There is not a conflict. This seems like a good outcome; undoing `git push`
    // was essentially a no-op.
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 1e742089 (empty) CC
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden d9a9f6a0 (empty) BB
    [EOF]
    ");
}

// This test is currently *identical* to `test_git_push_undo` except the repo
// it's operating it is colocated.
#[test]
fn test_git_push_undo_colocated() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path.clone());
    let work_dir = test_env.work_dir("clone");
    git::clone(work_dir.root(), git_repo_path.to_str().unwrap(), None);

    work_dir.run_jj(["git", "init", "--git-repo=."]).success();

    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir
        .run_jj(["bookmark", "create", "-r@", "main"])
        .success();
    work_dir.run_jj(["describe", "-m", "AA"]).success();
    work_dir.run_jj(["git", "push", "--allow-new"]).success();
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir.run_jj(["describe", "-m", "BB"]).success();
    //   Refs at this point look as follows (-- means no ref)
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local `main`     | BB      |   BB   | BB
    //    remote-tracking  | AA      |   AA   | AA
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm d9a9f6a0 (empty) BB
      @git: qpvuntsm d9a9f6a0 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 3a44d6c5 (empty) AA
    [EOF]
    ");
    let pre_push_opid = work_dir.current_operation_id();
    work_dir.run_jj(["git", "push"]).success();
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local `main`     | BB      |   BB   | BB
    //    remote-tracking  | BB      |   BB   | BB
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm d9a9f6a0 (empty) BB
      @git: qpvuntsm d9a9f6a0 (empty) BB
      @origin: qpvuntsm d9a9f6a0 (empty) BB
    [EOF]
    ");

    // Undo the push
    work_dir.run_jj(["op", "restore", &pre_push_opid]).success();
    //       === Before auto-export ====
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local `main`     | BB      |   BB   | BB
    //    remote-tracking  | AA      |   BB   | BB
    //       === After automatic `jj git export` ====
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local `main`     | BB      |   BB   | BB
    //    remote-tracking  | AA      |   AA   | AA
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm d9a9f6a0 (empty) BB
      @git: qpvuntsm d9a9f6a0 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 3a44d6c5 (empty) AA
    [EOF]
    ");
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir.run_jj(["describe", "-m", "CC"]).success();
    work_dir.run_jj(["git", "fetch"]).success();
    // We have the same conflict as `test_git_push_undo`. TODO: why did we get the
    // same result in a seemingly different way?
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main (conflicted):
      - qpvuntsm hidden 3a44d6c5 (empty) AA
      + qpvuntsm divergent 1e742089 (empty) CC
      + qpvuntsm divergent d9a9f6a0 (empty) BB
      @git (behind by 1 commits): qpvuntsm divergent 1e742089 (empty) CC
      @origin (behind by 1 commits): qpvuntsm divergent d9a9f6a0 (empty) BB
    [EOF]
    ");
}

// This test is currently *identical* to `test_git_push_undo` except
// both the git_refs and the remote-tracking bookmarks are preserved by undo.
// TODO: Investigate the different outcome
#[test]
fn test_git_push_undo_repo_only() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path);
    test_env
        .run_jj_in(".", ["git", "clone", "git-repo", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir
        .run_jj(["bookmark", "create", "-r@", "main"])
        .success();
    work_dir.run_jj(["describe", "-m", "AA"]).success();
    work_dir.run_jj(["git", "push", "--allow-new"]).success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 3a44d6c5 (empty) AA
      @origin: qpvuntsm 3a44d6c5 (empty) AA
    [EOF]
    ");
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir.run_jj(["describe", "-m", "BB"]).success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm d9a9f6a0 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 3a44d6c5 (empty) AA
    [EOF]
    ");
    let pre_push_opid = work_dir.current_operation_id();
    work_dir.run_jj(["git", "push"]).success();

    // Undo the push, but keep both the git_refs and the remote-tracking bookmarks
    work_dir
        .run_jj(["op", "restore", "--what=repo", &pre_push_opid])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm d9a9f6a0 (empty) BB
      @origin: qpvuntsm d9a9f6a0 (empty) BB
    [EOF]
    ");
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir.run_jj(["describe", "-m", "CC"]).success();
    work_dir.run_jj(["git", "fetch"]).success();
    // This currently gives an identical result to `test_git_push_undo_import`.
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 1e742089 (empty) CC
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden d9a9f6a0 (empty) BB
    [EOF]
    ");
}

#[test]
fn test_bookmark_track_untrack_undo() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path);
    test_env
        .run_jj_in(".", ["git", "clone", "git-repo", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["describe", "-mcommit"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "feature1", "feature2"])
        .success();
    work_dir.run_jj(["git", "push", "--allow-new"]).success();
    work_dir
        .run_jj(["bookmark", "delete", "feature2"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qpvuntsm bab5b5ef (empty) commit
      @origin: qpvuntsm bab5b5ef (empty) commit
    feature2 (deleted)
      @origin: qpvuntsm bab5b5ef (empty) commit
    [EOF]
    ");

    // Track/untrack can be undone so long as states can be trivially merged.
    work_dir
        .run_jj(["bookmark", "untrack", "feature1@origin", "feature2@origin"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qpvuntsm bab5b5ef (empty) commit
    feature1@origin: qpvuntsm bab5b5ef (empty) commit
    feature2@origin: qpvuntsm bab5b5ef (empty) commit
    [EOF]
    ");

    work_dir.run_jj(["undo"]).success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qpvuntsm bab5b5ef (empty) commit
      @origin: qpvuntsm bab5b5ef (empty) commit
    feature2 (deleted)
      @origin: qpvuntsm bab5b5ef (empty) commit
    [EOF]
    ");

    work_dir.run_jj(["undo"]).success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qpvuntsm bab5b5ef (empty) commit
    feature1@origin: qpvuntsm bab5b5ef (empty) commit
    feature2@origin: qpvuntsm bab5b5ef (empty) commit
    [EOF]
    ");

    work_dir
        .run_jj(["bookmark", "track", "feature1@origin"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qpvuntsm bab5b5ef (empty) commit
      @origin: qpvuntsm bab5b5ef (empty) commit
    feature2@origin: qpvuntsm bab5b5ef (empty) commit
    [EOF]
    ");

    work_dir.run_jj(["undo"]).success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qpvuntsm bab5b5ef (empty) commit
    feature1@origin: qpvuntsm bab5b5ef (empty) commit
    feature2@origin: qpvuntsm bab5b5ef (empty) commit
    [EOF]
    ");
}

#[test]
fn test_undo_latest_undo_implicitly() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Double-undo creation of child
    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: 6de77c1b46a3 (2001-02-03 08:05:09) undo operation dbcb2561b6fee72ea6de79511b6b62f1fff2424f79d16dd30339f94621100f77c86ca7450f7b1ec1bd95d4d56b7a54fe3f3e612353e62cedc682366211b4144e
    Working copy  (@) now at: rlvkpnrz 43444d88 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    Warning: The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in the same state as it was before the second-last `jj undo`.
    Hint: To undo multiple operations, use `jj op log` to see past states and `jj op restore` to restore one of these states.
    [EOF]
    ");

    // Double-undo creation of sibling
    work_dir.run_jj(["new", "@-"]).success();
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: b77c991c5a2f (2001-02-03 08:05:12) undo operation 58f57841c00da755413d291ed9e1a1d9a58dd4311b5000a8703f1bf93339dd12cdbc2c6e1c8cd5f43cb584cabddcf8366153a007c244d89ee80a4e42e513058d
    Working copy  (@) now at: mzvwutvl 8afc18ff (empty) (no description set)
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    Warning: The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in the same state as it was before the second-last `jj undo`.
    Hint: To undo multiple operations, use `jj op log` to see past states and `jj op restore` to restore one of these states.
    [EOF]
    ");
}

#[test]
fn test_undo_latest_undo_explicitly() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    let output = work_dir
        .run_jj(["op", "log", "--no-graph", "-T=id.short()", "-n=1"])
        .success();
    let op_id_hex = output.stdout.raw();
    let output = work_dir.run_jj(["undo", op_id_hex]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: 6de77c1b46a3 (2001-02-03 08:05:09) undo operation dbcb2561b6fee72ea6de79511b6b62f1fff2424f79d16dd30339f94621100f77c86ca7450f7b1ec1bd95d4d56b7a54fe3f3e612353e62cedc682366211b4144e
    Working copy  (@) now at: rlvkpnrz 43444d88 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["undo", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: c99ea8473832 (2001-02-03 08:05:13) undo operation 09cbd5c994ee9e437950a49fa4400d3c6e9d5c1e44ea55b38806a9692317716647578328c53b887a758a40270ad8ab6d5b67c1675d088281879cc7c74a2da6cc
    Working copy  (@) now at: royxmykx ba0e5dca (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 43444d88 (empty) (no description set)
    Warning: The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in the same state as it was before the second-last `jj undo`.
    Hint: To undo multiple operations, use `jj op log` to see past states and `jj op restore` to restore one of these states.
    [EOF]
    ");
}

#[test]
fn test_undo_an_older_undo() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    let output = work_dir
        .run_jj(["op", "log", "--no-graph", "-T=id.short()", "-n=1"])
        .success();
    let op_id_hex = output.stdout.raw();
    work_dir.run_jj(["new"]).success();
    // Undo an older undo operation that is not the immediately preceding operation.
    let output = work_dir.run_jj(["undo", op_id_hex]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: 6de77c1b46a3 (2001-02-03 08:05:09) undo operation dbcb2561b6fee72ea6de79511b6b62f1fff2424f79d16dd30339f94621100f77c86ca7450f7b1ec1bd95d4d56b7a54fe3f3e612353e62cedc682366211b4144e
    [EOF]
    ");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    work_dir.run_jj(["new"]).success();
    // Undo an older undo operation that is not the immediately preceding operation.
    let output = work_dir.run_jj(["undo", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: ffea82d2bf89 (2001-02-03 08:05:14) undo operation 2b3480d4ece49d59f5da51e1284b742773e6b858935f122bebab625430137f9aae1aaebf7a8c081e0715e9cd24deee9d0d039743fbb1dd61804c633c9c7f17a2
    [EOF]
    ");
}

#[test]
fn test_undo_an_undo_multiple_times() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: 6de77c1b46a3 (2001-02-03 08:05:09) undo operation dbcb2561b6fee72ea6de79511b6b62f1fff2424f79d16dd30339f94621100f77c86ca7450f7b1ec1bd95d4d56b7a54fe3f3e612353e62cedc682366211b4144e
    Working copy  (@) now at: rlvkpnrz 43444d88 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    Warning: The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in the same state as it was before the second-last `jj undo`.
    Hint: To undo multiple operations, use `jj op log` to see past states and `jj op restore` to restore one of these states.
    [EOF]
    ");
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: f17b43a603fd (2001-02-03 08:05:10) undo operation 6de77c1b46a39115938845d2fffb5780a49a4991d1f8f99b60509705faa38d496b06cdbb427a497ff22d1f2b81613ac80a8dd6b97ca2fa35d7cc66d2a33059e4
    Working copy  (@) now at: qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Warning: The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in the same state as it was before the second-last `jj undo`.
    Hint: To undo multiple operations, use `jj op log` to see past states and `jj op restore` to restore one of these states.
    [EOF]
    ");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: 1eee1feca2d3 (2001-02-03 08:05:13) undo operation 18ee4719e7c7a2da3fbeec66678dfb03c4f3deebbe887dbc46270102c86894615a07a223713981d12b3ddc757e5104c7e9f7b391202747cd1868e0d2ac01f35d
    Working copy  (@) now at: royxmykx e7d0d5fd (empty) (no description set)
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    Warning: The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in the same state as it was before the second-last `jj undo`.
    Hint: To undo multiple operations, use `jj op log` to see past states and `jj op restore` to restore one of these states.
    [EOF]
    ");
    let output = work_dir.run_jj(["undo", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: 079cb7121929 (2001-02-03 08:05:14) undo operation 1eee1feca2d334e1c7c956048eec1e54a35d58b39aaec88c8a48ce7ce216b30e287f49c0305c51ad86292c6a2e11dd98ed97d8ef0a497e7114cf2aea99d4e71d
    Working copy  (@) now at: qpvuntsm e8849ae1 (empty) (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Warning: The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in the same state as it was before the second-last `jj undo`.
    Hint: To undo multiple operations, use `jj op log` to see past states and `jj op restore` to restore one of these states.
    [EOF]
    ");
}

#[test]
fn test_undo_bookmark_deletion() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["bookmark", "create", "foo", "-r=@"])
        .success();
    work_dir.run_jj(["bookmark", "delete", "foo"]).success();
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: e1bcf7cd8080 (2001-02-03 08:05:09) delete bookmark foo
    [EOF]
    ");
}

#[must_use]
fn get_bookmark_output(work_dir: &TestWorkDir) -> CommandOutput {
    // --quiet to suppress deleted bookmarks hint
    work_dir.run_jj(["bookmark", "list", "--all-remotes", "--quiet"])
}
