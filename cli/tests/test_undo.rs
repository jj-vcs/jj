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
use std::path::Path;

use crate::common::git;
use crate::common::TestEnvironment;

#[test]
fn test_undo_rewrite_with_child() {
    // Test that if we undo an operation that rewrote some commit, any descendants
    // after that will be rebased on top of the un-rewritten commit.
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "init", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "initial"]);
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "modified"]);
    let stdout = test_env.jj_cmd_success(&repo_path, &["op", "log"]);
    let op_id_hex = stdout[3..15].to_string();
    test_env.jj_cmd_ok(&repo_path, &["new", "-m", "child"]);
    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "description"]);
    insta::assert_snapshot!(stdout, @r###"
    @  child
    ○  modified
    ◆
    "###);
    test_env.jj_cmd_ok(&repo_path, &["undo", &op_id_hex]);

    // Since we undid the description-change, the child commit should now be on top
    // of the initial commit
    let stdout = test_env.jj_cmd_success(&repo_path, &["log", "-T", "description"]);
    insta::assert_snapshot!(stdout, @r###"
    @  child
    ○  initial
    ◆
    "###);
}

#[test]
fn test_git_push_undo() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path);
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "clone", "git-repo", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    test_env.jj_cmd_ok(&repo_path, &["bookmark", "create", "main"]);
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "AA"]);
    test_env.jj_cmd_ok(&repo_path, &["git", "push", "--allow-new"]);
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "BB"]);
    //   Refs at this point look as follows (-- means no ref)
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local `main`     | BB      |   --   | --
    //    remote-tracking  | AA      |   AA   | AA
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    main: qpvuntsm 75e78001 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 2080bdb8 (empty) AA
    "###);
    let pre_push_opid = test_env.current_operation_id(&repo_path);
    test_env.jj_cmd_ok(&repo_path, &["git", "push"]);
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local  `main`    | BB      |   --   | --
    //    remote-tracking  | BB      |   BB   | BB
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    main: qpvuntsm 75e78001 (empty) BB
      @origin: qpvuntsm 75e78001 (empty) BB
    "###);

    // Undo the push
    test_env.jj_cmd_ok(&repo_path, &["op", "restore", &pre_push_opid]);
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local  `main`    | BB      |   --   | --
    //    remote-tracking  | AA      |   AA   | BB
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    main: qpvuntsm 75e78001 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 2080bdb8 (empty) AA
    "###);
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "CC"]);
    test_env.jj_cmd_ok(&repo_path, &["git", "fetch"]);
    // TODO: The user would probably not expect a conflict here. It currently is
    // because the undo made us forget that the remote was at v2, so the fetch
    // made us think it updated from v1 to v2 (instead of the no-op it could
    // have been).
    //
    // One option to solve this would be to have undo not restore remote-tracking
    // bookmarks, but that also has undersired consequences: the second fetch in
    // `jj git fetch && jj undo && jj git fetch` would become a no-op.
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    main (conflicted):
      - qpvuntsm hidden 2080bdb8 (empty) AA
      + qpvuntsm?? 20b2cc4b (empty) CC
      + qpvuntsm?? 75e78001 (empty) BB
      @origin (behind by 1 commits): qpvuntsm?? 75e78001 (empty) BB
    "###);
}

