// Copyright 2026 The Jujutsu Authors
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
fn test_graft_tree_basic() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create a linear chain of commits with files under src/foo
    work_dir
        .run_jj(["new", "root()", "-m", "add file1"])
        .success();
    work_dir.write_file("src/foo/file1.txt", "content1\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();

    work_dir.run_jj(["new", "a", "-m", "add file2"]).success();
    work_dir.write_file("src/foo/file2.txt", "content2\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();

    work_dir
        .run_jj(["new", "b", "-m", "modify file1"])
        .success();
    work_dir.write_file("src/foo/file1.txt", "modified content1\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "c"])
        .success();

    // Set up a destination commit
    work_dir.run_jj(["new", "root()", "-m", "base"]).success();
    work_dir.write_file("existing.txt", "existing\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "base"])
        .success();

    // Graft the commits
    let output = work_dir.run_jj([
        "graft",
        "tree",
        "--from",
        "a | b | c",
        "--path",
        "src/foo",
        "--onto",
        "vendor/foo",
        "--destination",
        "base",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Grafted rlvkpnrz b03b34b5 a | add file1 as uuzqqzqu ba201572 add file1
    Grafted zsuskuln 48427b33 b | add file2 as znxxwsul 06df95eb add file2
    Grafted royxmykx fc336f54 c | modify file1 as twsrurpv 71b3717e modify file1
    [EOF]
    ");

    // Verify the grafted commits have files at the translated paths.
    // Use children(base) to find the first grafted commit.
    let output = work_dir.run_jj(["file", "list", "-r", "children(base)"]);
    insta::assert_snapshot!(output, @"
    existing.txt
    vendor/foo/file1.txt
    [EOF]
    ");

    // Check the log to see the structure of grafted commits
    let output = work_dir.run_jj([
        "log",
        "-r",
        "descendants(base)",
        "-T",
        r#"separate(" ", commit_id.short(), description.first_line())"#,
        "--no-graph",
    ]);
    insta::assert_snapshot!(output, @"71b3717ef301 modify file106df95ebed13 add file2ba20157228d6 add file104766e24fc97 base[EOF]");

    // Verify file content was preserved on the head of the grafted chain
    let output = work_dir.run_jj([
        "file",
        "show",
        "-r",
        "heads(descendants(base))",
        "vendor/foo/file1.txt",
    ]);
    insta::assert_snapshot!(output, @"
    modified content1
    [EOF]
    ");

    // Also verify file2 exists
    let output = work_dir.run_jj([
        "file",
        "show",
        "-r",
        "heads(descendants(base))",
        "vendor/foo/file2.txt",
    ]);
    insta::assert_snapshot!(output, @"
    content2
    [EOF]
    ");
}

#[test]
fn test_graft_tree_skip_empty() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create A (touches src/foo), B (only touches other/), C (touches src/foo)
    work_dir
        .run_jj(["new", "root()", "-m", "A: add foo file"])
        .success();
    work_dir.write_file("src/foo/a.txt", "a\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();

    work_dir
        .run_jj(["new", "a", "-m", "B: other change"])
        .success();
    work_dir.write_file("other/b.txt", "b\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();

    work_dir
        .run_jj(["new", "b", "-m", "C: add another foo file"])
        .success();
    work_dir.write_file("src/foo/c.txt", "c\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "c"])
        .success();

    // Set up destination
    work_dir.run_jj(["new", "root()", "-m", "dest"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "dest"])
        .success();

    let output = work_dir.run_jj([
        "graft",
        "tree",
        "--from",
        "a | b | c",
        "--path",
        "src/foo",
        "--onto",
        "vendor/foo",
        "--destination",
        "dest",
    ]);
    // B should be skipped because it doesn't touch src/foo
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Grafted rlvkpnrz fe537245 a | A: add foo file as uuzqqzqu a6183293 A: add foo file
    Grafted royxmykx beac2696 c | C: add another foo file as znxxwsul 1f3116bf C: add another foo file
    [EOF]
    ");

    // Verify the head grafted commit has both files (C' has a.txt and c.txt)
    let output = work_dir.run_jj(["file", "list", "-r", "heads(descendants(dest))"]);
    insta::assert_snapshot!(output, @"
    vendor/foo/a.txt
    vendor/foo/c.txt
    [EOF]
    ");
}

#[test]
fn test_graft_tree_default_destination() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create a commit with files under src/
    work_dir
        .run_jj(["new", "root()", "-m", "source commit"])
        .success();
    work_dir.write_file("src/file.txt", "hello\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "source"])
        .success();

    // Create a base and working copy
    work_dir.run_jj(["new", "root()", "-m", "base"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "base"])
        .success();
    work_dir.run_jj(["new", "base", "-m", "wc"]).success();

    // Graft without --destination, should use @- (which is "base")
    let output = work_dir.run_jj([
        "graft", "tree", "--from", "source", "--path", "src", "--onto", "vendor",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Grafted rlvkpnrz baf688ae source | source commit as spxsnpux 2e73f3ad source commit
    [EOF]
    ");
}

#[test]
fn test_graft_tree_no_matching_commits() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create commits that don't touch the source path at all
    work_dir
        .run_jj(["new", "root()", "-m", "unrelated"])
        .success();
    work_dir.write_file("other/file.txt", "other\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "unrelated"])
        .success();

    work_dir.run_jj(["new", "root()", "-m", "dest"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "dest"])
        .success();

    let output = work_dir.run_jj([
        "graft",
        "tree",
        "--from",
        "unrelated",
        "--path",
        "src/foo",
        "--onto",
        "vendor/foo",
        "--destination",
        "dest",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    No revisions to graft.
    Nothing changed.
    [EOF]
    ");
}

#[test]
fn test_graft_tree_cross_repo() {
    let test_env = TestEnvironment::default();

    // Create the "upstream" repo with some commits
    test_env
        .run_jj_in(".", ["git", "init", "upstream"])
        .success();
    let upstream_dir = test_env.work_dir("upstream");

    upstream_dir
        .run_jj(["new", "root()", "-m", "upstream: initial"])
        .success();
    upstream_dir.write_file("src/lib.rs", "fn main() {}\n");
    upstream_dir.write_file("src/util.rs", "fn util() {}\n");
    upstream_dir
        .run_jj(["bookmark", "create", "-r@", "main"])
        .success();

    upstream_dir
        .run_jj(["new", "main", "-m", "upstream: add feature"])
        .success();
    upstream_dir.write_file("src/feature.rs", "fn feature() {}\n");
    upstream_dir
        .run_jj(["bookmark", "set", "-r@", "main"])
        .success();

    // Export git refs so we can fetch
    upstream_dir.run_jj(["git", "export"]).success();

    // Create the "local" repo
    test_env.run_jj_in(".", ["git", "init", "local"]).success();
    let local_dir = test_env.work_dir("local");

    // Set up some local content
    local_dir
        .run_jj(["new", "root()", "-m", "local base"])
        .success();
    local_dir.write_file("README.md", "# My Project\n");
    local_dir
        .run_jj(["bookmark", "create", "-r@", "local-main"])
        .success();
    local_dir.run_jj(["new", "local-main"]).success();

    // Add upstream as a remote and fetch
    local_dir
        .run_jj([
            "git",
            "remote",
            "add",
            "upstream",
            "../upstream/.jj/repo/store/git",
        ])
        .success();
    local_dir
        .run_jj(["git", "fetch", "--remote", "upstream"])
        .success();

    // Graft upstream commits, translating src/ → vendor/upstream/
    let output = local_dir.run_jj([
        "graft",
        "tree",
        "--from",
        "::main@upstream",
        "--path",
        "src",
        "--onto",
        "vendor/upstream",
        "--destination",
        "local-main",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Grafted rlvkpnrz c6c4dd1f upstream: initial as mouksmqu 7f7f8a92 upstream: initial
    Grafted zsuskuln 3291ef5d main@upstream | upstream: add feature as uvqyutox 6eb7e58f upstream: add feature
    [EOF]
    ");

    // Verify the grafted commits have vendor/upstream/ paths
    // Use "children(local-main) ~ @" to exclude the working copy
    let output = local_dir.run_jj(["file", "list", "-r", "children(local-main) ~ @"]);
    insta::assert_snapshot!(output, @"
    README.md
    vendor/upstream/lib.rs
    vendor/upstream/util.rs
    [EOF]
    ");

    let output = local_dir.run_jj(["file", "list", "-r", "heads(descendants(local-main) ~ @)"]);
    insta::assert_snapshot!(output, @"
    README.md
    vendor/upstream/feature.rs
    vendor/upstream/lib.rs
    vendor/upstream/util.rs
    [EOF]
    ");

    // Verify file content was preserved
    let output = local_dir.run_jj([
        "file",
        "show",
        "-r",
        "heads(descendants(local-main) ~ @)",
        "vendor/upstream/feature.rs",
    ]);
    insta::assert_snapshot!(output, @"
    fn feature() {}
    [EOF]
    ");
}

#[test]
fn test_graft_tree_merge_commit() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Build a diamond graph:
    //   A (src/foo/a.txt)
    //  / \
    // B   C  (both add files under src/foo/)
    //  \ /
    //   D (merge commit, adds src/foo/d.txt)
    work_dir
        .run_jj(["new", "root()", "-m", "A"])
        .success();
    work_dir.write_file("src/foo/a.txt", "a\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();

    work_dir.run_jj(["new", "a", "-m", "B"]).success();
    work_dir.write_file("src/foo/b.txt", "b\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();

    work_dir.run_jj(["new", "a", "-m", "C"]).success();
    work_dir.write_file("src/foo/c.txt", "c\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "c"])
        .success();

    work_dir.run_jj(["new", "b", "c", "-m", "D"]).success();
    work_dir.write_file("src/foo/d.txt", "d\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "d"])
        .success();

    // Set up destination
    work_dir
        .run_jj(["new", "root()", "-m", "dest"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "dest"])
        .success();

    // Graft the diamond
    let output = work_dir.run_jj([
        "graft",
        "tree",
        "--from",
        "a | b | c | d",
        "--path",
        "src/foo",
        "--onto",
        "vendor/foo",
        "--destination",
        "dest",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Grafted rlvkpnrz e9c3c719 a | A as msksykpx b7153d5f A
    Grafted zsuskuln 6392a98f b | B as yptpptny 3486759c B
    Grafted royxmykx f56177b0 c | C as nnmtxzwn f127badb C
    Grafted vruxwmqv df6b0da5 d | D as tqxkopmw 5d4101ad D
    [EOF]
    ");

    // Show the graph structure of grafted commits — D' should be a merge of B' and C'
    let output = work_dir.run_jj([
        "log",
        "-r",
        "descendants(dest) ~ dest",
        "-T",
        r#"separate(" ", change_id.short(), description.first_line())"#,
    ]);
    insta::assert_snapshot!(output, @"
    ○    tqxkopmwmltt D
    ├─╮
    │ ○  nnmtxzwnrlpn C
    ○ │  yptpptnyuprq B
    ├─╯
    ○  msksykpxotkr A
    │
    ~
    [EOF]
    ");

    // Verify D' has all four files
    let output = work_dir.run_jj(["file", "list", "-r", "heads(descendants(dest))"]);
    insta::assert_snapshot!(output, @"
    vendor/foo/a.txt
    vendor/foo/b.txt
    vendor/foo/c.txt
    vendor/foo/d.txt
    [EOF]
    ");
}

#[test]
fn test_graft_tree_merge_one_branch_skipped() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Diamond where one branch doesn't touch source path:
    //   A (src/foo/a.txt)
    //  / \
    // B   C
    //  \ /
    //   D (src/foo/d.txt)
    // B touches src/foo/ but C only touches other/
    // Result: D' should NOT be a merge — C' is skipped, so D' parents on B' only.
    work_dir
        .run_jj(["new", "root()", "-m", "A"])
        .success();
    work_dir.write_file("src/foo/a.txt", "a\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();

    work_dir.run_jj(["new", "a", "-m", "B"]).success();
    work_dir.write_file("src/foo/b.txt", "b\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();

    work_dir.run_jj(["new", "a", "-m", "C"]).success();
    work_dir.write_file("other/c.txt", "c\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "c"])
        .success();

    work_dir.run_jj(["new", "b", "c", "-m", "D"]).success();
    work_dir.write_file("src/foo/d.txt", "d\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "d"])
        .success();

    work_dir
        .run_jj(["new", "root()", "-m", "dest"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "dest"])
        .success();

    let output = work_dir.run_jj([
        "graft",
        "tree",
        "--from",
        "a | b | c | d",
        "--path",
        "src/foo",
        "--onto",
        "vendor/foo",
        "--destination",
        "dest",
    ]);
    // C should be skipped (no src/foo files)
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Grafted rlvkpnrz e9c3c719 a | A as msksykpx b7153d5f A
    Grafted zsuskuln 6392a98f b | B as yptpptny 3486759c B
    Grafted vruxwmqv 9b2ead47 d | D as nnmtxzwn afea031b D
    [EOF]
    ");

    // D' is a merge of B' and A'. C was skipped and mapped to A' (its
    // inherited parent). A' is an ancestor of B' so the merge is redundant
    // but structurally correct. Use `jj simplify-parents` to clean up.
    let output = work_dir.run_jj([
        "log",
        "-r",
        "descendants(dest) ~ dest",
        "-T",
        r#"separate(" ", change_id.short(), description.first_line())"#,
    ]);
    insta::assert_snapshot!(output, @"
    ○    nnmtxzwnrlpn D
    ├─╮
    ○ │  yptpptnyuprq B
    ├─╯
    ○  msksykpxotkr A
    │
    ~
    [EOF]
    ");
}

#[test]
fn test_graft_tree_merge_both_branches_skipped() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Diamond where both branches don't touch source path:
    //   A (src/foo/a.txt)
    //  / \
    // B   C     (both only touch other/)
    //  \ /
    //   D (src/foo/d.txt)
    // B and C are both skipped, both map to A'.
    // D' should have a single parent A' (deduplication), not A' twice.
    work_dir
        .run_jj(["new", "root()", "-m", "A"])
        .success();
    work_dir.write_file("src/foo/a.txt", "a\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();

    work_dir.run_jj(["new", "a", "-m", "B"]).success();
    work_dir.write_file("other/b.txt", "b\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();

    work_dir.run_jj(["new", "a", "-m", "C"]).success();
    work_dir.write_file("other/c.txt", "c\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "c"])
        .success();

    work_dir.run_jj(["new", "b", "c", "-m", "D"]).success();
    work_dir.write_file("src/foo/d.txt", "d\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "d"])
        .success();

    work_dir
        .run_jj(["new", "root()", "-m", "dest"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "dest"])
        .success();

    let output = work_dir.run_jj([
        "graft",
        "tree",
        "--from",
        "a | b | c | d",
        "--path",
        "src/foo",
        "--onto",
        "vendor/foo",
        "--destination",
        "dest",
    ]);
    // B and C should be skipped
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Grafted rlvkpnrz e9c3c719 a | A as msksykpx b7153d5f A
    Grafted vruxwmqv 8c8b2205 d | D as yptpptny 853de519 D
    [EOF]
    ");

    // D' should be a plain child of A' (not a merge of A' and A')
    let output = work_dir.run_jj([
        "log",
        "-r",
        "descendants(dest) ~ dest",
        "-T",
        r#"separate(" ", change_id.short(), description.first_line())"#,
    ]);
    insta::assert_snapshot!(output, @"
    ○  yptpptnyuprq D
    ○  msksykpxotkr A
    │
    ~
    [EOF]
    ");
}

#[test]
fn test_graft_tree_skipped_merge_commit() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Diamond where the merge commit itself only adds non-source files:
    //   A (src/foo/a.txt)
    //  / \
    // B   C  (both touch src/foo/)
    //  \ /
    //   D     (only adds other/d.txt, but inherits src/foo/ from parents)
    //   |
    //   E (src/foo/e.txt)
    // D' is grafted as an empty merge (it has src/foo entries from parents
    // but adds no new ones), preserving the merge structure.
    work_dir
        .run_jj(["new", "root()", "-m", "A"])
        .success();
    work_dir.write_file("src/foo/a.txt", "a\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();

    work_dir.run_jj(["new", "a", "-m", "B"]).success();
    work_dir.write_file("src/foo/b.txt", "b\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();

    work_dir.run_jj(["new", "a", "-m", "C"]).success();
    work_dir.write_file("src/foo/c.txt", "c\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "c"])
        .success();

    work_dir.run_jj(["new", "b", "c", "-m", "D"]).success();
    work_dir.write_file("other/d.txt", "d\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "d"])
        .success();

    work_dir.run_jj(["new", "d", "-m", "E"]).success();
    work_dir.write_file("src/foo/e.txt", "e\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "e"])
        .success();

    work_dir
        .run_jj(["new", "root()", "-m", "dest"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "dest"])
        .success();

    let output = work_dir.run_jj([
        "graft",
        "tree",
        "--from",
        "a | b | c | d | e",
        "--path",
        "src/foo",
        "--onto",
        "vendor/foo",
        "--destination",
        "dest",
    ]);
    // D' is grafted as an empty merge preserving the B'+C' merge structure
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Grafted rlvkpnrz e9c3c719 a | A as rsllmpnm e7684405 A
    Grafted zsuskuln 6392a98f b | B as pkpnqqml a185fa53 B
    Grafted royxmykx f56177b0 c | C as smunspyn c9dc1342 C
    Grafted vruxwmqv 0bc556f0 d | D as upswqmow 35b93e80 (empty) D
    Grafted znkkpsqq 9231a599 e | E as mklrktsx 4fca05d1 E
    [EOF]
    ");

    // The full diamond structure is preserved: E' → D' → {B', C'} → A'
    let output = work_dir.run_jj([
        "log",
        "-r",
        "descendants(dest) ~ dest",
        "-T",
        r#"separate(" ", change_id.short(), description.first_line())"#,
    ]);
    insta::assert_snapshot!(output, @"
    ○  mklrktsxtqpo E
    ○    upswqmowktzu D
    ├─╮
    │ ○  smunspyntsyz C
    ○ │  pkpnqqmlowtv B
    ├─╯
    ○  rsllmpnmslon A
    │
    ~
    [EOF]
    ");

    // E' should have all files from the full chain (a, b, c, e)
    let output = work_dir.run_jj(["file", "list", "-r", "heads(descendants(dest))"]);
    insta::assert_snapshot!(output, @"
    vendor/foo/a.txt
    vendor/foo/b.txt
    vendor/foo/c.txt
    vendor/foo/e.txt
    [EOF]
    ");
}

#[test]
fn test_graft_tree_merge_unmapped_parent() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // D merges B (in the --from revset) and X (NOT in the revset):
    //   A (src/foo/a.txt)   X (src/foo/x.txt, not in revset)
    //   |                   |
    //   B (src/foo/b.txt)   |
    //    \                 /
    //     \               /
    //      D (merge, src/foo/d.txt)
    // X is not in the --from set, so its parent won't be in old_to_new.
    // D' should only have B' as parent (the mapped one).
    work_dir
        .run_jj(["new", "root()", "-m", "A"])
        .success();
    work_dir.write_file("src/foo/a.txt", "a\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "a"])
        .success();

    work_dir.run_jj(["new", "a", "-m", "B"]).success();
    work_dir.write_file("src/foo/b.txt", "b\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "b"])
        .success();

    work_dir
        .run_jj(["new", "root()", "-m", "X"])
        .success();
    work_dir.write_file("src/foo/x.txt", "x\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "x"])
        .success();

    work_dir.run_jj(["new", "b", "x", "-m", "D"]).success();
    work_dir.write_file("src/foo/d.txt", "d\n");
    work_dir
        .run_jj(["bookmark", "create", "-r@", "d"])
        .success();

    work_dir
        .run_jj(["new", "root()", "-m", "dest"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "dest"])
        .success();

    // Only graft a, b, d — NOT x
    let output = work_dir.run_jj([
        "graft",
        "tree",
        "--from",
        "a | b | d",
        "--path",
        "src/foo",
        "--onto",
        "vendor/foo",
        "--destination",
        "dest",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Grafted rlvkpnrz e9c3c719 a | A as msksykpx b7153d5f A
    Grafted zsuskuln 6392a98f b | B as yptpptny 3486759c B
    Grafted vruxwmqv 425e5e94 d | D as nnmtxzwn f6974470 D
    [EOF]
    ");

    // D' should be a child of B' only (X is not in the revset)
    let output = work_dir.run_jj([
        "log",
        "-r",
        "descendants(dest) ~ dest",
        "-T",
        r#"separate(" ", change_id.short(), description.first_line())"#,
    ]);
    insta::assert_snapshot!(output, @"
    ○  nnmtxzwnrlpn D
    ○  yptpptnyuprq B
    ○  msksykpxotkr A
    │
    ~
    [EOF]
    ");
}

#[test]
fn test_graft_tree_preserves_metadata() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create a commit with custom author metadata
    work_dir
        .run_jj(["new", "root()", "-m", "original commit"])
        .success();
    work_dir.write_file("src/file.txt", "content\n");
    // Override the author on this commit
    work_dir
        .run_jj([
            "describe",
            "--config",
            "user.name=Original Author",
            "--config",
            "user.email=original@example.com",
            "--reset-author",
            "-m",
            "original commit",
        ])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "source"])
        .success();

    // Set up destination with different user config
    work_dir
        .run_jj(["new", "root()", "-m", "dest"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "dest"])
        .success();

    // Graft — the grafted commit should preserve the original author
    work_dir
        .run_jj([
            "graft",
            "tree",
            "--from",
            "source",
            "--path",
            "src",
            "--onto",
            "vendor",
            "--destination",
            "dest",
        ])
        .success();

    // Check that the grafted commit has the original author, not the grafter's
    let output = work_dir.run_jj([
        "log",
        "-r",
        "children(dest)",
        "-T",
        r#"separate("\n", "author: " ++ author.name() ++ " <" ++ author.email() ++ ">", "committer: " ++ committer.name() ++ " <" ++ committer.email() ++ ">")"#,
        "--no-graph",
    ]);
    insta::assert_snapshot!(output, @"
    author: Original Author <original@example.com>
    committer: Original Author <original@example.com>[EOF]
    ");
}
