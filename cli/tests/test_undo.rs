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
    Undid operation: 94337eb25498 (2001-02-03 08:05:09) undo operation c7b028ea7b47461d4328dde306e13337ea9000b6abfde4cb751902fae3124d578f2e0082cbd1e1d32b5e64afc001a933be5acc81a015a7f15d62d90750eaa9ca
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
    Undid operation: 0456966dabcb (2001-02-03 08:05:12) undo operation aea8c90f370bd851b9b66a6542aeff24f01418d745efcd7a94c7399afe4ce9aaf79903cd1feb34a9d091bfad3b3680e138ec90ecbed759cf2f6eacdb5c34e4e6
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
    Undid operation: 94337eb25498 (2001-02-03 08:05:09) undo operation c7b028ea7b47461d4328dde306e13337ea9000b6abfde4cb751902fae3124d578f2e0082cbd1e1d32b5e64afc001a933be5acc81a015a7f15d62d90750eaa9ca
    Working copy  (@) now at: rlvkpnrz 43444d88 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    [EOF]
    ");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    let output = work_dir.run_jj(["undo", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: 38e14f528fb2 (2001-02-03 08:05:13) undo operation a479f2846c170617a366097141d14c2ce78a7d2ae730f137a28bda493edbd17f40b334fb9344fd54915dcc8f30f1a3e7feeb0ea17ecc3d1d599f1fb9b54f7cfe
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
    Undid operation: 94337eb25498 (2001-02-03 08:05:09) undo operation c7b028ea7b47461d4328dde306e13337ea9000b6abfde4cb751902fae3124d578f2e0082cbd1e1d32b5e64afc001a933be5acc81a015a7f15d62d90750eaa9ca
    [EOF]
    ");

    work_dir.run_jj(["new"]).success();
    work_dir.run_jj(["undo"]).success();
    work_dir.run_jj(["new"]).success();
    // Undo an older undo operation that is not the immediately preceding operation.
    let output = work_dir.run_jj(["undo", "@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: 4653f935d392 (2001-02-03 08:05:14) undo operation de46f96595ec455388b0e35d59df3e1ffaaeba420c8333e8ee88f78fc5b799a42313e734a4ba6f3c0efb2dd866ba0e9a2c814e7f85b718340dfb6087dff5e8f3
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
    Undid operation: 94337eb25498 (2001-02-03 08:05:09) undo operation c7b028ea7b47461d4328dde306e13337ea9000b6abfde4cb751902fae3124d578f2e0082cbd1e1d32b5e64afc001a933be5acc81a015a7f15d62d90750eaa9ca
    Working copy  (@) now at: rlvkpnrz 43444d88 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    Warning: The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in the same state as it was before the second-last `jj undo`.
    Hint: To undo multiple operations, use `jj op log` to see past states and `jj op restore` to restore one of these states.
    [EOF]
    ");
    let output = work_dir.run_jj(["undo"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: a43b6dc67d14 (2001-02-03 08:05:10) undo operation 94337eb25498c16c342e4c270978d856b34ab6a669ffa4631a9ca6d940377e0a310e67a1f1911407b28500bdfd0f7d866f761eff434594f9d9dc7d63d8b51610
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
    Undid operation: 1d6091bcbe71 (2001-02-03 08:05:13) undo operation 407478e37f74d99a9284168a0c8c8aa3c9afcbc741a8d03948212864d64fe30f8c75d2f570bc82b2772b9a47b72b651058acbdb2384ce5de31acb46a35e9d77b
    Working copy  (@) now at: royxmykx e7d0d5fd (empty) (no description set)
    Parent commit (@-)      : qpvuntsm e8849ae1 (empty) (no description set)
    Warning: The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in the same state as it was before the second-last `jj undo`.
    Hint: To undo multiple operations, use `jj op log` to see past states and `jj op restore` to restore one of these states.
    [EOF]
    ");
    let output = work_dir.run_jj(["undo", "@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: b16c5dfbbafc (2001-02-03 08:05:14) undo operation 1d6091bcbe71afd3fd33a4e397597b6867649cdd7d22013675c8895bd3406257de86a9dfc4a9dc118db70f76f631ea5e0adf9bb2eb0f32f9e67a4fb227bdd3e1
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
    // FIXME: The warning and hint should not be shown here, since we did not undo
    // an actual undo operation.
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Undid operation: 1075351137a2 (2001-02-03 08:05:09) delete bookmark foo
    Warning: The second-last `jj undo` was reverted by the latest `jj undo`. The repo is now in the same state as it was before the second-last `jj undo`.
    Hint: To undo multiple operations, use `jj op log` to see past states and `jj op restore` to restore one of these states.
    [EOF]
    ");
}

#[must_use]
fn get_bookmark_output(work_dir: &TestWorkDir) -> CommandOutput {
    // --quiet to suppress deleted bookmarks hint
    work_dir.run_jj(["bookmark", "list", "--all-remotes", "--quiet"])
}