/// This test is identical to the previous one, except for one additional
/// import. It demonstrates that this changes the outcome.
#[test]
fn test_git_push_undo_with_import() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path);
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "clone", "git-repo", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    test_env.jj_cmd_ok(&repo_path, &["bookmark", "create", "main"]);
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "AA"]);
    test_env.jj_cmd_ok(&repo_path, &["git", "push", "--allow-new"]);
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "BB"]);
    //   Refs at this point look as follows (-- means no ref)
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local `main`     | BB      |   --   | --
    //    remote-tracking  | AA      |   AA   | AA
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    main: qpvuntsm 75e78001 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 2080bdb8 (empty) AA
    "###);
    let pre_push_opid = test_env.current_operation_id(&repo_path);
    test_env.jj_cmd_ok(&repo_path, &["git", "push"]);
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local  `main`    | BB      |   --   | --
    //    remote-tracking  | BB      |   BB   | BB
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    main: qpvuntsm 75e78001 (empty) BB
      @origin: qpvuntsm 75e78001 (empty) BB
    "###);

    // Undo the push
    test_env.jj_cmd_ok(&repo_path, &["op", "restore", &pre_push_opid]);
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local  `main`    | BB      |   --   | --
    //    remote-tracking  | AA      |   AA   | BB
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    main: qpvuntsm 75e78001 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 2080bdb8 (empty) AA
    "###);

    // PROBLEM: inserting this import changes the outcome compared to previous test
    // TODO: decide if this is the better behavior, and whether import of
    // remote-tracking bookmarks should happen on every operation.
    test_env.jj_cmd_ok(&repo_path, &["git", "import"]);
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local  `main`    | BB      |   --   | --
    //    remote-tracking  | BB      |   BB   | BB
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    main: qpvuntsm 75e78001 (empty) BB
      @origin: qpvuntsm 75e78001 (empty) BB
    "###);
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "CC"]);
    test_env.jj_cmd_ok(&repo_path, &["git", "fetch"]);
    // There is not a conflict. This seems like a good outcome; undoing `git push`
    // was essentially a no-op.
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    main: qpvuntsm 20b2cc4b (empty) CC
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 75e78001 (empty) BB
    "###);
}

// This test is currently *identical* to `test_git_push_undo` except the repo
// it's operating it is colocated.
#[test]
fn test_git_push_undo_colocated() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path.clone());
    let repo_path = test_env.env_root().join("clone");
    git::clone(&repo_path, git_repo_path.to_str().unwrap());

    test_env.jj_cmd_ok(&repo_path, &["git", "init", "--git-repo=."]);

    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    test_env.jj_cmd_ok(&repo_path, &["bookmark", "create", "main"]);
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "AA"]);
    test_env.jj_cmd_ok(&repo_path, &["git", "push", "--allow-new"]);
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "BB"]);
    //   Refs at this point look as follows (-- means no ref)
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local `main`     | BB      |   BB   | BB
    //    remote-tracking  | AA      |   AA   | AA
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    main: qpvuntsm 75e78001 (empty) BB
      @git: qpvuntsm 75e78001 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 2080bdb8 (empty) AA
    "###);
    let pre_push_opid = test_env.current_operation_id(&repo_path);
    test_env.jj_cmd_ok(&repo_path, &["git", "push"]);
    //                     | jj refs | jj's   | git
    //                     |         | git    | repo
    //                     |         |tracking|
    //   ------------------------------------------
    //    local `main`     | BB      |   BB   | BB
    //    remote-tracking  | BB      |   BB   | BB
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    main: qpvuntsm 75e78001 (empty) BB
      @git: qpvuntsm 75e78001 (empty) BB
      @origin: qpvuntsm 75e78001 (empty) BB
    "###);

    // Undo the push
    test_env.jj_cmd_ok(&repo_path, &["op", "restore", &pre_push_opid]);
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
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    main: qpvuntsm 75e78001 (empty) BB
      @git: qpvuntsm 75e78001 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 2080bdb8 (empty) AA
    "###);
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "CC"]);
    test_env.jj_cmd_ok(&repo_path, &["git", "fetch"]);
    // We have the same conflict as `test_git_push_undo`. TODO: why did we get the
    // same result in a seemingly different way?
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    main (conflicted):
      - qpvuntsm hidden 2080bdb8 (empty) AA
      + qpvuntsm?? 20b2cc4b (empty) CC
      + qpvuntsm?? 75e78001 (empty) BB
      @git (behind by 1 commits): qpvuntsm?? 20b2cc4b (empty) CC
      @origin (behind by 1 commits): qpvuntsm?? 75e78001 (empty) BB
    "###);
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
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "clone", "git-repo", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    test_env.jj_cmd_ok(&repo_path, &["bookmark", "create", "main"]);
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "AA"]);
    test_env.jj_cmd_ok(&repo_path, &["git", "push", "--allow-new"]);
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    main: qpvuntsm 2080bdb8 (empty) AA
      @origin: qpvuntsm 2080bdb8 (empty) AA
    "###);
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "BB"]);
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    main: qpvuntsm 75e78001 (empty) BB
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 2080bdb8 (empty) AA
    "###);
    let pre_push_opid = test_env.current_operation_id(&repo_path);
    test_env.jj_cmd_ok(&repo_path, &["git", "push"]);

    // Undo the push, but keep both the git_refs and the remote-tracking bookmarks
    test_env.jj_cmd_ok(
        &repo_path,
        &["op", "restore", "--what=repo", &pre_push_opid],
    );
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    main: qpvuntsm 75e78001 (empty) BB
      @origin: qpvuntsm 75e78001 (empty) BB
    "###);
    test_env.advance_test_rng_seed_to_multiple_of(100_000);
    test_env.jj_cmd_ok(&repo_path, &["describe", "-m", "CC"]);
    test_env.jj_cmd_ok(&repo_path, &["git", "fetch"]);
    // This currently gives an identical result to `test_git_push_undo_import`.
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    main: qpvuntsm 20b2cc4b (empty) CC
      @origin (ahead by 1 commits, behind by 1 commits): qpvuntsm hidden 75e78001 (empty) BB
    "###);
}

