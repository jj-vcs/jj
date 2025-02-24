// Copyright 2023 The Jujutsu Authors
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
fn test_rewrite_immutable_generic() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    std::fs::write(repo_path.join("file"), "a").unwrap();
    test_env
        .run_jj_in(&repo_path, ["describe", "-m=a"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "-m=b"]).success();
    std::fs::write(repo_path.join("file"), "b").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "main"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["new", "main-", "-m=c"])
        .success();
    std::fs::write(repo_path.join("file"), "c").unwrap();
    let output = test_env.run_jj_in(&repo_path, ["log"]);
    insta::assert_snapshot!(output, @r"
    @  mzvwutvl test.user@example.com 2001-02-03 08:05:12 7adb43e8
    │  c
    │ ○  kkmpptxz test.user@example.com 2001-02-03 08:05:10 main 72e1b68c
    ├─╯  b
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 b84b821b
    │  a
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    // Cannot rewrite a commit in the configured set
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "main""#);
    let output = test_env.run_jj_in(&repo_path, ["edit", "main"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit 72e1b68cbcf2 is immutable
    Hint: Could not modify commit: kkmpptxz 72e1b68c main | b
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // Cannot rewrite an ancestor of the configured set
    let output = test_env.run_jj_in(&repo_path, ["edit", "main-"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit b84b821b8a2b is immutable
    Hint: Could not modify commit: qpvuntsm b84b821b a
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 2 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // Cannot rewrite the root commit even with an empty set of immutable commits
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "none()""#);
    let output = test_env.run_jj_in(&repo_path, ["edit", "root()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The root commit 000000000000 is immutable
    [EOF]
    [exit status: 1]
    ");

    // Error mutating the repo if immutable_heads() uses a ref that can't be
    // resolved
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "bookmark_that_does_not_exist""#);
    // Suppress warning in the commit summary template
    test_env.add_config("template-aliases.'format_short_id(id)' = 'id.short(8)'");
    let output = test_env.run_jj_in(&repo_path, ["new", "main"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Config error: Invalid `revset-aliases.immutable_heads()`
    Caused by: Revision `bookmark_that_does_not_exist` doesn't exist
    For help, see https://jj-vcs.github.io/jj/latest/config/.
    [EOF]
    [exit status: 1]
    ");

    // Can use --ignore-immutable to override
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "main""#);
    let output = test_env.run_jj_in(&repo_path, ["--ignore-immutable", "edit", "main"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: kkmpptxz 72e1b68c main | b
    Parent commit      : qpvuntsm b84b821b a
    Added 0 files, modified 1 files, removed 0 files
    [EOF]
    ");
    // ... but not the root commit
    let output = test_env.run_jj_in(&repo_path, ["--ignore-immutable", "edit", "root()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The root commit 000000000000 is immutable
    [EOF]
    [exit status: 1]
    ");

    // Mutating the repo works if ref is wrapped in present()
    test_env.add_config(
        r#"revset-aliases."immutable_heads()" = "present(bookmark_that_does_not_exist)""#,
    );
    let output = test_env.run_jj_in(&repo_path, ["new", "main"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy now at: wqnwkozp fc921593 (empty) (no description set)
    Parent commit      : kkmpptxz 72e1b68c main | b
    [EOF]
    ");

    // immutable_heads() of different arity doesn't shadow the 0-ary one
    test_env.add_config(r#"revset-aliases."immutable_heads(foo)" = "none()""#);
    let output = test_env.run_jj_in(&repo_path, ["edit", "root()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The root commit 000000000000 is immutable
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_new_wc_commit_when_wc_immutable() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init"])
        .success();
    test_env
        .run_jj_in(test_env.env_root(), ["bookmark", "create", "-r@", "main"])
        .success();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "main""#);
    test_env
        .run_jj_in(test_env.env_root(), ["new", "-m=a"])
        .success();
    let output = test_env.run_jj_in(test_env.env_root(), ["bookmark", "set", "main", "-r@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 1 bookmarks to kkmpptxz a164195b main | (empty) a
    Warning: The working-copy commit in workspace 'default' became immutable, so a new commit has been created on top of it.
    Working copy now at: zsuskuln ef5fa85b (empty) (no description set)
    Parent commit      : kkmpptxz a164195b main | (empty) a
    [EOF]
    ");
}

#[test]
fn test_immutable_heads_set_to_working_copy() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init"])
        .success();
    test_env
        .run_jj_in(test_env.env_root(), ["bookmark", "create", "-r@", "main"])
        .success();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "@""#);
    let output = test_env.run_jj_in(test_env.env_root(), ["new", "-m=a"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: The working-copy commit in workspace 'default' became immutable, so a new commit has been created on top of it.
    Working copy now at: pmmvwywv 7278b2d8 (empty) (no description set)
    Parent commit      : kkmpptxz a713ef56 (empty) a
    [EOF]
    ");
}

#[test]
fn test_new_wc_commit_when_wc_immutable_multi_workspace() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "main"])
        .success();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "main""#);
    test_env.run_jj_in(&repo_path, ["new", "-m=a"]).success();
    test_env
        .run_jj_in(&repo_path, ["workspace", "add", "../workspace1"])
        .success();
    let workspace1_envroot = test_env.env_root().join("workspace1");
    test_env
        .run_jj_in(&workspace1_envroot, ["edit", "default@"])
        .success();
    let output = test_env.run_jj_in(&repo_path, ["bookmark", "set", "main", "-r@"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Moved 1 bookmarks to kkmpptxz 7796c4df main | (empty) a
    Warning: The working-copy commit in workspace 'default' became immutable, so a new commit has been created on top of it.
    Warning: The working-copy commit in workspace 'workspace1' became immutable, so a new commit has been created on top of it.
    Working copy now at: royxmykx 896465c4 (empty) (no description set)
    Parent commit      : kkmpptxz 7796c4df main | (empty) a
    [EOF]
    ");
    test_env
        .run_jj_in(&workspace1_envroot, ["workspace", "update-stale"])
        .success();
    let output = test_env.run_jj_in(&workspace1_envroot, ["log", "--no-graph"]);
    insta::assert_snapshot!(output, @r"
    nppvrztz test.user@example.com 2001-02-03 08:05:11 workspace1@ ee0671fd
    (empty) (no description set)
    royxmykx test.user@example.com 2001-02-03 08:05:12 default@ 896465c4
    (empty) (no description set)
    kkmpptxz test.user@example.com 2001-02-03 08:05:09 main 7796c4df
    (empty) a
    zzzzzzzz root() 00000000
    [EOF]
    ");
}

#[test]
fn test_rewrite_immutable_commands() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    let repo_path = test_env.env_root().join("repo");
    std::fs::write(repo_path.join("file"), "a").unwrap();
    test_env
        .run_jj_in(&repo_path, ["describe", "-m=a"])
        .success();
    test_env.run_jj_in(&repo_path, ["new", "-m=b"]).success();
    std::fs::write(repo_path.join("file"), "b").unwrap();
    test_env
        .run_jj_in(&repo_path, ["new", "@-", "-m=c"])
        .success();
    std::fs::write(repo_path.join("file"), "c").unwrap();
    test_env
        .run_jj_in(&repo_path, ["new", "all:visible_heads()", "-m=merge"])
        .success();
    // Create another file to make sure the merge commit isn't empty (to satisfy `jj
    // split`) and still has a conflict (to satisfy `jj resolve`).
    std::fs::write(repo_path.join("file2"), "merged").unwrap();
    test_env
        .run_jj_in(&repo_path, ["bookmark", "create", "-r@", "main"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["new", "description(b)"])
        .success();
    std::fs::write(repo_path.join("file"), "w").unwrap();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "main""#);
    test_env.add_config(r#"revset-aliases."trunk()" = "main""#);

    // Log shows mutable commits, their parents, and trunk() by default
    let output = test_env.run_jj_in(&repo_path, ["log"]);
    insta::assert_snapshot!(output, @r"
    @  yqosqzyt test.user@example.com 2001-02-03 08:05:14 55641cc5
    │  (no description set)
    │ ◆  mzvwutvl test.user@example.com 2001-02-03 08:05:12 main bcab555f conflict
    ╭─┤  merge
    │ │
    │ ~
    │
    ◆  kkmpptxz test.user@example.com 2001-02-03 08:05:10 72e1b68c
    │  b
    ~
    [EOF]
    ");

    // abandon
    let output = test_env.run_jj_in(&repo_path, ["abandon", "main"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit bcab555fc80e is immutable
    Hint: Could not modify commit: mzvwutvl bcab555f main | (conflict) merge
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // absorb
    let output = test_env.run_jj_in(&repo_path, ["absorb", "--into=::@-"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit 72e1b68cbcf2 is immutable
    Hint: Could not modify commit: kkmpptxz 72e1b68c b
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 2 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // chmod
    let output = test_env.run_jj_in(&repo_path, ["file", "chmod", "-r=main", "x", "file"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit bcab555fc80e is immutable
    Hint: Could not modify commit: mzvwutvl bcab555f main | (conflict) merge
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // describe
    let output = test_env.run_jj_in(&repo_path, ["describe", "main"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit bcab555fc80e is immutable
    Hint: Could not modify commit: mzvwutvl bcab555f main | (conflict) merge
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // diffedit
    let output = test_env.run_jj_in(&repo_path, ["diffedit", "-r=main"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit bcab555fc80e is immutable
    Hint: Could not modify commit: mzvwutvl bcab555f main | (conflict) merge
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // edit
    let output = test_env.run_jj_in(&repo_path, ["edit", "main"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit bcab555fc80e is immutable
    Hint: Could not modify commit: mzvwutvl bcab555f main | (conflict) merge
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // new --insert-before
    let output = test_env.run_jj_in(&repo_path, ["new", "--insert-before", "main"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit bcab555fc80e is immutable
    Hint: Could not modify commit: mzvwutvl bcab555f main | (conflict) merge
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // new --insert-after parent_of_main
    let output = test_env.run_jj_in(&repo_path, ["new", "--insert-after", "description(b)"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit bcab555fc80e is immutable
    Hint: Could not modify commit: mzvwutvl bcab555f main | (conflict) merge
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // parallelize
    let output = test_env.run_jj_in(&repo_path, ["parallelize", "description(b)", "main"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit bcab555fc80e is immutable
    Hint: Could not modify commit: mzvwutvl bcab555f main | (conflict) merge
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 2 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // rebase -s
    let output = test_env.run_jj_in(&repo_path, ["rebase", "-s=main", "-d=@"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit bcab555fc80e is immutable
    Hint: Could not modify commit: mzvwutvl bcab555f main | (conflict) merge
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // rebase -b
    let output = test_env.run_jj_in(&repo_path, ["rebase", "-b=main", "-d=@"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit 77cee210cbf5 is immutable
    Hint: Could not modify commit: zsuskuln 77cee210 c
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 2 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // rebase -r
    let output = test_env.run_jj_in(&repo_path, ["rebase", "-r=main", "-d=@"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit bcab555fc80e is immutable
    Hint: Could not modify commit: mzvwutvl bcab555f main | (conflict) merge
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // resolve
    let output = test_env.run_jj_in(&repo_path, ["resolve", "-r=description(merge)", "file"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit bcab555fc80e is immutable
    Hint: Could not modify commit: mzvwutvl bcab555f main | (conflict) merge
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // restore -c
    let output = test_env.run_jj_in(&repo_path, ["restore", "-c=main"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit bcab555fc80e is immutable
    Hint: Could not modify commit: mzvwutvl bcab555f main | (conflict) merge
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // restore --into
    let output = test_env.run_jj_in(&repo_path, ["restore", "--into=main"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit bcab555fc80e is immutable
    Hint: Could not modify commit: mzvwutvl bcab555f main | (conflict) merge
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // split
    let output = test_env.run_jj_in(&repo_path, ["split", "-r=main"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit bcab555fc80e is immutable
    Hint: Could not modify commit: mzvwutvl bcab555f main | (conflict) merge
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // squash -r
    let output = test_env.run_jj_in(&repo_path, ["squash", "-r=description(b)"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit 72e1b68cbcf2 is immutable
    Hint: Could not modify commit: kkmpptxz 72e1b68c b
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 4 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // squash --from
    let output = test_env.run_jj_in(&repo_path, ["squash", "--from=main"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit bcab555fc80e is immutable
    Hint: Could not modify commit: mzvwutvl bcab555f main | (conflict) merge
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // squash --into
    let output = test_env.run_jj_in(&repo_path, ["squash", "--into=main"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Error: Commit bcab555fc80e is immutable
    Hint: Could not modify commit: mzvwutvl bcab555f main | (conflict) merge
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
    // unsquash
    let output = test_env.run_jj_in(&repo_path, ["unsquash", "-r=main"]);
    insta::assert_snapshot!(output, @r##"
    ------- stderr -------
    Warning: `jj unsquash` is deprecated; use `jj diffedit --restore-descendants` or `jj squash` instead
    Warning: `jj unsquash` will be removed in a future version, and this will be a hard error
    Error: Commit bcab555fc80e is immutable
    Hint: Could not modify commit: mzvwutvl bcab555f main | (conflict) merge
    Hint: Immutable commits are used to protect shared history.
    Hint: For more information, see:
          - https://jj-vcs.github.io/jj/latest/config/#set-of-immutable-commits
          - `jj help -k config`, "Set of immutable commits"
    Hint: This operation would rewrite 1 immutable commits.
    [EOF]
    [exit status: 1]
    "##);
}
