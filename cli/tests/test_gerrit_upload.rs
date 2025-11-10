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

use crate::common::TestEnvironment;
use crate::common::create_commit;
use crate::common::to_toml_value;

#[test]
fn test_gerrit_upload_dryrun() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["a"]);
    let output = work_dir.run_jj(["gerrit", "upload", "-r", "b"]);
    insta::assert_snapshot!(output, @r###"
    ------- stderr -------
    Error: No remote specified, and no 'gerrit' remote was found
    [EOF]
    [exit status: 1]
    "###);

    // With remote specified but.
    test_env.add_config(r#"gerrit.default-remote="origin""#);
    let output = work_dir.run_jj(["gerrit", "upload", "-r", "b"]);
    insta::assert_snapshot!(output, @r###"
    ------- stderr -------
    Error: The remote 'origin' (configured via `gerrit.default-remote`) does not exist
    [EOF]
    [exit status: 1]
    "###);

    let output = work_dir.run_jj(["gerrit", "upload", "-r", "b", "--remote=origin"]);
    insta::assert_snapshot!(output, @r###"
    ------- stderr -------
    Error: The remote 'origin' (specified via `--remote`) does not exist
    [EOF]
    [exit status: 1]
    "###);

    let output = work_dir.run_jj([
        "git",
        "remote",
        "add",
        "origin",
        "http://example.com/repo/foo",
    ]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["gerrit", "upload", "-r", "b", "--remote=origin"]);
    insta::assert_snapshot!(output, @r###"
    ------- stderr -------
    Error: No target branch specified via --remote-branch, and no 'gerrit.default-remote-branch' was found
    [EOF]
    [exit status: 1]
    "###);

    test_env.add_config(r#"gerrit.default-remote-branch="main""#);
    let output = work_dir.run_jj(["gerrit", "upload", "-r", "b", "--dry-run"]);
    insta::assert_snapshot!(output, @r###"
    ------- stderr -------

    Found 1 heads to push to Gerrit (remote 'origin'), target branch 'main'

    Dry-run: Would push zsuskuln 123b4d91 b | b
    [EOF]
    "###);

    let output = work_dir.run_jj(["gerrit", "upload", "-r", "b", "--dry-run", "-b", "other"]);
    insta::assert_snapshot!(output, @r###"
    ------- stderr -------

    Found 1 heads to push to Gerrit (remote 'origin'), target branch 'other'

    Dry-run: Would push zsuskuln 123b4d91 b | b
    [EOF]
    "###);
}

#[test]
fn test_gerrit_upload_local() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "remote"])
        .success();
    let remote_dir = test_env.work_dir("remote");
    create_commit(&remote_dir, "a", &[]);

    test_env
        .run_jj_in(".", ["git", "clone", "remote", "local"])
        .success();
    let local_dir = test_env.work_dir("local");
    create_commit(&local_dir, "b", &["a@origin"]);
    create_commit(&local_dir, "c", &["b"]);

    // The output should only mentioned commit IDs from the log output above (no
    // temporary commits)
    let output = local_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r###"
    @  yqosqzyt test.user@example.com 2001-02-03 08:05:14 c 9590bf26
    │  c
    ○  mzvwutvl test.user@example.com 2001-02-03 08:05:12 b 3bcb28c4
    │  b
    ◆  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 a@origin 7d980be7
    │  a
    ◆  zzzzzzzz root() 00000000
    [EOF]
    "###);

    let output = local_dir.run_jj(["gerrit", "upload", "-r", "c", "--remote-branch=main"]);
    insta::assert_snapshot!(output, @r###"
    ------- stderr -------

    Found 1 heads to push to Gerrit (remote 'origin'), target branch 'main'

    Pushing yqosqzyt 9590bf26 c | c
    [EOF]
    "###);

    // The output should be unchanged because we only add Change-Id trailers
    // transiently
    let output = local_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r###"
    @  yqosqzyt test.user@example.com 2001-02-03 08:05:14 c 9590bf26
    │  c
    ○  mzvwutvl test.user@example.com 2001-02-03 08:05:12 b 3bcb28c4
    │  b
    ◆  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 a@origin 7d980be7
    │  a
    ◆  zzzzzzzz root() 00000000
    [EOF]
    "###);

    // There's no particular reason to run this with jj util exec, it's just that
    // the infra makes it easier to run this way.
    let output = remote_dir.run_jj(["util", "exec", "--", "git", "log", "refs/for/main"]);
    insta::assert_snapshot!(output, @r###"
    commit ab6776c073b82fbbd2cd0858482a9646afd56f85
    Author: Test User <test.user@example.com>
    Date:   Sat Feb 3 04:05:13 2001 +0700

        c
        
        Change-Id: I19b790168e73f7a73a98deae21e807c06a6a6964

    commit 81b723522d1c1a583a045eab5bfb323e45e6198d
    Author: Test User <test.user@example.com>
    Date:   Sat Feb 3 04:05:11 2001 +0700

        b
        
        Change-Id: Id043564ef93650b06a70f92f9d91912b6a6a6964

    commit 7d980be7a1d499e4d316ab4c01242885032f7eaf
    Author: Test User <test.user@example.com>
    Date:   Sat Feb 3 04:05:08 2001 +0700

        a
    [EOF]
    "###);
}

#[test]
fn test_pre_upload_fix_hook() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "remote"])
        .success();
    let remote_dir = test_env.work_dir("remote");
    create_commit(&remote_dir, "a", &[]);

    test_env
        .run_jj_in(".", ["git", "clone", "remote", "local"])
        .success();
    let local_dir = test_env.work_dir("local");
    local_dir.write_file("file.txt", "content\n");
    local_dir.run_jj(["commit", "-m", "a"]).success();
    local_dir.write_file("file.txt", "new content\n");
    local_dir.run_jj(["describe", "-m", "b"]).success();

    let formatter_path = assert_cmd::cargo::cargo_bin("fake-formatter");
    assert!(formatter_path.is_file());
    let formatter = to_toml_value(formatter_path.to_str().unwrap());

    test_env.add_config(format!(
        r###"
        hooks.pre-upload.fix.enabled = true

        [fix.tools.my-tool]
        command = [{formatter}, "--uppercase"]
        patterns = ["file.txt"]
        "###
    ));

    let output = local_dir.run_jj(["gerrit", "upload", "--remote-branch=main", "-r", "@"]);
    insta::assert_snapshot!(output, @r###"
    ------- stderr -------
    Fixed 2 commits of 2 checked.
    Working copy  (@) now at: mzvwutvl ed78d33d b
    Parent commit (@-)      : zsuskuln 531e34f0 a
    Added 0 files, modified 1 files, removed 0 files

    Found 1 heads to push to Gerrit (remote 'origin'), target branch 'main'

    Pushing mzvwutvl ed78d33d b
    [EOF]
    "###);

    assert_eq!(local_dir.read_file("file.txt"), "NEW CONTENT\n");
    let output = local_dir.run_jj(["file", "show", "file.txt", "-r", "@"]);
    insta::assert_snapshot!(output, @r###"
    NEW CONTENT
    [EOF]
    "###);
    let output = local_dir.run_jj(["file", "show", "file.txt", "-r", "@-"]);
    insta::assert_snapshot!(output, @r###"
    CONTENT
    [EOF]
    "###);

    // The revision doesn't show up on the remote in jj.
    // Not sure why, but it doesn't really matter.
    let output = remote_dir.run_jj([
        "util",
        "exec",
        "--",
        "git",
        "cat-file",
        "-p",
        "refs/for/main:file.txt",
    ]);
    insta::assert_snapshot!(output, @r###"
    NEW CONTENT
    [EOF]
    "###);
}