#[test]
fn test_bookmark_track_untrack_undo() {
    let test_env = TestEnvironment::default();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    let git_repo_path = test_env.env_root().join("git-repo");
    git::init_bare(git_repo_path);
    test_env.jj_cmd_ok(test_env.env_root(), &["git", "clone", "git-repo", "repo"]);
    let repo_path = test_env.env_root().join("repo");

    test_env.jj_cmd_ok(&repo_path, &["describe", "-mcommit"]);
    test_env.jj_cmd_ok(&repo_path, &["bookmark", "create", "feature1", "feature2"]);
    test_env.jj_cmd_ok(&repo_path, &["git", "push", "--allow-new"]);
    test_env.jj_cmd_ok(&repo_path, &["bookmark", "delete", "feature2"]);
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    feature1: qpvuntsm 8da1cfc8 (empty) commit
      @origin: qpvuntsm 8da1cfc8 (empty) commit
    feature2 (deleted)
      @origin: qpvuntsm 8da1cfc8 (empty) commit
    "###);

    // Track/untrack can be undone so long as states can be trivially merged.
    test_env.jj_cmd_ok(
        &repo_path,
        &["bookmark", "untrack", "feature1@origin", "feature2@origin"],
    );
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    feature1: qpvuntsm 8da1cfc8 (empty) commit
    feature1@origin: qpvuntsm 8da1cfc8 (empty) commit
    feature2@origin: qpvuntsm 8da1cfc8 (empty) commit
    "###);

    test_env.jj_cmd_ok(&repo_path, &["undo"]);
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    feature1: qpvuntsm 8da1cfc8 (empty) commit
      @origin: qpvuntsm 8da1cfc8 (empty) commit
    feature2 (deleted)
      @origin: qpvuntsm 8da1cfc8 (empty) commit
    "###);

    test_env.jj_cmd_ok(&repo_path, &["undo"]);
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    feature1: qpvuntsm 8da1cfc8 (empty) commit
    feature1@origin: qpvuntsm 8da1cfc8 (empty) commit
    feature2@origin: qpvuntsm 8da1cfc8 (empty) commit
    "###);

    test_env.jj_cmd_ok(&repo_path, &["bookmark", "track", "feature1@origin"]);
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    feature1: qpvuntsm 8da1cfc8 (empty) commit
      @origin: qpvuntsm 8da1cfc8 (empty) commit
    feature2@origin: qpvuntsm 8da1cfc8 (empty) commit
    "###);

    test_env.jj_cmd_ok(&repo_path, &["undo"]);
    insta::assert_snapshot!(get_bookmark_output(&test_env, &repo_path), @r###"
    feature1: qpvuntsm 8da1cfc8 (empty) commit
    feature1@origin: qpvuntsm 8da1cfc8 (empty) commit
    feature2@origin: qpvuntsm 8da1cfc8 (empty) commit
    "###);
}

fn get_bookmark_output(test_env: &TestEnvironment, repo_path: &Path) -> String {
    // --quiet to suppress deleted bookmarks hint
    test_env.jj_cmd_success(repo_path, &["bookmark", "list", "--all-remotes", "--quiet"])
}
