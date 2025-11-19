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

#[test]
fn test_gerrit_upload_dryrun() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["a"]);
    let output = work_dir.run_jj(["gerrit", "upload", "-r", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No remote specified, and no 'gerrit' remote was found
    [EOF]
    [exit status: 1]
    ");

    // With remote specified but.
    test_env.add_config(r#"gerrit.default-remote="origin""#);
    let output = work_dir.run_jj(["gerrit", "upload", "-r", "b"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The remote 'origin' (configured via `gerrit.default-remote`) does not exist
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["gerrit", "upload", "-r", "b", "--remote=origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: The remote 'origin' (specified via `--remote`) does not exist
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj([
        "git",
        "remote",
        "add",
        "origin",
        "http://example.com/repo/foo",
    ]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["gerrit", "upload", "-r", "b", "--remote=origin"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No target branch specified via --remote-branch, and no 'gerrit.default-remote-branch' was found
    [EOF]
    [exit status: 1]
    ");

    test_env.add_config(r#"gerrit.default-remote-branch="main""#);
    let output = work_dir.run_jj(["gerrit", "upload", "-r", "b", "--dry-run"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------

    Found 1 heads to push to Gerrit (remote 'origin'), target branch 'main'

    Dry-run: Would push zsuskuln 123b4d91 b | b
    [EOF]
    ");

    let output = work_dir.run_jj(["gerrit", "upload", "-r", "b", "--dry-run", "-b", "other"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------

    Found 1 heads to push to Gerrit (remote 'origin'), target branch 'other'

    Dry-run: Would push zsuskuln 123b4d91 b | b
    [EOF]
    ");
}

#[test]
fn test_gerrit_upload_local_implicit_change_ids() {
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

    // The output should only mention commit IDs from the log output above (no
    // temporary commits)
    let output = local_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r"
    @  yqosqzyt test.user@example.com 2001-02-03 08:05:14 c 9590bf26
    │  c
    ○  mzvwutvl test.user@example.com 2001-02-03 08:05:12 b 3bcb28c4
    │  b
    ◆  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 a@origin 7d980be7
    │  a
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    let output = local_dir.run_jj(["gerrit", "upload", "-r", "c", "--remote-branch=main"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------

    Found 1 heads to push to Gerrit (remote 'origin'), target branch 'main'

    Pushing yqosqzyt 9590bf26 c | c
    [EOF]
    ");

    // The output should be unchanged because we only add Change-Id trailers
    // transiently
    let output = local_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r"
    @  yqosqzyt test.user@example.com 2001-02-03 08:05:14 c 9590bf26
    │  c
    ○  mzvwutvl test.user@example.com 2001-02-03 08:05:12 b 3bcb28c4
    │  b
    ◆  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 a@origin 7d980be7
    │  a
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    // There's no particular reason to run this with jj util exec, it's just that
    // the infra makes it easier to run this way.
    let output = remote_dir.run_jj(["util", "exec", "--", "git", "log", "refs/for/main"]);
    insta::assert_snapshot!(output, @r"
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
    ");
}

#[test]
fn test_gerrit_upload_local_explicit_change_ids() {
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

    // Add an explicit Change-Id footer to b
    let output = local_dir.run_jj([
        "describe",
        "b",
        "-m",
        "b\n\nChange-Id: Id39b308212fe7e0b746d16c13355f3a90712d7f9\n",
    ]);
    insta::assert_snapshot!(output, @r###"
    ------- stderr -------
    Working copy  (@) now at: mzvwutvl 887a7016 b | b
    Parent commit (@-)      : rlvkpnrz 7d980be7 a@origin | a
    [EOF]
    "###);

    create_commit(&local_dir, "c", &["b"]);

    // Add an explicit Change-Id footer to c
    let output = local_dir.run_jj([
        "describe",
        "c",
        "-m",
        "c\n\nChange-Id: Idfac1e8c149efddf5c7a286f787b43886a6a6964\n",
    ]);
    insta::assert_snapshot!(output, @r###"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv 27c0bdd0 c | c
    Parent commit (@-)      : mzvwutvl 887a7016 b | b
    [EOF]
    "###);

    // The output should only mention commit IDs from the log output above (no
    // temporary commits)
    let output = local_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r###"
    @  vruxwmqv test.user@example.com 2001-02-03 08:05:16 c 27c0bdd0
    │  c
    ○  mzvwutvl test.user@example.com 2001-02-03 08:05:13 b 887a7016
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

    Pushing vruxwmqv 27c0bdd0 c | c
    [EOF]
    "###);

    // The output should be unchanged because no temporary commits should have
    // been created
    let output = local_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r###"
    @  vruxwmqv test.user@example.com 2001-02-03 08:05:16 c 27c0bdd0
    │  c
    ○  mzvwutvl test.user@example.com 2001-02-03 08:05:13 b 887a7016
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
    commit 27c0bdd070a0dd4aec5a22bd3ea454cd2502f430
    Author: Test User <test.user@example.com>
    Date:   Sat Feb 3 04:05:14 2001 +0700

        c
        
        Change-Id: Idfac1e8c149efddf5c7a286f787b43886a6a6964

    commit 887a7016ec03a904835da1059543d8cc34b6ba76
    Author: Test User <test.user@example.com>
    Date:   Sat Feb 3 04:05:11 2001 +0700

        b
        
        Change-Id: Id39b308212fe7e0b746d16c13355f3a90712d7f9

    commit 7d980be7a1d499e4d316ab4c01242885032f7eaf
    Author: Test User <test.user@example.com>
    Date:   Sat Feb 3 04:05:08 2001 +0700

        a
    [EOF]
    "###);
}

#[test]
fn test_gerrit_upload_local_mixed_change_ids() {
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

    // Add an explicit Change-Id footer to c but not b
    let output = local_dir.run_jj([
        "describe",
        "c",
        "-m",
        "c\n\nChange-Id: Id39b308212fe7e0b746d16c13355f3a90712d7f9\n",
    ]);
    insta::assert_snapshot!(output, @r###"
    ------- stderr -------
    Working copy  (@) now at: yqosqzyt 8d46d915 c | c
    Parent commit (@-)      : mzvwutvl 3bcb28c4 b | b
    [EOF]
    "###);

    // The output should only mention commit IDs from the log output above (no
    // temporary commits)
    let output = local_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r###"
    @  yqosqzyt test.user@example.com 2001-02-03 08:05:15 c 8d46d915
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

    Pushing yqosqzyt 8d46d915 c | c
    [EOF]
    "###);

    // The output should be unchanged because commits created within 'upload'
    // should all be temporary
    let output = local_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @r###"
    @  yqosqzyt test.user@example.com 2001-02-03 08:05:15 c 8d46d915
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
    commit 015df2b1d38bdc71ae7ef24c2889100e39d34ef8
    Author: Test User <test.user@example.com>
    Date:   Sat Feb 3 04:05:13 2001 +0700

        c
        
        Change-Id: Id39b308212fe7e0b746d16c13355f3a90712d7f9

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
