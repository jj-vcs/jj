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
use crate::common::create_commit;

#[test]
fn test_arrange_bad_revisions() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(&work_dir, "a", &[]);
    create_commit(&work_dir, "b", &["a"]);
    create_commit(&work_dir, "c", &["b"]);

    let output = work_dir.run_jj(["arrange", "-r", "none()"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    No revisions to arrange.
    [EOF]
    ");

    let output = work_dir.run_jj(["arrange", "-r", "a|c"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Cannot arrange revset with gaps in.
    Hint: Revision 123b4d91f6e5 would need to be in the set.
    [EOF]
    [exit status: 1]
    ");
}
