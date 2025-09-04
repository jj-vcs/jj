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

use crate::common::TestEnvironment;

#[test]
fn test_simple_rename() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new"]).success();
    work_dir.write_file("original", "original");
    work_dir.write_file("something", "something");
    work_dir.run_jj(["commit", "-mfirst"]).success();
    work_dir.remove_file("original");
    work_dir.write_file("modified", "original");
    work_dir.write_file("something", "changed");
    insta::assert_snapshot!(
        work_dir.run_jj(["debug", "copy-detection"]).normalize_backslash(), @r"
    original -> modified
    [EOF]
    ");
}
