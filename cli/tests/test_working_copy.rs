// Copyright 2023 The Jujutsu Authors
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

use indoc::indoc;
use regex::Regex;

use crate::common::TestEnvironment;

#[test]
fn test_snapshot_large_file() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // test a small file using raw-integer-literal syntax, which is interpreted
    // in bytes
    test_env.add_config(r#"snapshot.max-new-file-size = 10"#);
    work_dir.write_file("empty", "");
    work_dir.write_file("large", "a lot of text");
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @"
    empty
    [EOF]
    ------- stderr -------
    Warning: Refused to snapshot some files:
      large: 13.0B (13 bytes); the maximum size allowed is 10.0B (10 bytes)
    Hint: This is to prevent large files from being added by accident. To fix this:
      * Add the file(s) to `.gitignore`
      * Run `jj config set --repo snapshot.max-new-file-size 13`
        This will increase the maximum file size allowed for new files, in this repository only.
      * Run `jj --config snapshot.max-new-file-size=13 status`
        This will increase the maximum file size allowed for new files, for this command only.
    [EOF]
    ");

    // test with a larger file using 'KB' human-readable syntax
    test_env.add_config(r#"snapshot.max-new-file-size = "10KB""#);
    let big_string = vec![0; 1024 * 11];
    work_dir.write_file("large", &big_string);
    let output = work_dir.run_jj(["file", "list"]);
    insta::assert_snapshot!(output, @"
    empty
    [EOF]
    ------- stderr -------
    Warning: Refused to snapshot some files:
      large: 11.0KiB (11264 bytes); the maximum size allowed is 10.0KiB (10240 bytes)
    Hint: This is to prevent large files from being added by accident. To fix this:
      * Add the file(s) to `.gitignore`
      * Run `jj config set --repo snapshot.max-new-file-size 11264`
        This will increase the maximum file size allowed for new files, in this repository only.
      * Run `jj --config snapshot.max-new-file-size=11264 status`
        This will increase the maximum file size allowed for new files, for this command only.
    [EOF]
    ");

    // test with file track for hint formatting, both files should appear in
    // warnings even though they were snapshotted separately
    work_dir.write_file("large 2", big_string);
    let output = work_dir.run_jj([
        "file",
        "--config=snapshot.auto-track='large'",
        "track",
        "large 2",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Refused to snapshot some files:
      large: 11.0KiB (11264 bytes); the maximum size allowed is 10.0KiB (10240 bytes)
      large 2: 11.0KiB (11264 bytes); the maximum size allowed is 10.0KiB (10240 bytes)
    Hint: This is to prevent large files from being added by accident. To fix this:
      * Add the file(s) to `.gitignore`
      * Run `jj config set --repo snapshot.max-new-file-size 11264`
        This will increase the maximum file size allowed for new files, in this repository only.
      * Run `jj --config snapshot.max-new-file-size=11264 file track large 'large 2'`
        This will increase the maximum file size allowed for new files, for this command only.
      * Run `jj file track --include-ignored large 'large 2'`
        This will track the file(s) regardless of size.
    [EOF]
    ");

    // test invalid configuration
    let output = work_dir.run_jj(["file", "list", "--config=snapshot.max-new-file-size=[]"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Config error: Invalid type or value for snapshot.max-new-file-size
    Caused by: Expected a positive integer or a string in '<number><unit>' form
    For help, see https://docs.jj-vcs.dev/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    ");

    // No error if we disable auto-tracking of the path
    let output = work_dir.run_jj(["file", "list", "--config=snapshot.auto-track='none()'"]);
    insta::assert_snapshot!(output, @"
    empty
    [EOF]
    ");

    // max-new-file-size=0 means no limit
    let output = work_dir.run_jj(["file", "list", "--config=snapshot.max-new-file-size=0"]);
    insta::assert_snapshot!(output, @"
    empty
    large
    large 2
    [EOF]
    ");
}

#[test]
fn test_snapshot_large_file_restore() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    test_env.add_config("snapshot.max-new-file-size = 10");

    work_dir.run_jj(["describe", "-mcommitted"]).success();
    work_dir.write_file("file", "small");

    // Write a large file in the working copy, restore it from a commit. The
    // working-copy content shouldn't be overwritten.
    work_dir.run_jj(["new", "root()"]).success();
    work_dir.write_file("file", "a lot of text");
    let output = work_dir.run_jj(["restore", "--from=subject(committed)"]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Warning: Refused to snapshot some files:
      file: 13.0B (13 bytes); the maximum size allowed is 10.0B (10 bytes)
    Hint: This is to prevent large files from being added by accident. To fix this:
      * Add the file(s) to `.gitignore`
      * Run `jj config set --repo snapshot.max-new-file-size 13`
        This will increase the maximum file size allowed for new files, in this repository only.
      * Run `jj --config snapshot.max-new-file-size=13 status`
        This will increase the maximum file size allowed for new files, for this command only.
    Working copy  (@) now at: kkmpptxz 119f5156 (no description set)
    Parent commit (@-)      : zzzzzzzz 00000000 (empty) (no description set)
    Added 1 files, modified 0 files, removed 0 files
    Warning: 1 of those updates were skipped because there were conflicting changes in the working copy.
    Hint: Inspect the changes compared to the intended target with `jj diff --from 119f5156d330`.
    Discard the conflicting changes with `jj restore --from 119f5156d330`.
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.read_file("file"), @"a lot of text");

    // However, the next command will snapshot the large file because it is now
    // tracked. TODO: Should we remember the untracked state?
    let output = work_dir.run_jj(["status"]);
    insta::assert_snapshot!(output, @"
    Working copy changes:
    A file
    Working copy  (@) : kkmpptxz 09eba65e (no description set)
    Parent commit (@-): zzzzzzzz 00000000 (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_materialize_and_snapshot_different_conflict_markers() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Configure to use Git-style conflict markers
    test_env.add_config(r#"ui.conflict-marker-style = "git""#);

    // Create a conflict in the working copy
    work_dir.write_file(
        "file",
        indoc! {"
            line 1
            line 2
            line 3
        "},
    );
    work_dir.run_jj(["commit", "-m", "base"]).success();
    work_dir.write_file(
        "file",
        indoc! {"
            line 1
            line 2 - a
            line 3
        "},
    );
    work_dir.run_jj(["commit", "-m", "side-a"]).success();
    work_dir
        .run_jj(["new", "subject(base)", "-m", "side-b"])
        .success();
    work_dir.write_file(
        "file",
        indoc! {"
            line 1
            line 2 - b
            line 3 - b
        "},
    );
    work_dir
        .run_jj(["new", "subject(side-a)", "subject(side-b)"])
        .success();

    // File should have Git-style conflict markers
    insta::assert_snapshot!(work_dir.read_file("file"), @r#"
    line 1
    <<<<<<< rlvkpnrz df1cdd77 "side-a"
    line 2 - a
    line 3
    ||||||| qpvuntsm 2205b3ac "base"
    line 2
    line 3
    =======
    line 2 - b
    line 3 - b
    >>>>>>> zsuskuln 68dcce1b "side-b"
    "#);

    // Configure to use JJ-style "snapshot" conflict markers
    test_env.add_config(r#"ui.conflict-marker-style = "snapshot""#);

    // Update the conflict, still using Git-style conflict markers
    work_dir.write_file(
        "file",
        indoc! {"
            line 1
            <<<<<<<
            line 2 - a
            line 3 - a
            |||||||
            line 2
            line 3
            =======
            line 2 - b
            line 3 - b
            >>>>>>>
        "},
    );

    // Git-style markers should be parsed, then rendered with new config
    insta::assert_snapshot!(work_dir.run_jj(["diff", "--git"]), @r#"
    diff --git a/file b/file
    --- a/file
    +++ b/file
    @@ -2,7 +2,7 @@
     <<<<<<< conflict 1 of 1
     +++++++ rlvkpnrz df1cdd77 "side-a"
     line 2 - a
    -line 3
    +line 3 - a
     ------- qpvuntsm 2205b3ac "base"
     line 2
     line 3
    [EOF]
    "#);

    // The timestamps in the `jj debug local-working-copy` output change, so we want
    // to remove them before asserting the snapshot
    let timestamp_regex = Regex::new(r"\b\d{10,}\b").unwrap();
    let redact_output = |output: String| {
        let output = timestamp_regex.replace_all(&output, "<timestamp>");
        output.into_owned()
    };

    // Working copy should contain conflict marker length
    let output = work_dir.run_jj(["debug", "local-working-copy"]);
    insta::assert_snapshot!(output.normalize_stdout_with(redact_output), @r#"
    Current operation: OperationId("e42a404c4cdfefa27ad284fd04699a4155e7e6f49174fac6beb38f6aae63029d9ead29a045697c985ab98bb046959d80896dc3dc7af552da358747a6517ae5cc")
    Current tree: MergedTree { tree_ids: Conflicted([TreeId("ba2e5292905a6bf094ae5993d969cc0c342064a1"), TreeId("771f3d31c4588ea40a8864b2a981749888e596c2"), TreeId("2b4adac1dae8f6b5f486b7ceb45f6fdbfbefd780")]), labels: Labeled(["rlvkpnrz df1cdd77 \"side-a\"", "qpvuntsm 2205b3ac \"base\"", "zsuskuln 68dcce1b \"side-b\""]), .. }
    Normal { exec_bit: ExecBit(false) }            97 <timestamp> Some(MaterializedConflictData { conflict_marker_len: 7 }) "file"
    [EOF]
    "#);

    // Update the conflict with more fake markers, and it should still parse
    // correctly (the markers should be ignored)
    work_dir.write_file(
        "file",
        indoc! {"
            line 1
            <<<<    
        "},
    );

    // The file should still be conflicted, and the new content should be saved
    let output = work_dir.run_jj(["st"]);
    insta::assert_snapshot!(output, @"
    Working copy changes:
    M file
    Working copy  (@) : mzvwutvl 7e64cd32 (no description set)
    Parent commit (@-): rlvkpnrz df1cdd77 side-a
    Parent commit (@-): zsuskuln 68dcce1b side-b
    [EOF]
    ");
    insta::assert_snapshot!(work_dir.run_jj(["diff", "--git"]), @r#"
    diff --git a/file b/file
    index 0000000000..167e1cd6f6 100644
    --- a/file
    +++ b/file
    @@ -1,12 +1,2 @@
     line 1
    -<<<<<<< conflict 1 of 1
    -+++++++ rlvkpnrz df1cdd77 "side-a"
    -line 2 - a
    -line 3
    -------- qpvuntsm 2205b3ac "base"
    -line 2
    -line 3
    -+++++++ zsuskuln 68dcce1b "side-b"
    -line 2 - b
    -line 3 - b
    ->>>>>>> conflict 1 of 1 ends
    +<<<<    
    [EOF]
    "#);

    // Working copy should still contain conflict marker length
    let output = work_dir.run_jj(["debug", "local-working-copy"]);
    insta::assert_snapshot!(output.normalize_stdout_with(redact_output), @r#"
    Current operation: OperationId("0e3726a0d11dbcf6a56e87540689f30fd8c2d1028eb14b22d2173207040b8e776b2854466d46cb4afb35ff0128f14f99d199a4b92f923398227280f4181a0f3f")
    Current tree: MergedTree { tree_ids: Resolved(TreeId("29a7d1038a251433333cf50f920ab512fbc3a7b8")), labels: Unlabeled, .. }
    Normal { exec_bit: ExecBit(false) }            16 <timestamp> None "file"
    [EOF]
    "#);

    // Resolve the conflict
    work_dir.write_file(
        "file",
        indoc! {"
            line 1
            <<<<<<< fake marker
            ||||||| fake marker
            line 2 - left
            line 2 - right
            ======= fake marker
            line 3 - left
            >>>>>>> fake marker
        "},
    );

    let output = work_dir.run_jj(["st"]);
    insta::assert_snapshot!(output, @"
    Working copy changes:
    M file
    Working copy  (@) : mzvwutvl 891cdb11 (no description set)
    Parent commit (@-): rlvkpnrz df1cdd77 side-a
    Parent commit (@-): zsuskuln 68dcce1b side-b
    [EOF]
    ");

    // When the file is resolved, the conflict marker length is removed from the
    // working copy
    let output = work_dir.run_jj(["debug", "local-working-copy"]);
    insta::assert_snapshot!(output.normalize_stdout_with(redact_output), @r#"
    Current operation: OperationId("b5a77a212b159132cf83d7d9c077cf0b58d3ffd67e0addd17d6cb545dc54e399450a86ea37bbe80539a5d7c1d994b0bce9333a674d1f1c1cb97b34ad12817fb7")
    Current tree: MergedTree { tree_ids: Resolved(TreeId("6120567b3cb2472d549753ed3e4b84183d52a650")), labels: Unlabeled, .. }
    Normal { exec_bit: ExecBit(false) }           130 <timestamp> None "file"
    [EOF]
    "#);
}

#[test]
fn test_submodule_ignored() {
    let test_env = TestEnvironment::default();

    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "submodule"])
        .success();
    let submodule_dir = test_env.work_dir("submodule");
    submodule_dir.write_file("sub", "sub");
    submodule_dir
        .run_jj(["commit", "-m", "Submodule commit"])
        .success();

    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "repo"])
        .success();
    let work_dir = test_env.work_dir("repo");

    // There's no particular reason to run this with jj util exec, it's just that
    // the infra makes it easier to run this way.
    let output = work_dir.run_jj([
        "util",
        "exec",
        "--",
        "git",
        "-c",
        // Git normally doesn't allow file:// in submodules.
        "protocol.file.allow=always",
        "submodule",
        "add",
        &format!("{}/submodule", test_env.env_root().display()),
        "sub",
    ]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    Cloning into '$TEST_ENV/repo/sub'...
    done.
    [EOF]
    ");
    // Use git to commit since jj won't play nice with the submodule.
    work_dir
        .run_jj([
            "util",
            "exec",
            "--",
            "git",
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=Test user",
            "commit",
            "-m",
            "Add submodule",
        ])
        .success();

    // This should be empty. We shouldn't track the submodule itself.
    let output = work_dir.run_jj(["diff", "--summary"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    ignoring git submodule at "sub"
    Done importing changes from the underlying Git repo.
    [EOF]
    "#);

    // Switch to a historical commit before the submodule was checked in.
    work_dir.run_jj(["prev"]).success();
    // jj new (or equivalently prev) should always leave you with an empty working
    // copy.
    let output = work_dir.run_jj(["diff", "--summary"]);
    insta::assert_snapshot!(output, @"");
}

#[test]
fn test_snapshot_jjconflict_trees() {
    let test_env = TestEnvironment::default();
    test_env
        .run_jj_in(".", ["git", "init", "repo", "--colocate"])
        .success();
    let work_dir = test_env.work_dir("repo");

    // Create a conflict in the working copy
    work_dir.write_file(
        "file",
        indoc! {"
            line 1
            line 2
            line 3
        "},
    );
    work_dir.run_jj(["new", "-m", "side-a"]).success();
    work_dir.write_file(
        "file",
        indoc! {"
            line 1
            line 2 - left
            line 3 - left
        "},
    );
    work_dir
        .run_jj(["new", "subject(side-a)-", "-m", "side-b"])
        .success();
    work_dir.write_file(
        "file",
        indoc! {"
            line 1
            line 2 - right
            line 3
        "},
    );
    work_dir.run_jj(["new"]).success();
    work_dir
        .run_jj(["rebase", "-s", "subject(side-b)", "-o", "subject(side-a)"])
        .success();

    // Run `git reset --hard HEAD` to simulate checking out the branch with Git.
    let output = std::process::Command::new("git")
        .current_dir(work_dir.root())
        .args(["reset", "--hard", "HEAD"])
        .output()
        .unwrap();
    assert!(output.status.success());

    // We should see a warning regarding '.jjconflict' trees being checked out.
    let output = work_dir.run_jj(["st"]);
    insta::assert_snapshot!(output.to_string().replace('\\', "/"), @"
    Working copy changes:
    A .jjconflict-base-0/file
    A .jjconflict-side-0/file
    A .jjconflict-side-1/file
    A JJ-CONFLICT-README
    M file
    Working copy  (@) : zsuskuln 2681a418 (no description set)
    Parent commit (@-): kkmpptxz aadeb8eb (conflict) side-b
    Hint: Conflict in parent commit has been resolved in working copy
    [EOF]
    ------- stderr -------
    Warning: The working copy contains '.jjconflict' files. These files are used by `jj` internally and should not be present in the working copy.
    Hint: You may have used a regular `git` command to check out a conflicted commit.
    Hint: You can use `jj abandon` to discard the working copy changes.
    [EOF]
    ");
}
