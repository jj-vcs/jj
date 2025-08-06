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

use testutils::git;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[test]
fn test_revert_root_operation() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // TODO: `jj op revert 'root()'` is not a valid command, so use the
    // hardcoded root op id here.
    let output = work_dir.run_jj(["op", "revert", "000000000000"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Cannot revert root operation
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_revert_merge_operation() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["new", "--at-op=@-"]).success();
    let output = work_dir.run_jj(["op", "revert"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Concurrent modification detected, resolving automatically.
    Error: Cannot revert a merge operation
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_revert_rewrite_with_child() {
    // Test that if we revert an operation that rewrote some commit, any descendants
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
    work_dir.run_jj(["op", "revert", "@-"]).success();

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
fn test_git_push_revert() {
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
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm/1 hidden 3a44d6c5 (empty) AA
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

    // Revert the push
    work_dir.run_jj(["op", "restore", &pre_push_opid]).success();
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local  `main`    | BB      |   --   | --
    //    remote-tracking  | AA      |   AA   | BB
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm d9a9f6a0 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm/1 hidden 3a44d6c5 (empty) AA
    [EOF]
    ");
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir.run_jj(["describe", "-m", "CC"]).success();
    work_dir.run_jj(["git", "fetch"]).success();
    // TODO: The user would probably not expect a conflict here. It currently is
    // because the revert made us forget that the remote was at v2, so the fetch
    // made us think it updated from v1 to v2 (instead of the no-op it could
    // have been).
    //
    // One option to solve this would be to have `op revert` not restore
    // remote-tracking bookmarks, but that also has undersired consequences: the
    // second fetch in `jj git fetch && jj op revert && jj git fetch` would
    // become a no-op.
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main (conflicted):
      - qpvuntsm/2 hidden 3a44d6c5 (empty) AA
      + qpvuntsm/0 1e742089 (empty) CC
      + qpvuntsm/1 d9a9f6a0 (empty) BB
      @origin (behind by 1 commits): qpvuntsm/1 d9a9f6a0 (empty) BB
    [EOF]
    ");
}

/// This test is identical to the previous one, except for one additional
/// import. It demonstrates that this changes the outcome.
#[test]
fn test_git_push_revert_with_import() {
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
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm/1 hidden 3a44d6c5 (empty) AA
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

    // Revert the push
    work_dir.run_jj(["op", "restore", &pre_push_opid]).success();
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local  `main`    | BB      |   --   | --
    //    remote-tracking  | AA      |   AA   | BB
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm d9a9f6a0 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm/1 hidden 3a44d6c5 (empty) AA
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
    // There is not a conflict. This seems like a good outcome; reverting `git push`
    // was essentially a no-op.
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 1e742089 (empty) CC
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm/1 hidden d9a9f6a0 (empty) BB
    [EOF]
    ");
}

// This test is currently *identical* to `test_git_push_revert` except the repo
// it's operating on is colocated.
#[test]
fn test_git_push_revert_colocated() {
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
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm/1 hidden 3a44d6c5 (empty) AA
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

    // Revert the push
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
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm/1 hidden 3a44d6c5 (empty) AA
    [EOF]
    ");
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    work_dir.run_jj(["describe", "-m", "CC"]).success();
    work_dir.run_jj(["git", "fetch"]).success();
    // We have the same conflict as `test_git_push_revert`. TODO: why did we get the
    // same result in a seemingly different way?
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main (conflicted):
      - qpvuntsm/2 hidden 3a44d6c5 (empty) AA
      + qpvuntsm/0 1e742089 (empty) CC
      + qpvuntsm/1 d9a9f6a0 (empty) BB
      @git (behind by 1 commits): qpvuntsm/0 1e742089 (empty) CC
      @origin (behind by 1 commits): qpvuntsm/1 d9a9f6a0 (empty) BB
    [EOF]
    ");
}

// This test is currently *identical* to `test_git_push_revert` except
// both the git_refs and the remote-tracking bookmarks are preserved by revert.
// TODO: Investigate the different outcome
#[test]
fn test_git_push_revert_repo_only() {
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
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm/1 hidden 3a44d6c5 (empty) AA
    [EOF]
    ");
    let pre_push_opid = work_dir.current_operation_id();
    work_dir.run_jj(["git", "push"]).success();

    // Revert the push, but keep both the git_refs and the remote-tracking bookmarks
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
    // This currently gives an identical result to `test_git_push_revert_import`.
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    main: qpvuntsm 1e742089 (empty) CC
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm/1 hidden d9a9f6a0 (empty) BB
    [EOF]
    ");
}

#[test]
fn test_bookmark_track_untrack_revert() {
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

    // Track/untrack can be reverted so long as states can be trivially merged.
    work_dir
        .run_jj(["bookmark", "untrack", "feature1 | feature2"])
        .success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qpvuntsm bab5b5ef (empty) commit
    feature1@origin: qpvuntsm bab5b5ef (empty) commit
    feature2@origin: qpvuntsm bab5b5ef (empty) commit
    [EOF]
    ");

    work_dir.run_jj(["op", "revert"]).success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qpvuntsm bab5b5ef (empty) commit
      @origin: qpvuntsm bab5b5ef (empty) commit
    feature2 (deleted)
      @origin: qpvuntsm bab5b5ef (empty) commit
    [EOF]
    ");

    work_dir.run_jj(["op", "revert"]).success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qpvuntsm bab5b5ef (empty) commit
    feature1@origin: qpvuntsm bab5b5ef (empty) commit
    feature2@origin: qpvuntsm bab5b5ef (empty) commit
    [EOF]
    ");

    work_dir.run_jj(["bookmark", "track", "feature1"]).success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qpvuntsm bab5b5ef (empty) commit
      @origin: qpvuntsm bab5b5ef (empty) commit
    feature2@origin: qpvuntsm bab5b5ef (empty) commit
    [EOF]
    ");

    work_dir.run_jj(["op", "revert"]).success();
    insta::assert_snapshot!(get_bookmark_output(&work_dir), @r"
    feature1: qpvuntsm bab5b5ef (empty) commit
    feature1@origin: qpvuntsm bab5b5ef (empty) commit
    feature2@origin: qpvuntsm bab5b5ef (empty) commit
    [EOF]
    ");
}

#[must_use]
fn get_bookmark_output(work_dir: &TestWorkDir) -> CommandOutput {
    // --quiet to suppress deleted bookmarks hint
    work_dir.run_jj(["bookmark", "list", "--all-remotes", "--quiet"])
}
