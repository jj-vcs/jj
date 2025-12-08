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

use std::path::PathBuf;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[test]
fn test_commit_with_description_from_cli() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Description applies to the current working-copy (not the new one)
    work_dir.run_jj(["commit", "-m=first"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  eb9fd2ab82e7
    ○  68a505386f93 first
    ◆  000000000000
    [EOF]
    ");
}

#[test]
fn test_commit_with_editor() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Check that the text file gets initialized with the current description and
    // set a new one
    work_dir.run_jj(["describe", "-m=initial"]).success();
    std::fs::write(&edit_script, ["dump editor0", "write\nmodified"].join("\0")).unwrap();
    work_dir.run_jj(["commit"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  2094c8f2e360
    ○  a7ba1eb73836 modified
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor0")).unwrap(), @r#"
    initial

    JJ: Change ID: qpvuntsm
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);

    // Check that the editor content includes diff summary
    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "foo\n");
    work_dir.run_jj(["describe", "-m=add files"]).success();
    std::fs::write(&edit_script, "dump editor1").unwrap();
    work_dir.run_jj(["commit"]).success();
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor1")).unwrap(), @r#"
    add files

    JJ: Change ID: kkmpptxz
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:     A file2
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
}

#[test]
fn test_commit_with_editor_avoids_unc() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    std::fs::write(edit_script, "dump-path path").unwrap();
    work_dir.run_jj(["commit"]).success();

    let edited_path =
        PathBuf::from(std::fs::read_to_string(test_env.env_root().join("path")).unwrap());
    // While `assert!(!edited_path.starts_with("//?/"))` could work here in most
    // cases, it fails when it is not safe to strip the prefix, such as paths
    // over 260 chars.
    assert_eq!(edited_path, dunce::simplified(&edited_path));
}

#[test]
fn test_commit_with_empty_description_from_cli() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["commit", "-m="]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  51b556e22ca0
    ○  cc8ff2284a8c
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz 51b556e2 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm cc8ff228 (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_commit_with_empty_description_from_editor() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Check that the text file gets initialized and leave it untouched.
    std::fs::write(&edit_script, ["dump editor0"].join("\0")).unwrap();
    let output = work_dir.run_jj(["commit"]).success();
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  51b556e22ca0
    ○  cc8ff2284a8c
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor0")).unwrap(),
        @r#"


    JJ: Change ID: qpvuntsm
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Hint: The commit message was left empty.
    If this was not intentional, run `jj undo` to restore the previous state.
    Or run `jj desc @-` to add a description to the parent commit.
    Working copy  (@) now at: rlvkpnrz 51b556e2 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm cc8ff228 (empty) (no description set)
    [EOF]
    ");
}

#[test]
fn test_commit_interactive() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    let diff_editor = test_env.set_up_fake_diff_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "bar\n");
    work_dir.run_jj(["describe", "-m=add files"]).success();
    std::fs::write(edit_script, ["dump editor"].join("\0")).unwrap();

    let diff_script = ["rm file2", "dump JJ-INSTRUCTIONS instrs"].join("\0");
    std::fs::write(diff_editor, diff_script).unwrap();
    let setup_opid = work_dir.current_operation_id();

    // Create a commit interactively and select only file1
    work_dir.run_jj(["commit", "-i"]).success();

    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("instrs")).unwrap(), @r"
    You are splitting the working-copy commit: qpvuntsm d849dc34 add files

    The diff initially shows all changes. Adjust the right side until it shows the
    contents you want for the first commit. The remainder will be included in the
    new working-copy commit.
    ");

    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor")).unwrap(), @r#"
    add files

    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);

    // Try again with --tool=<name>, which implies --interactive
    work_dir.run_jj(["op", "restore", &setup_opid]).success();
    work_dir
        .run_jj([
            "commit",
            "--config=ui.diff-editor='false'",
            "--tool=fake-diff-editor",
        ])
        .success();

    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor")).unwrap(), @r#"
    add files

    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);

    let output = work_dir.run_jj(["log", "--summary"]);
    insta::assert_snapshot!(output, @r"
    @  mzvwutvl test.user@example.com 2001-02-03 08:05:11 9b0176ab
    │  (no description set)
    │  A file2
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:11 6e6fa925
    │  add files
    │  A file1
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
}

