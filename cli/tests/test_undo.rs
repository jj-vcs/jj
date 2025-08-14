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
      + qpvuntsm?? 1e742089 (empty) CC
      + qpvuntsm?? d9a9f6a0 (empty) BB
      @origin (behind by 1 commits): qpvuntsm?? d9a9f6a0 (empty) BB
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
      + qpvuntsm?? 1e742089 (empty) CC
      + qpvuntsm?? d9a9f6a0 (empty) BB
      @git (behind by 1 commits): qpvuntsm?? 1e742089 (empty) CC
      @origin (behind by 1 commits): qpvuntsm?? d9a9f6a0 (empty) BB
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
    Undid operation: 5b31042c020b (2001-02-03 08:05:09) revert operation dbcb2561b6fee72ea6de79511b6b62f1fff2424f79d16dd30339f94621100f77c86ca7450f7b1ec1bd95d4d56b7a54fe3f3e612353e62cedc682366211b4144e
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
    Undid operation: e83fef84da56 (2001-02-03 08:05:12) revert operation c9a93954e94b008f7295813623c4c12aeffd2cb81728e8d7813a37c8b6252b8f9a15ddfc6e496b393de355bd07405e740a05178cf8845af0087fb02223e4c404
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
    Undid operation: 5b31042c020b (2001-02-03 08:05:09) revert operation dbcb2561b6fee72ea6de79511b6b62f1fff2424f79d16dd30339f94621100f77c86ca7450f7b1ec1bd95d4d56b7a54fe3f3e612353e62cedc682366211b4144e
    Working copy  (@) now at: rlvkpnrz 43444d88 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["undo", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: f344a2234512 (2001-02-03 08:05:13) revert operation a6365efd7a8958525e22cd7b4fb01f308260464facdc3f03c82a151892a073e0fbf6b4d9ad49991ab7ee4f05c55c147c74841714710c87ce7a990e112fe782b8
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
    Undid operation: 5b31042c020b (2001-02-03 08:05:09) revert operation dbcb2561b6fee72ea6de79511b6b62f1fff2424f79d16dd30339f94621100f77c86ca7450f7b1ec1bd95d4d56b7a54fe3f3e612353e62cedc682366211b4144e
    [EOF]
    ");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    work_dir.run_jj(["new"]).success();
    // Undo an older undo operation that is not the immediately preceding operation.
    let output = work_dir.run_jj(["undo", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: f2c360817b17 (2001-02-03 08:05:14) revert operation 20e354c3f097e96da28c0470b2d9e38c07370ebbae6c01b33c62a44bee913603086b66c97cb8a24a6b1df284b64a82edea14b7fa3cb124da55ebc4d743a92475
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
    Undid operation: 5b31042c020b (2001-02-03 08:05:09) revert operation dbcb2561b6fee72ea6de79511b6b62f1fff2424f79d16dd30339f94621100f77c86ca7450f7b1ec1bd95d4d56b7a54fe3f3e612353e62cedc682366211b4144e
    Working copy  (@) now at: rlvkpnrz 43444d88 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    Warning: The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in the same state as it was before the second-last `jj undo`.
    Hint: To undo multiple operations, use `jj op log` to see past states and `jj op restore` to restore one of these states.
    [EOF]
    ");
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: 91cc66ce7fb2 (2001-02-03 08:05:10) revert operation 5b31042c020bd6090d52b932c998a263655cd541b7922c2f56e372a0ee367aa4f7dfb0ccb89c55d2fa232ba8ff6fb22276607ccd12f03844841e6a3888f5972d
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
    Undid operation: e26664a8b05f (2001-02-03 08:05:13) revert operation 3ac3dec981c7b97070849ee22a468ec4d16b2dbb1f3df26092ea2c11289b61bc3a38a1b5e3f5a91f576feb8769e171c1905f70f39e0284648c68c7a21f439817
    Working copy  (@) now at: royxmykx e7d0d5fd (empty) (no description set)
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    Warning: The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in the same state as it was before the second-last `jj undo`.
    Hint: To undo multiple operations, use `jj op log` to see past states and `jj op restore` to restore one of these states.
    [EOF]
    ");
    let output = work_dir.run_jj(["undo", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: cd5fdc0df497 (2001-02-03 08:05:14) revert operation e26664a8b05f2380b6857ea564f389363bc150ddbdc6cf087908787fc3831b3aa95575baed369581f8709c5cfc63f4787ff7b601e51639e8badc8251b8b2b9f9
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
