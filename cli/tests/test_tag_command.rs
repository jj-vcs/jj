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

use testutils::git;

use crate::common::TestEnvironment;

#[test]
fn test_tag_list() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let git_repo = {
        let mut git_repo_path = work_dir.root().to_owned();
        git_repo_path.extend([".jj", "repo", "store", "git"]);
        git::open(git_repo_path)
    };

    let copy_ref = |src_name: &str, tag_name: &str| {
        let src = git_repo.find_reference(src_name).unwrap();
        git_repo
            .tag_reference(
                tag_name,
                src.target().id(),
                gix::refs::transaction::PreviousValue::Any,
            )
            .unwrap();
    };

    work_dir.run_jj(["new", "root()", "-mcommit1"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark1"])
        .success();
    work_dir.run_jj(["new", "root()", "-mcommit2"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark2"])
        .success();
    work_dir.run_jj(["new", "root()", "-mcommit3"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bookmark3"])
        .success();
    work_dir.run_jj(["git", "export"]).success();

    copy_ref("refs/heads/bookmark1", "test_tag");
    copy_ref("refs/heads/bookmark2", "test_tag2");
    copy_ref("refs/heads/bookmark1", "conflicted_tag");
    work_dir.run_jj(["git", "import"]).success();
    copy_ref("refs/heads/bookmark2", "conflicted_tag");
    work_dir.run_jj(["git", "import"]).success();
    copy_ref("refs/heads/bookmark3", "conflicted_tag");
    work_dir.run_jj(["git", "import", "--at-op=@-"]).success();
    work_dir.run_jj(["status"]).success(); // resolve concurrent ops

    insta::assert_snapshot!(work_dir.run_jj(["tag", "list"]), @r"
    conflicted_tag (conflicted):
      - rlvkpnrz 893e67dc (empty) commit1
      + zsuskuln 76abdd20 (empty) commit2
      + royxmykx 13c4e819 (empty) commit3
    test_tag: rlvkpnrz 893e67dc (empty) commit1
    test_tag2: zsuskuln 76abdd20 (empty) commit2
    [EOF]
    ");

    insta::assert_snapshot!(work_dir.run_jj(["tag", "list", "--color=always"]), @r"
    [38;5;5mconflicted_tag[39m [38;5;1m(conflicted)[39m:
      - [1m[38;5;5mrl[0m[38;5;8mvkpnrz[39m [1m[38;5;4m8[0m[38;5;8m93e67dc[39m [38;5;2m(empty)[39m commit1
      + [1m[38;5;5mzs[0m[38;5;8muskuln[39m [1m[38;5;4m7[0m[38;5;8m6abdd20[39m [38;5;2m(empty)[39m commit2
      + [1m[38;5;5mr[0m[38;5;8moyxmykx[39m [1m[38;5;4m13[0m[38;5;8mc4e819[39m [38;5;2m(empty)[39m commit3
    [38;5;5mtest_tag[39m: [1m[38;5;5mrl[0m[38;5;8mvkpnrz[39m [1m[38;5;4m8[0m[38;5;8m93e67dc[39m [38;5;2m(empty)[39m commit1
    [38;5;5mtest_tag2[39m: [1m[38;5;5mzs[0m[38;5;8muskuln[39m [1m[38;5;4m7[0m[38;5;8m6abdd20[39m [38;5;2m(empty)[39m commit2
    [EOF]
    ");

    // Test pattern matching.
    insta::assert_snapshot!(work_dir.run_jj(["tag", "list", "test_tag2"]), @r"
    test_tag2: zsuskuln 76abdd20 (empty) commit2
    [EOF]
    ");

    insta::assert_snapshot!(work_dir.run_jj(["tag", "list", "glob:test_tag?"]), @r"
    test_tag2: zsuskuln 76abdd20 (empty) commit2
    [EOF]
    ");

    let template = r#"
    concat(
      "[" ++ name ++ "]\n",
      separate(" ", "present:", present) ++ "\n",
      separate(" ", "conflict:", conflict) ++ "\n",
      separate(" ", "normal_target:", normal_target.description().first_line()) ++ "\n",
      separate(" ", "removed_targets:", removed_targets.map(|c| c.description().first_line())) ++ "\n",
      separate(" ", "added_targets:", added_targets.map(|c| c.description().first_line())) ++ "\n",
    )
    "#;
    insta::assert_snapshot!(work_dir.run_jj(["tag", "list", "-T", template]), @r"
    [conflicted_tag]
    present: true
    conflict: true
    normal_target: <Error: No Commit available>
    removed_targets: commit1
    added_targets: commit2 commit3
    [test_tag]
    present: true
    conflict: false
    normal_target: commit1
    removed_targets:
    added_targets: commit1
    [test_tag2]
    present: true
    conflict: false
    normal_target: commit2
    removed_targets:
    added_targets: commit2
    [EOF]
    ");
}
