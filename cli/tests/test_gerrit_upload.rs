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
use crate::common::create_commit_with_files;

#[test]
fn test_gerrit_upload_dryrun() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["a"]);
    let output = work_dir.run_jj(["gerrit", "upload", "-r", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: No remote specified, and no 'gerrit' remote was found
    [EOF]
    [exit status: 1]
    ");

    // With remote specified but.
    test_env.add_config(r#"gerrit.default-remote="origin""#);
    let output = work_dir.run_jj(["gerrit", "upload", "-r", "b"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: The remote 'origin' (configured via `gerrit.default-remote`) does not exist
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["gerrit", "upload", "-r", "b", "--remote=origin"]);
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: No target branch specified via --remote-branch, and no 'gerrit.default-remote-branch' was found
    [EOF]
    [exit status: 1]
    ");

    test_env.add_config(r#"gerrit.default-remote-branch="main""#);
    let output = work_dir.run_jj(["gerrit", "upload", "-r", "b", "--dry-run"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Found 1 heads to push to Gerrit (remote 'origin'), target branch 'main'
    Dry-run: Would push zsuskuln 123b4d91 b | b
    [EOF]
    ");

    let output = work_dir.run_jj(["gerrit", "upload", "-r", "b", "--dry-run", "-b", "other"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Found 1 heads to push to Gerrit (remote 'origin'), target branch 'other'
    Dry-run: Would push zsuskuln 123b4d91 b | b
    [EOF]
    ");
}

#[test]
fn test_gerrit_upload_default_revision() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj([
            "git",
            "remote",
            "add",
            "origin",
            "http://example.com/repo/foo",
        ])
        .success();
    test_env.add_config(r#"gerrit.default-remote="origin""#);
    test_env.add_config(r#"gerrit.default-remote-branch="main""#);

    work_dir
        .run_jj(["new", "--message", "parent", "root()"])
        .success();
    work_dir.write_file("parent", "parent");
    let output = work_dir.run_jj(["gerrit", "upload", "--dry-run"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    No revision provided. Defaulting to @
    Found 1 heads to push to Gerrit (remote 'origin'), target branch 'main'
    Dry-run: Would push kkmpptxz a41ea4e9 parent
    [EOF]
    ");

    work_dir.run_jj(["new"]).success();
    let output = work_dir.run_jj(["gerrit", "upload", "--dry-run"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    No revision provided and @ has no description. Defaulting to @-
    Found 1 heads to push to Gerrit (remote 'origin'), target branch 'main'
    Dry-run: Would push kkmpptxz a41ea4e9 parent
    [EOF]
    ");

    work_dir.run_jj(["new", "@", "@-"]).success();
    let output = work_dir.run_jj(["gerrit", "upload", "--dry-run"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: No revision provided, and @ is a merge commit with no description. Unable to determine a suitable default commit to upload.
    Hint: Explicitly specify a revision to upload with `-r`
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_gerrit_upload_option_failure() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // upload options are validated before anything else
    // malformed custom option
    let output = work_dir.run_jj(["gerrit", "upload", "--custom", "foo"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Custom values must be of the form 'key:value'. Got foo
    [EOF]
    [exit status: 1]
    ");

    // mutually exclusive flags
    let output = work_dir.run_jj(["gerrit", "upload", "--wip", "--ready"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: --wip and --ready are mutually exclusive
    [EOF]
    [exit status: 1]
    ");
    let output = work_dir.run_jj(["gerrit", "upload", "--private", "--remove-private"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: --private and --remove-private are mutually exclusive
    [EOF]
    [exit status: 1]
    ");
    let output = work_dir.run_jj([
        "gerrit",
        "upload",
        "--publish-comments",
        "--no-publish-comments",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: --publish-comments and --no-publish-comments are mutually exclusive
    [EOF]
    [exit status: 1]
    ");

    // cannot skip validation without submitting
    let output = work_dir.run_jj(["gerrit", "upload", "--skip-validation"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: --skip-validation is only supported for --submit
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_gerrit_upload_failure() {
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

    // construct test revisions
    create_commit_with_files(&local_dir, "b", &["a@origin"], &[]);
    create_commit(&local_dir, "c", &["a@origin"]);
    local_dir.run_jj(["describe", "-m="]).success();
    create_commit(&local_dir, "d", &["a@origin"]);

    let output = local_dir.run_jj(["gerrit", "upload", "-r", "none()", "--remote-branch=main"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    No revisions to upload.
    [EOF]
    ");

    // empty revisions are not allowed
    let output = local_dir.run_jj(["gerrit", "upload", "-r", "b", "--remote-branch=main"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Refusing to upload revision mzvwutvlkqwt because it is empty
    Hint: Perhaps you squashed then ran upload? Maybe you meant to upload the parent commit instead (eg. @-)
    [EOF]
    [exit status: 1]
    ");

    // empty descriptions are not allowed
    let output = local_dir.run_jj(["gerrit", "upload", "-r", "c", "--remote-branch=main"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Refusing to upload revision yqosqzytrlsw because it is has no description
    Hint: Maybe you meant to upload the parent commit instead (eg. @-)
    [EOF]
    [exit status: 1]
    ");

    // upload failure
    local_dir
        .run_jj(["git", "remote", "set-url", "origin", "nonexistent"])
        .success();
    let output = local_dir.run_jj(["gerrit", "upload", "-r", "d", "--remote-branch=main"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Found 1 heads to push to Gerrit (remote 'origin'), target branch 'main'
    Pushing znkkpsqq 47f1f88c d | d
    Error: Internal git error while pushing to gerrit
    Caused by: Could not find repository at '$TEST_ENV/local/nonexistent'
    [EOF]
    [exit status: 1]
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

    // Ensure other trailers are preserved (no extra newlines)
    local_dir
        .run_jj([
            "describe",
            "c",
            "-m",
            "c\n\nSigned-off-by: Lucky K Maintainer <lucky@maintainer.example.org>\n",
        ])
        .success();

    // The output should only mention commit IDs from the log output above (no
    // temporary commits)
    let output = local_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @"
    @  yqosqzyt test.user@example.com 2001-02-03 08:05:15 c f6e97ced
    │  c
    ○  mzvwutvl test.user@example.com 2001-02-03 08:05:12 b 3bcb28c4
    │  b
    ◆  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 a@origin 7d980be7
    │  a
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    let output = local_dir.run_jj(["gerrit", "upload", "-r", "c", "--remote-branch=main"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Found 1 heads to push to Gerrit (remote 'origin'), target branch 'main'
    Pushing yqosqzyt f6e97ced c | c
    [EOF]
    ");

    // The output should be unchanged because we only add Change-Id trailers
    // transiently
    let output = local_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @"
    @  yqosqzyt test.user@example.com 2001-02-03 08:05:15 c f6e97ced
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
    insta::assert_snapshot!(output, @"
    commit 68b986d2eb820643b767ae219fb48128dcc2fc03
    Author: Test User <test.user@example.com>
    Date:   Sat Feb 3 04:05:13 2001 +0700

        c
        
        Signed-off-by: Lucky K Maintainer <lucky@maintainer.example.org>
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
fn test_gerrit_upload_local_implicit_change_id_link() {
    let test_env = TestEnvironment::default();
    test_env.add_config(
        r#"
[gerrit]
review-url = "https://gerrit.example.com/"
        "#,
    );
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

    // Ensure other trailers are preserved (no extra newlines)
    local_dir
        .run_jj([
            "describe",
            "c",
            "-m",
            "c\n\nSigned-off-by: Lucky K Maintainer <lucky@maintainer.example.org>\n",
        ])
        .success();

    // The output should only mention commit IDs from the log output above (no
    // temporary commits)
    let output = local_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @"
    @  yqosqzyt test.user@example.com 2001-02-03 08:05:15 c f6e97ced
    │  c
    ○  mzvwutvl test.user@example.com 2001-02-03 08:05:12 b 3bcb28c4
    │  b
    ◆  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 a@origin 7d980be7
    │  a
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    let output = local_dir.run_jj(["gerrit", "upload", "-r", "c", "--remote-branch=main"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Found 1 heads to push to Gerrit (remote 'origin'), target branch 'main'
    Pushing yqosqzyt f6e97ced c | c
    [EOF]
    ");

    // The output should be unchanged because we only add Link trailers
    // transiently
    let output = local_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @"
    @  yqosqzyt test.user@example.com 2001-02-03 08:05:15 c f6e97ced
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
    commit b2731737e530be944c12679a86dacca2a3d3c6ad
    Author: Test User <test.user@example.com>
    Date:   Sat Feb 3 04:05:13 2001 +0700

        c
        
        Signed-off-by: Lucky K Maintainer <lucky@maintainer.example.org>
        Link: https://gerrit.example.com/id/I19b790168e73f7a73a98deae21e807c06a6a6964

    commit 9bc0339b54de4f3bcf241f8d68daf75bd6501cff
    Author: Test User <test.user@example.com>
    Date:   Sat Feb 3 04:05:11 2001 +0700

        b
        
        Link: https://gerrit.example.com/id/Id043564ef93650b06a70f92f9d91912b6a6a6964

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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: mzvwutvl 887a7016 b | b
    Parent commit (@-)      : rlvkpnrz 7d980be7 a@origin | a
    [EOF]
    ");

    create_commit(&local_dir, "c", &["b"]);

    // Add an explicit Link footer to c
    let output = local_dir.run_jj([
        "describe",
        "c",
        "-m",
        "c\n\nLink: https://gerrit.example.com/id/Idfac1e8c149efddf5c7a286f787b43886a6a6964\n",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: vruxwmqv b4124fc9 c | c
    Parent commit (@-)      : mzvwutvl 887a7016 b | b
    [EOF]
    ");

    // The output should only mention commit IDs from the log output above (no
    // temporary commits)
    let output = local_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @"
    @  vruxwmqv test.user@example.com 2001-02-03 08:05:16 c b4124fc9
    │  c
    ○  mzvwutvl test.user@example.com 2001-02-03 08:05:13 b 887a7016
    │  b
    ◆  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 a@origin 7d980be7
    │  a
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    let output = local_dir.run_jj(["gerrit", "upload", "-r", "c", "--remote-branch=main"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Found 1 heads to push to Gerrit (remote 'origin'), target branch 'main'
    Pushing vruxwmqv b4124fc9 c | c
    [EOF]
    ");

    // The output should be unchanged because no temporary commits should have
    // been created
    let output = local_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @"
    @  vruxwmqv test.user@example.com 2001-02-03 08:05:16 c b4124fc9
    │  c
    ○  mzvwutvl test.user@example.com 2001-02-03 08:05:13 b 887a7016
    │  b
    ◆  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 a@origin 7d980be7
    │  a
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    // There's no particular reason to run this with jj util exec, it's just that
    // the infra makes it easier to run this way.
    let output = remote_dir.run_jj(["util", "exec", "--", "git", "log", "refs/for/main"]);
    insta::assert_snapshot!(output, @"
    commit b4124fc9d4694eecb4d9938cf4874cd13f1252b6
    Author: Test User <test.user@example.com>
    Date:   Sat Feb 3 04:05:14 2001 +0700

        c
        
        Link: https://gerrit.example.com/id/Idfac1e8c149efddf5c7a286f787b43886a6a6964

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
    ");
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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: yqosqzyt 8d46d915 c | c
    Parent commit (@-)      : mzvwutvl 3bcb28c4 b | b
    [EOF]
    ");

    // The output should only mention commit IDs from the log output above (no
    // temporary commits)
    let output = local_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @"
    @  yqosqzyt test.user@example.com 2001-02-03 08:05:15 c 8d46d915
    │  c
    ○  mzvwutvl test.user@example.com 2001-02-03 08:05:12 b 3bcb28c4
    │  b
    ◆  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 a@origin 7d980be7
    │  a
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    let output = local_dir.run_jj(["gerrit", "upload", "-r", "c", "--remote-branch=main"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Found 1 heads to push to Gerrit (remote 'origin'), target branch 'main'
    Pushing yqosqzyt 8d46d915 c | c
    [EOF]
    ");

    // The output should be unchanged because commits created within 'upload'
    // should all be temporary
    let output = local_dir.run_jj(["log", "-r", "all()"]);
    insta::assert_snapshot!(output, @"
    @  yqosqzyt test.user@example.com 2001-02-03 08:05:15 c 8d46d915
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
    insta::assert_snapshot!(output, @"
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
    ");
}

#[test]
fn test_gerrit_upload_bad_change_ids() {
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
    create_commit(&local_dir, "b2", &["b"]);
    create_commit(&local_dir, "b3", &["b2"]);
    create_commit(&local_dir, "b4", &["b3"]);
    create_commit(&local_dir, "c", &["a@origin"]);
    create_commit(&local_dir, "d", &["a@origin"]);
    create_commit(&local_dir, "e", &["a@origin"]);

    local_dir
        .run_jj(["describe", "-rb", "-m\n\nChange-Id: malformed\n"])
        .success();
    local_dir
        .run_jj([
            "describe",
            "-rb2",
            "-m\n\nChange-Id: i0000000000000000000000000000000000000000\n",
        ])
        .success();
    local_dir
        .run_jj(["describe", "-rb3", "-m\n\nLink: malformed\n"])
        .success();
    local_dir
        .run_jj([
            "describe",
            "-rb4",
            "-m\n\nLink: https://gerrit.example.com/id/Imalformed\n",
        ])
        .success();
    local_dir
        .run_jj([
            "describe",
            "-rc",
            "-m",
            concat!(
                "\n\n",
                "Change-Id: I1111111111111111111111111111111111111111\n",
                "Change-Id: I2222222222222222222222222222222222222222\n",
            ),
        ])
        .success();
    local_dir
        .run_jj([
            "describe",
            "-rd",
            "-m",
            concat!(
                "\n\n",
                "Link: https://gerrit.example.com/id/I1111111111111111111111111111111111111111\n",
                "Change-Id: I2222222222222222222222222222222222222222\n",
            ),
        ])
        .success();
    local_dir
        .run_jj([
            "describe",
            "-re",
            "-m",
            concat!(
                "\n\n",
                "Link: https://gerrit.example.com/id/I1111111111111111111111111111111111111111\n",
                "Link: https://gerrit.example.com/id/I2222222222222222222222222222222222222222\n",
            ),
        ])
        .success();

    let output = local_dir.run_jj(["gerrit", "upload", "-rc", "--remote-branch=main"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Multiple Change-Id footers in revision wqnwkozpkust
    [EOF]
    [exit status: 1]
    ");
    let output = local_dir.run_jj(["gerrit", "upload", "-rd", "--remote-branch=main"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Multiple Change-Id footers in revision kxryzmorwvtz
    [EOF]
    [exit status: 1]
    ");
    let output = local_dir.run_jj(["gerrit", "upload", "-re", "--remote-branch=main"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Error: Multiple Change-Id footers in revision uyznsvlquzzm
    [EOF]
    [exit status: 1]
    ");

    // check both badly and slightly malformed Change-Id / Link trailers
    let output = local_dir.run_jj(["gerrit", "upload", "-rb4", "--remote-branch=main"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Invalid Change-Id footer in revision mzvwutvlkqwt
    Warning: Invalid Change-Id footer in revision yqosqzytrlsw
    Warning: Invalid Link footer in revision yostqsxwqrlt
    Warning: Invalid Link footer in revision kpqxywonksrl
    Found 1 heads to push to Gerrit (remote 'origin'), target branch 'main'
    Pushing kpqxywon 69536ef3 b4
    [EOF]
    ");
}

#[test]
fn test_gerrit_upload_rejected_by_remote() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "remote"])
        .success();
    let remote_dir = test_env.work_dir("remote");
    create_commit(&remote_dir, "a", &[]);

    // create a hook on the remote that prevents pushing
    let hook_path = test_env
        .env_root()
        .join("remote")
        .join(".git")
        .join("hooks")
        .join("update");

    std::fs::write(&hook_path, "#!/bin/sh\nexit 1").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

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
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Working copy  (@) now at: mzvwutvl 887a7016 b | b
    Parent commit (@-)      : rlvkpnrz 7d980be7 a@origin | a
    [EOF]
    ");

    let output = local_dir.run_jj(["gerrit", "upload", "-r", "b", "--remote-branch=main"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Found 1 heads to push to Gerrit (remote 'origin'), target branch 'main'
    Pushing mzvwutvl 887a7016 b | b
    remote: error: hook declined to update refs/for/main        
    Warning: The remote rejected the following updates:
      refs/for/main (reason: hook declined)
    Hint: Try checking if you have permission to push to all the bookmarks.
    Error: Failed to push all changes to gerrit
    [EOF]
    [exit status: 1]
    ");
}