#[test]
fn test_commit_interactive_with_paths() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    let diff_editor = test_env.set_up_fake_diff_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file2", "");
    work_dir.write_file("file3", "");
    work_dir.run_jj(["new", "-medit"]).success();
    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "bar\n");
    work_dir.write_file("file3", "baz\n");

    std::fs::write(edit_script, ["dump editor"].join("\0")).unwrap();
    let diff_script = [
        "files-before file2",
        "files-after JJ-INSTRUCTIONS file1 file2",
        "reset file2",
    ]
    .join("\0");
    std::fs::write(diff_editor, diff_script).unwrap();

    // Select file1 and file2 by args, then select file1 interactively
    let output = work_dir.run_jj(["commit", "-i", "file1", "file2"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: kkmpptxz 50f426df (no description set)
    Parent commit (@-)      : rlvkpnrz eb640375 edit
    [EOF]
    ");

    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor")).unwrap(), @r#"
    edit

    JJ: Change ID: rlvkpnrz
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);

    let output = work_dir.run_jj(["log", "--summary"]);
    insta::assert_snapshot!(output, @r"
    @  kkmpptxz test.user@example.com 2001-02-03 08:05:09 50f426df
    │  (no description set)
    │  M file2
    │  M file3
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 eb640375
    │  edit
    │  A file1
    ○  qpvuntsm test.user@example.com 2001-02-03 08:05:08 ff687a2f
    │  (no description set)
    │  A file2
    │  A file3
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");
}

#[test]
fn test_commit_with_default_description() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    test_env.add_config(r#"ui.default-description = "\n\nTESTED=TODO""#);
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "bar\n");
    std::fs::write(edit_script, ["dump editor"].join("\0")).unwrap();
    work_dir.run_jj(["commit"]).success();

    insta::assert_snapshot!(get_log_output(&work_dir), @r#"
    @  cba559ac1a48
    ○  7276dfff8027 TESTED=TODO
    ◆  000000000000
    [EOF]
    ------- stderr -------
    Warning: Deprecated user-level config: ui.default-description is updated to template-aliases.default_commit_description = '"\n\nTESTED=TODO\n"'
    [EOF]
    "#);
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor")).unwrap(), @r#"


    TESTED=TODO

    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:     A file2
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
}

#[test]
fn test_commit_with_description_template() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    test_env.add_config(
        r#"
        [templates]
        draft_commit_description = '''
        concat(
          description,
          "\n",
          indent(
            "JJ: ",
            concat(
              "Author: " ++ format_detailed_signature(author) ++ "\n",
              "Committer: " ++ format_detailed_signature(committer)  ++ "\n",
              "\n",
              diff.stat(76),
            ),
          ),
        )
        '''
        "#,
    );
    let work_dir = test_env.work_dir("repo");

    std::fs::write(edit_script, ["dump editor"].join("\0")).unwrap();

    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "bar\n");
    work_dir.write_file("file3", "foobar\n");

    // Only file1 should be included in the diff
    work_dir.run_jj(["commit", "file1"]).success();
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor")).unwrap(), @r#"

    JJ: Author: Test User <test.user@example.com> (2001-02-03 08:05:08)
    JJ: Committer: Test User <test.user@example.com> (2001-02-03 08:05:08)

    JJ: file1 | 1 +
    JJ: 1 file changed, 1 insertion(+), 0 deletions(-)
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);

    // Only file2 with modified author should be included in the diff
    work_dir
        .run_jj([
            "commit",
            "--author",
            "Another User <another.user@example.com>",
            "file2",
        ])
        .success();
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor")).unwrap(), @r#"

    JJ: Author: Another User <another.user@example.com> (2001-02-03 08:05:08)
    JJ: Committer: Test User <test.user@example.com> (2001-02-03 08:05:09)

    JJ: file2 | 1 +
    JJ: 1 file changed, 1 insertion(+), 0 deletions(-)
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);

    // Timestamp after the reset should be available to the template
    work_dir.run_jj(["commit", "--reset-author"]).success();
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor")).unwrap(), @r#"

    JJ: Author: Test User <test.user@example.com> (2001-02-03 08:05:10)
    JJ: Committer: Test User <test.user@example.com> (2001-02-03 08:05:10)

    JJ: file3 | 1 +
    JJ: 1 file changed, 1 insertion(+), 0 deletions(-)
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
}

