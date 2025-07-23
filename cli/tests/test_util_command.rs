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

use insta::assert_snapshot;

use crate::common::TestEnvironment;

#[test]
fn test_util_config_schema() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["util", "config-schema"]);
    // Validate partial snapshot, redacting any lines nested 2+ indent levels.
    insta::with_settings!({filters => vec![(r"(?m)(^        .*$\r?\n)+", "        [...]\n")]}, {
        assert_snapshot!(output, @r#"
        {
            "$schema": "http://json-schema.org/draft-04/schema",
            "$comment": "`taplo` and the corresponding VS Code plugins only support version draft-04 of JSON Schema, see <https://taplo.tamasfe.dev/configuration/developing-schemas.html>. draft-07 is mostly compatible with it, newer versions may not be.",
            "title": "Jujutsu config",
            "type": "object",
            "description": "User configuration for Jujutsu VCS. See https://jj-vcs.github.io/jj/latest/config/ for details",
            "properties": {
                [...]
            }
        }
        [EOF]
        "#);
    });
}

#[test]
fn test_gc_args() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["util", "gc"]);
    insta::assert_snapshot!(output, @"");

    let output = work_dir.run_jj(["util", "gc", "--at-op=@-"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: Cannot garbage collect from a non-head operation
    [EOF]
    [exit status: 1]
    ");

    let output = work_dir.run_jj(["util", "gc", "--expire=foobar"]);
    insta::assert_snapshot!(output, @r"
    ------- stderr -------
    Error: --expire only accepts 'now'
    [EOF]
    [exit status: 1]
    ");
}

#[test]
fn test_gc_operation_log() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create an operation.
    work_dir.write_file("file", "a change\n");
    work_dir.run_jj(["commit", "-m", "a change"]).success();
    let op_to_remove = work_dir.current_operation_id();

    // Make another operation the head.
    work_dir.write_file("file", "another change\n");
    work_dir
        .run_jj(["commit", "-m", "another change"])
        .success();

    // This works before the operation is removed.
    work_dir
        .run_jj(["debug", "operation", &op_to_remove])
        .success();

    // Remove some operations.
    work_dir.run_jj(["operation", "abandon", "..@-"]).success();
    work_dir.run_jj(["util", "gc", "--expire=now"]).success();

    // Now this doesn't work.
    let output = work_dir.run_jj(["debug", "operation", &op_to_remove]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: No operation ID matching "b50d0a8f111a9d30d45d429d62c8e54016cc7c891706921a6493756c8074e883671cf3dac0ac9f94ef0fa8c79738a3dfe38c3e1f6c5e1a4a4d0857d266ef2040"
    [EOF]
    [exit status: 1]
    "#);
}

#[test]
fn test_gc_rerere_cache() {
    let test_env = TestEnvironment::default();
    test_env.add_config("rerere.enabled = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create a conflict and resolve it to populate the rerere cache
    work_dir.write_file("file.txt", "base\n");
    work_dir.run_jj(["commit", "-m", "base"]).success();

    // Create diverging changes
    work_dir.run_jj(["new", "-m", "a"]).success();
    work_dir.write_file("file.txt", "a\n");
    work_dir.run_jj(["commit", "-m", "commit a"]).success();

    work_dir.run_jj(["new", "-m", "b", "@--"]).success();
    work_dir.write_file("file.txt", "b\n");
    work_dir.run_jj(["commit", "-m", "commit b"]).success();

    // Create a merge with conflicts
    work_dir
        .run_jj([
            "new",
            "-m",
            "merge1",
            "description(\"commit a\")",
            "description(\"commit b\")",
        ])
        .success();

    // Resolve the conflict
    work_dir.write_file("file.txt", "resolved\n");
    work_dir
        .run_jj(["commit", "-m", "resolved merge1"])
        .success();

    // Check that resolution cache exists
    let cache_dir = work_dir.root().join(".jj/repo/resolution_cache");
    assert!(cache_dir.exists());
    let entries: Vec<_> = std::fs::read_dir(&cache_dir).unwrap().collect();
    assert_eq!(entries.len(), 1);

    // Run GC with default expiration (14 days) - should keep the resolution
    work_dir.run_jj(["util", "gc"]).success();
    let entries: Vec<_> = std::fs::read_dir(&cache_dir).unwrap().collect();
    assert_eq!(entries.len(), 1, "Recent resolution should be kept");

    // Run GC with --expire=now - should remove the resolution
    work_dir.run_jj(["util", "gc", "--expire=now"]).success();
    let entries: Vec<_> = std::fs::read_dir(&cache_dir).unwrap().collect();
    assert_eq!(
        entries.len(),
        0,
        "Resolution should be removed with --expire=now"
    );
}

#[test]
fn test_rerere_concurrent_access() {
    let test_env = TestEnvironment::default();
    test_env.add_config("rerere.enabled = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // Create a conflict scenario with proper sibling commits
    work_dir.write_file("file.txt", "base\n");
    work_dir.run_jj(["commit", "-m", "base"]).success();

    // Get the base commit ID
    let base_commit = work_dir
        .run_jj(["log", "-r", "@-", "--no-graph", "-T", "commit_id"])
        .success()
        .stdout
        .into_raw()
        .trim()
        .to_string();

    // Create first sibling
    work_dir.run_jj(["new", &base_commit, "-m", "a"]).success();
    work_dir.write_file("file.txt", "a\n");
    work_dir.run_jj(["commit", "-m", "commit a"]).success();

    // Create second sibling (branching from base, not from a)
    work_dir.run_jj(["new", &base_commit, "-m", "b"]).success();
    work_dir.write_file("file.txt", "b\n");
    work_dir.run_jj(["commit", "-m", "commit b"]).success();

    // Create multiple workspaces to simulate concurrent access
    let repo_path = work_dir.root().to_path_buf();

    // Create workspaces sequentially (avoiding thread safety issues with
    // TestEnvironment)
    for i in 0..3 {
        let workspace_name = format!("workspace{i}");
        let workspace_path = repo_path.parent().unwrap().join(&workspace_name);
        work_dir
            .run_jj([
                "workspace",
                "add",
                workspace_path.to_str().unwrap(),
                "-r",
                "description(\"commit a\")",
            ])
            .success();

        // Create a merge which should produce a conflict
        // Use the main work_dir but specify the workspace path
        let merge_output = work_dir
            .run_jj([
                "-R",
                workspace_path.to_str().unwrap(),
                "new",
                "description(\"commit a\")",
                "description(\"commit b\")",
                "-m",
                &format!("merge{i}"),
            ])
            .success();

        // On first iteration we should get a conflict, on subsequent ones rerere might
        // resolve it
        if i == 0 {
            // First time should create a conflict
            let status = work_dir
                .run_jj(["-R", workspace_path.to_str().unwrap(), "status"])
                .success();
            assert!(
                status
                    .stdout
                    .raw()
                    .contains("There are unresolved conflicts at these paths:"),
                "Expected conflict message in output but got: {}",
                status.stdout.raw()
            );

            // Resolve the conflict with different content
            std::fs::write(workspace_path.join("file.txt"), format!("resolved{i}\n")).unwrap();

            // Trigger snapshot to record the resolution
            work_dir
                .run_jj(["-R", workspace_path.to_str().unwrap(), "status"])
                .success();
        } else {
            // Check if rerere auto-resolved it
            if merge_output.stderr.raw().contains("Applied")
                && merge_output
                    .stderr
                    .raw()
                    .contains("cached conflict resolution")
            {
                // Rerere already resolved it, good!
                continue;
            }

            // Otherwise resolve it manually
            std::fs::write(workspace_path.join("file.txt"), format!("resolved{i}\n")).unwrap();
            work_dir
                .run_jj(["-R", workspace_path.to_str().unwrap(), "status"])
                .success();
        }
    }

    // Check that the resolution cache is consistent
    let cache_dir = work_dir.root().join(".jj/repo/resolution_cache");
    assert!(cache_dir.exists());

    // There should be at least one resolution recorded
    let entries: Vec<_> = std::fs::read_dir(&cache_dir).unwrap().collect();
    assert!(
        !entries.is_empty(),
        "Resolution cache should contain entries"
    );

    // Verify that new merges can still use the cached resolutions
    work_dir.run_jj(["workspace", "update-stale"]).success();
    let merge_output = work_dir
        .run_jj([
            "new",
            "description(\"commit a\")",
            "description(\"commit b\")",
            "-m",
            "final_merge",
        ])
        .success();

    // The conflict should be auto-resolved if rerere is working correctly
    // Check that rerere applied a cached resolution
    assert!(merge_output
        .stderr
        .raw()
        .contains("Applied 1 cached conflict resolution"));

    // Verify no conflicts remain
    let status = work_dir.run_jj(["status"]).success();
    assert!(!status.stderr.raw().contains("unresolved conflicts"));
}

#[test]
fn test_shell_completions() {
    #[track_caller]
    fn test(shell: &str) {
        let test_env = TestEnvironment::default();
        // Use the local backend because GitBackend::gc() depends on the git CLI.
        let output = test_env
            .run_jj_in(".", ["util", "completion", shell])
            .success();
        // Ensures only stdout contains text
        assert!(
            !output.stdout.is_empty() && output.stderr.is_empty(),
            "{output}"
        );
    }

    test("bash");
    test("fish");
    test("nushell");
    test("zsh");
}

#[test]
fn test_util_exec() {
    let test_env = TestEnvironment::default();
    let formatter_path = assert_cmd::cargo::cargo_bin("fake-formatter");
    let output = test_env.run_jj_in(
        ".",
        [
            "util",
            "exec",
            "--",
            formatter_path.to_str().unwrap(),
            "--append",
            "hello",
        ],
    );
    // Ensures only stdout contains text
    insta::assert_snapshot!(output, @"hello[EOF]");
}

#[test]
fn test_util_exec_fail() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(".", ["util", "exec", "--", "jj-test-missing-program"]);
    insta::assert_snapshot!(output.strip_stderr_last_line(), @r"
    ------- stderr -------
    Error: Failed to execute external command 'jj-test-missing-program'
    [EOF]
    [exit status: 1]
    ");
}
