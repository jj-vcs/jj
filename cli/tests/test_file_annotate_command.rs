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

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use crate::common::TestEnvironment;

fn append_to_file(file_path: &Path, contents: &str) {
    let mut options = OpenOptions::new();
    options.append(true);
    let mut file = options.open(file_path).unwrap();
    writeln!(file, "{contents}").unwrap();
}

#[test]
fn test_annotate_linear() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file.txt"), "line1\n").unwrap();
    test_env
        .run_jj_in(
            &repo_path,
            ["describe", "-m=initial", "--author=Foo <foo@example.org>"],
        )
        .success();

    test_env.run_jj_in(&repo_path, ["new", "-m=next"]).success();
    append_to_file(&repo_path.join("file.txt"), "new text from new commit");

    let output = test_env.run_jj_in(&repo_path, ["file", "annotate", "file.txt"]);
    insta::assert_snapshot!(output, @r"
    qpvuntsm foo      2001-02-03 08:05:08    1: line1
    kkmpptxz test.use 2001-02-03 08:05:10    2: new text from new commit
    [EOF]
    ");
}

#[test]
fn test_annotate_merge() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file.txt"), "line1\n").unwrap();
    test_env
        .run_jj_in(&repo_path, ["describe", "-m=initial"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["branch", "create", "-r@", "initial"])
        .success();

    test_env
        .run_jj_in(&repo_path, ["new", "-m=commit1"])
        .success();
    append_to_file(&repo_path.join("file.txt"), "new text from new commit 1");
    test_env
        .run_jj_in(&repo_path, ["branch", "create", "-r@", "commit1"])
        .success();

    test_env
        .run_jj_in(&repo_path, ["new", "-m=commit2", "initial"])
        .success();
    append_to_file(&repo_path.join("file.txt"), "new text from new commit 2");
    test_env
        .run_jj_in(&repo_path, ["branch", "create", "-r@", "commit2"])
        .success();

    // create a (conflicted) merge
    test_env
        .run_jj_in(&repo_path, ["new", "-m=merged", "commit1", "commit2"])
        .success();
    // resolve conflicts
    std::fs::write(
        repo_path.join("file.txt"),
        "line1\nnew text from new commit 1\nnew text from new commit 2\n",
    )
    .unwrap();

    let output = test_env.run_jj_in(&repo_path, ["file", "annotate", "file.txt"]);
    insta::assert_snapshot!(output, @r"
    qpvuntsm test.use 2001-02-03 08:05:08    1: line1
    zsuskuln test.use 2001-02-03 08:05:11    2: new text from new commit 1
    royxmykx test.use 2001-02-03 08:05:13    3: new text from new commit 2
    [EOF]
    ");
}

#[test]
fn test_annotate_conflicted() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file.txt"), "line1\n").unwrap();
    test_env
        .run_jj_in(&repo_path, ["describe", "-m=initial"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["branch", "create", "-r@", "initial"])
        .success();

    test_env
        .run_jj_in(&repo_path, ["new", "-m=commit1"])
        .success();
    append_to_file(&repo_path.join("file.txt"), "new text from new commit 1");
    test_env
        .run_jj_in(&repo_path, ["branch", "create", "-r@", "commit1"])
        .success();

    test_env
        .run_jj_in(&repo_path, ["new", "-m=commit2", "initial"])
        .success();
    append_to_file(&repo_path.join("file.txt"), "new text from new commit 2");
    test_env
        .run_jj_in(&repo_path, ["branch", "create", "-r@", "commit2"])
        .success();

    // create a (conflicted) merge
    test_env
        .run_jj_in(&repo_path, ["new", "-m=merged", "commit1", "commit2"])
        .success();
    test_env.run_jj_in(&repo_path, ["new"]).success();

    let output = test_env.run_jj_in(&repo_path, ["file", "annotate", "file.txt"]);
    insta::assert_snapshot!(output, @r"
    qpvuntsm test.use 2001-02-03 08:05:08    1: line1
    yostqsxw test.use 2001-02-03 08:05:15    2: <<<<<<< Conflict 1 of 1
    yostqsxw test.use 2001-02-03 08:05:15    3: %%%%%%% Changes from base to side #1
    yostqsxw test.use 2001-02-03 08:05:15    4: +new text from new commit 1
    yostqsxw test.use 2001-02-03 08:05:15    5: +++++++ Contents of side #2
    royxmykx test.use 2001-02-03 08:05:13    6: new text from new commit 2
    yostqsxw test.use 2001-02-03 08:05:15    7: >>>>>>> Conflict 1 of 1 ends
    [EOF]
    ");
}

#[test]
fn test_annotate_merge_one_sided_conflict_resolution() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file.txt"), "line1\n").unwrap();
    test_env
        .run_jj_in(&repo_path, ["describe", "-m=initial"])
        .success();
    test_env
        .run_jj_in(&repo_path, ["branch", "create", "-r@", "initial"])
        .success();

    test_env
        .run_jj_in(&repo_path, ["new", "-m=commit1"])
        .success();
    append_to_file(&repo_path.join("file.txt"), "new text from new commit 1");
    test_env
        .run_jj_in(&repo_path, ["branch", "create", "-r@", "commit1"])
        .success();

    test_env
        .run_jj_in(&repo_path, ["new", "-m=commit2", "initial"])
        .success();
    append_to_file(&repo_path.join("file.txt"), "new text from new commit 2");
    test_env
        .run_jj_in(&repo_path, ["branch", "create", "-r@", "commit2"])
        .success();

    // create a (conflicted) merge
    test_env
        .run_jj_in(&repo_path, ["new", "-m=merged", "commit1", "commit2"])
        .success();
    // resolve conflicts
    std::fs::write(
        repo_path.join("file.txt"),
        "line1\nnew text from new commit 1\n",
    )
    .unwrap();

    let output = test_env.run_jj_in(&repo_path, ["file", "annotate", "file.txt"]);
    insta::assert_snapshot!(output, @r"
    qpvuntsm test.use 2001-02-03 08:05:08    1: line1
    zsuskuln test.use 2001-02-03 08:05:11    2: new text from new commit 1
    [EOF]
    ");
}

#[test]
fn test_annotate_with_template() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let repo_path = test_env.env_root().join("repo");

    std::fs::write(repo_path.join("file.txt"), "line1\n").unwrap();
    test_env
        .run_jj_in(&repo_path, ["commit", "-m=initial"])
        .success();

    append_to_file(
        &repo_path.join("file.txt"),
        "new text from new commit 1\nthat splits into multiple lines",
    );
    test_env
        .run_jj_in(&repo_path, ["commit", "-m=commit1"])
        .success();

    append_to_file(
        &repo_path.join("file.txt"),
        "new text from new commit 2\nalso continuing on a second line\nand a third!",
    );
    test_env
        .run_jj_in(&repo_path, ["describe", "-m=commit2"])
        .success();

    let template = indoc::indoc! {r#"
    if(first_line_in_hunk, "\n" ++ separate("\n",
        commit.change_id().shortest(8)
            ++ " "
            ++ commit.description().first_line(),
        commit_timestamp(commit).local().format('%Y-%m-%d %H:%M:%S')
            ++ " "
            ++ commit.author(),
    ) ++ "\n") ++ pad_start(4, line_number) ++ ": " ++ content
    "#};

    let output = test_env.run_jj_in(&repo_path, ["file", "annotate", "file.txt", "-T", template]);
    insta::assert_snapshot!(output, @r"
    qpvuntsm initial
    2001-02-03 08:05:08 Test User <test.user@example.com>
       1: line1

    rlvkpnrz commit1
    2001-02-03 08:05:09 Test User <test.user@example.com>
       2: new text from new commit 1
       3: that splits into multiple lines

    kkmpptxz commit2
    2001-02-03 08:05:10 Test User <test.user@example.com>
       4: new text from new commit 2
       5: also continuing on a second line
       6: and a third!
    [EOF]
    ");
}