#[test]
fn test_commit_without_working_copy() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["workspace", "forget"]).success();
    let output = work_dir.run_jj(["commit", "-m=first"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: This command requires a working copy
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_commit_paths() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "bar\n");

    work_dir.run_jj(["commit", "-m=first", "file1"]).success();
    let output = work_dir.run_jj(["diff", "-r", "@-"]);
    insta::assert_snapshot!(output, @r"
    Added regular file file1:
            1: foo
    [EOF]
    ");

    let output = work_dir.run_jj(["diff"]);
    insta::assert_snapshot!(output, @r"
    Added regular file file2:
            1: bar
    [EOF]
    ");
}

#[test]
fn test_commit_paths_warning() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    work_dir.write_file("file2", "bar\n");

    let output = work_dir.run_jj(["commit", "-m=first", "file3"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Warning: The given paths do not match any file: file3
    Working copy  (@) now at: rlvkpnrz 4c6f0146 (no description set)
    Parent commit (@-)      : qpvuntsm 68a50538 (empty) first
    [EOF]
    ");

    let output = work_dir.run_jj(["diff"]);
    insta::assert_snapshot!(output, @r"
    Added regular file file1:
            1: foo
    Added regular file file2:
            1: bar
    [EOF]
    ");
}

#[test]
fn test_commit_reset_author() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    test_env.add_config(
        r#"[template-aliases]
'format_signature(signature)' = 'signature.name() ++ " " ++ signature.email() ++ " " ++ signature.timestamp()'"#,
    );
    let get_signatures = || {
        let template = r#"format_signature(author) ++ "\n" ++ format_signature(committer)"#;
        work_dir.run_jj(["log", "-r@", "-T", template])
    };
    insta::assert_snapshot!(get_signatures(), @r"
    @  Test User test.user@example.com 2001-02-03 04:05:07.000 +07:00
    │  Test User test.user@example.com 2001-02-03 04:05:07.000 +07:00
    ~
    [EOF]
    ");

    // Reset the author (the committer is always reset)
    work_dir
        .run_jj([
            "commit",
            "--config=user.name=Ove Ridder",
            "--config=user.email=ove.ridder@example.com",
            "--reset-author",
            "-m1",
        ])
        .success();
    insta::assert_snapshot!(get_signatures(), @r"
    @  Ove Ridder ove.ridder@example.com 2001-02-03 04:05:09.000 +07:00
    │  Ove Ridder ove.ridder@example.com 2001-02-03 04:05:09.000 +07:00
    ~
    [EOF]
    ");
}

#[test]
fn test_commit_trailers() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    test_env.add_config(
        r#"[templates]
        commit_trailers = '''"Reviewed-by: " ++ self.committer().email()'''"#,
    );
    work_dir.write_file("file1", "foo\n");

    let output = work_dir.run_jj(["commit", "-m=first"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: rlvkpnrz 0c0495f3 (empty) (no description set)
    Parent commit (@-)      : qpvuntsm ae86ffd4 first
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "--no-graph", "-r@-", "-Tdescription"]);
    insta::assert_snapshot!(output, @r"
    first

    Reviewed-by: test.user@example.com
    [EOF]
    ");

    // the new committer should appear in the trailer
    let output = work_dir.run_jj(["commit", "--config=user.email=foo@bar.org", "-m=second"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: zsuskuln fd73eac2 (empty) (no description set)
    Parent commit (@-)      : rlvkpnrz 6e69e833 (empty) second
    [EOF]
    ");

    let output = work_dir.run_jj(["log", "--no-graph", "-r@-", "-Tdescription"]);
    insta::assert_snapshot!(output, @r"
    second

    Reviewed-by: foo@bar.org
    [EOF]
    ");

    // the trailer is added in the editor
    std::fs::write(&edit_script, "dump editor0").unwrap();
    let output = work_dir.run_jj(["commit", "--config=user.email=foo@bar.org"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Working copy  (@) now at: royxmykx dac9709c (empty) (no description set)
    Parent commit (@-)      : zsuskuln d9ced309 (empty) Reviewed-by: foo@bar.org
    [EOF]
    ");

    let editor0 = std::fs::read_to_string(test_env.env_root().join("editor0")).unwrap();
    insta::assert_snapshot!(
        format!("-----\n{editor0}-----\n"), @r#"
    -----


    Reviewed-by: foo@bar.org

    JJ: Change ID: zsuskuln
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    -----
    "#);

    let output = work_dir.run_jj(["log", "--no-graph", "-r@-", "-Tdescription"]);
    insta::assert_snapshot!(output, @r"
    Reviewed-by: foo@bar.org
    [EOF]
    ");
}

#[test]
fn test_commit_with_editor_and_message_args() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");
    std::fs::write(&edit_script, "dump editor").unwrap();
    work_dir
        .run_jj(["commit", "-m", "message from command line", "--editor"])
        .success();

    // Verify editor was opened with the message from command line
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor")).unwrap(), @r#"
    message from command line

    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  c3a4bb8be8d7
    ○  f6acb1a163f2 message from command line
    ◆  000000000000
    [EOF]
    ");
}

#[test]
fn test_commit_with_editor_and_empty_message() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");

    // Use --editor with an empty message. The trailers should be added because
    // the editor will be opened.
    std::fs::write(&edit_script, "dump editor").unwrap();
    work_dir
        .run_jj([
            "commit",
            "-m",
            "",
            "--editor",
            "--config",
            r#"templates.commit_trailers='"Trailer: value"'"#,
        ])
        .success();

    // Verify editor was opened with trailers added to the empty message
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor")).unwrap(), @r#"


    Trailer: value

    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
}

#[test]
fn test_commit_with_editor_without_message() {
    let mut test_env = TestEnvironment::default();
    let edit_script = test_env.set_up_fake_editor();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.write_file("file1", "foo\n");

    // --editor without -m should behave the same as without --editor (normal flow)
    std::fs::write(&edit_script, "dump editor").unwrap();
    let output = work_dir.run_jj(["commit", "--editor"]).success();

    // Verify editor was opened
    insta::assert_snapshot!(
        std::fs::read_to_string(test_env.env_root().join("editor")).unwrap(), @r#"


    JJ: Change ID: qpvuntsm
    JJ: This commit contains the following changes:
    JJ:     A file1
    JJ:
    JJ: Lines starting with "JJ:" (like this one) will be removed.
    "#);
    insta::assert_snapshot!(get_log_output(&work_dir), @r"
    @  f5a89f0f6366
    ○  38f3e84bb6a9
    ◆  000000000000
    [EOF]
    ");
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Hint: The commit message was left empty.
    If this was not intentional, run `jj undo` to restore the previous state.
    Or run `jj desc @-` to add a description to the parent commit.
    Working copy  (@) now at: rlvkpnrz f5a89f0f (empty) (no description set)
    Parent commit (@-)      : qpvuntsm 38f3e84b (no description set)
    [EOF]
    ");
}

#[must_use]
fn get_log_output(work_dir: &TestWorkDir) -> CommandOutput {
    let template = r#"commit_id.short() ++ " " ++ description"#;
    work_dir.run_jj(["log", "-T", template])
}
