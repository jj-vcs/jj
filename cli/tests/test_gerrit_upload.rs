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
