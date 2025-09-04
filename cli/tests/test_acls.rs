// Copyright 2024 The Jujutsu Authors
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

use jj_lib::secret_backend::SecretBackend;

use crate::common::TestEnvironment;

#[test]
fn test_diff() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.create_dir("dir");
    work_dir.write_file("a-first", "foo\n");
    work_dir.write_file("deleted-secret", "foo\n");
    work_dir.write_file("dir/secret", "foo\n");
    work_dir.write_file("modified-secret", "foo\n");
    work_dir.write_file("z-last", "foo\n");
    work_dir.run_jj(["new"]).success();
    work_dir.write_file("a-first", "bar\n");
    work_dir.remove_file("deleted-secret");
    work_dir.write_file("added-secret", "bar\n");
    work_dir.write_file("dir/secret", "bar\n");
    work_dir.write_file("modified-secret", "bar\n");
    work_dir.write_file("z-last", "bar\n");

    SecretBackend::adopt_git_repo(work_dir.root());

    let output = work_dir.run_jj(["diff", "--color-words"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    Modified regular file a-first:
       1    1: foobar
    Access denied to added-secret: No access
    Access denied to deleted-secret: No access
    Access denied to dir/secret: No access
    Access denied to modified-secret: No access
    Modified regular file z-last:
       1    1: foobar
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "--summary"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    M a-first
    C {a-first => added-secret}
    D deleted-secret
    M dir/secret
    M modified-secret
    M z-last
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "--types"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    FF a-first
    FF {a-first => added-secret}
    F- deleted-secret
    FF dir/secret
    FF modified-secret
    FF z-last
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "--stat"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    a-first                   | 2 +-
    {a-first => added-secret} | 2 +-
    deleted-secret            | 1 -
    dir/secret                | 0
    modified-secret           | 0
    z-last                    | 2 +-
    6 files changed, 3 insertions(+), 4 deletions(-)
    [EOF]
    ");
    let output = work_dir.run_jj(["diff", "--git"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    diff --git a/a-first b/a-first
    index 257cc5642c..5716ca5987 100644
    --- a/a-first
    +++ b/a-first
    @@ -1,1 +1,1 @@
    -foo
    +bar
    [EOF]
    ------- stderr -------
    Error: Access denied to added-secret
    Caused by: No access
    [EOF]
    [exit status: 1]
    ");

    // TODO: Test external tool
}

#[test]
fn test_file_list_show() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("a-first", "foo\n");
    work_dir.write_file("secret", "bar\n");
    work_dir.write_file("z-last", "baz\n");

    SecretBackend::adopt_git_repo(work_dir.root());

    // "file list" should just work since it doesn't access file content
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @r"
    a-first
    secret
    z-last
    [EOF]
    ");

    let output = work_dir.run_jj(["file", "show", "."]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    foo
    baz
    [EOF]
    ------- stderr -------
    Warning: Path 'secret' exists but access is denied: No access
    [EOF]
    ");
}
