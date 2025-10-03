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

#[test]
fn test_repo_managed_config() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    test_env.add_config(r#"ui.pager = "user pager""#);

    let output = work_dir.run_jj(["config", "get", "ui.pager"]);
    insta::assert_snapshot!(output, @r###"
    user pager
    [EOF]
    "###);

    work_dir.write_file(
        ".config/jj/config.toml",
        r#"repo-managed-config.enabled = false"#,
    );

    let output = work_dir.run_jj(["config", "get", "ui.pager"]);
    insta::assert_snapshot!(output, @r###"
    user pager
    [EOF]
    ------- stderr -------
    Warning: Your repo-managed config is out of date
    Hint: Run `jj config review-managed`
    [EOF]
    "###);

    // We have to use the --trust flag here because we can't interact with the TUI.
    let output = work_dir.run_jj(["config", "review-managed", "--trust"]);
    insta::assert_snapshot!(output, @r###"
    ------- stderr -------
    Warning: Your repo-managed config is out of date
    Hint: Run `jj config review-managed`
    Updated repo config file
    [EOF]
    "###);

    let output = work_dir.run_jj(["config", "get", "ui.pager"]);
    insta::assert_snapshot!(output, @r###"
    user pager
    [EOF]
    "###);

    test_env.add_config(r"repo-managed-config.enabled = false");
    let output = work_dir.run_jj(["config", "get", "ui.pager"]);
    insta::assert_snapshot!(output, @r###"
    user pager
    [EOF]
    "###);

    test_env.add_config(
        r###"
    [[--scope]]
    --when.commands = ["config get"]
    [--scope.repo-managed-config]
    enabled = true
    "###,
    );
    work_dir.write_file(
        ".config/jj/config.toml",
        r#"ui.pager = "repo-managed pager""#,
    );
    let output = work_dir.run_jj(["config", "get", "ui.pager"]);
    insta::assert_snapshot!(output, @r###"
    user pager
    [EOF]
    ------- stderr -------
    Warning: Your repo-managed config is out of date
    Hint: Run `jj config review-managed`
    [EOF]
    "###);

    let other_dir = test_env.work_dir(".");
    let output = other_dir.run_jj([
        "-R",
        work_dir.root().to_str().unwrap(),
        "config",
        "get",
        "ui.pager",
    ]);
    insta::assert_snapshot!(output, @r###"
    user pager
    [EOF]
    ------- stderr -------
    Warning: Your repo-managed config is out of date
    Hint: Run `jj config review-managed`
    [EOF]
    "###);

    // If the repo says to enable repo-managed config, but the user disallows it,
    // it should definitely not be enabled.
    work_dir.write_file(
        ".jj/repo/config.toml",
        r#"repo-managed-config.enabled = false"#,
    );
    work_dir.write_file(
        ".config/jj/config.toml",
        r#"repo-managed-config.enabled = true"#,
    );
    other_dir.write_file(
        ".config/jj/config.toml",
        r#"repo-managed-config.enabled = true"#,
    );
    let output = other_dir.run_jj([
        "-R",
        work_dir.root().to_str().unwrap(),
        "config",
        "get",
        "ui.pager",
    ]);
    insta::assert_snapshot!(output, @r###"
    user pager
    [EOF]
    "###);

    work_dir.write_file(".jj/repo/config.toml", "");

    // We have to use the --trust flag here because we can't interact with the TUI.
    let output = work_dir.run_jj(["config", "review-managed", "--trust"]);
    insta::assert_snapshot!(output, @r###"
    ------- stderr -------
    Updated repo config file
    [EOF]
    "###);
    let output = work_dir.run_jj(["config", "get", "ui.pager"]);
    insta::assert_snapshot!(output, @r###"
    user pager
    [EOF]
    "###);
}

#[test]
fn test_multi_workspace_config() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    test_env
        .run_jj_in("repo", ["workspace", "add", "../second"])
        .success();
    let work_dir = test_env.work_dir("repo");
    let second_dir = test_env.work_dir("second");
    test_env.add_config(r#"ui.pager = "user pager""#);

    let output = work_dir.run_jj(["config", "get", "ui.pager"]);
    insta::assert_snapshot!(output, @r###"
    user pager
    [EOF]
    "###);

    work_dir.write_file(".config/jj/config.toml", r#"ui.pager = "repo pager""#);

    // The config should be out of date in the main repo
    let output = work_dir.run_jj(["config", "get", "ui.pager"]);
    insta::assert_snapshot!(output, @r###"
    user pager
    [EOF]
    ------- stderr -------
    Warning: Your repo-managed config is out of date
    Hint: Run `jj config review-managed`
    [EOF]
    "###);

    // But the second repo should still rely on its own local config
    let output = second_dir.run_jj(["config", "get", "ui.pager"]);
    insta::assert_snapshot!(output, @r###"
    user pager
    [EOF]
    "###);

    second_dir.write_file(".config/jj/config.toml", r#"ui.pager = "repo pager""#);

    // Now the second dir is out of date
    let output = second_dir.run_jj(["config", "get", "ui.pager"]);
    insta::assert_snapshot!(output, @r###"
    user pager
    [EOF]
    ------- stderr -------
    Warning: Your repo-managed config is out of date
    Hint: Run `jj config review-managed`
    [EOF]
    "###);

    work_dir
        .run_jj(["config", "review-managed", "--trust"])
        .success();

    // We should now be using the new config.
    let output = work_dir.run_jj(["config", "get", "ui.pager"]);
    insta::assert_snapshot!(output, @r###"
    repo pager
    [EOF]
    "###);

    // Approving the same content for one workspace should approve it for the other.
    let output = second_dir.run_jj(["config", "get", "ui.pager"]);
    insta::assert_snapshot!(output, @r###"
    repo pager
    [EOF]
    "###);

    // Now we update them divergently
    work_dir.write_file(".config/jj/config.toml", r#"ui.pager = "repo pager v2""#);
    second_dir.write_file(".config/jj/config.toml", r#"ui.pager = "second pager v2""#);

    // Both directories should use the last approved config.
    // This is despite the fact that the the second workspace doesn't have a
    // last approved config, so it needs to use the global last approved config
    let output = work_dir.run_jj(["config", "get", "ui.pager"]);
    insta::assert_snapshot!(output, @r###"
    repo pager
    [EOF]
    ------- stderr -------
    Warning: Your repo-managed config is out of date
    Hint: Run `jj config review-managed`
    [EOF]
    "###);
    let output = second_dir.run_jj(["config", "get", "ui.pager"]);
    insta::assert_snapshot!(output, @r###"
    repo pager
    [EOF]
    ------- stderr -------
    Warning: Your repo-managed config is out of date
    Hint: Run `jj config review-managed`
    [EOF]
    "###);

    // This time we update them divergently, but both have a last approved config.
    work_dir
        .run_jj(["config", "review-managed", "--trust"])
        .success();
    second_dir
        .run_jj(["config", "review-managed", "--trust"])
        .success();
    work_dir.write_file(".config/jj/config.toml", r#"ui.pager = "repo pager v3""#);
    second_dir.write_file(".config/jj/config.toml", r#"ui.pager = "second pager v3""#);

    let output = work_dir.run_jj(["config", "get", "ui.pager"]);
    insta::assert_snapshot!(output, @r###"
    repo pager v2
    [EOF]
    ------- stderr -------
    Warning: Your repo-managed config is out of date
    Hint: Run `jj config review-managed`
    [EOF]
    "###);
    let output = second_dir.run_jj(["config", "get", "ui.pager"]);
    insta::assert_snapshot!(output, @r###"
    second pager v2
    [EOF]
    ------- stderr -------
    Warning: Your repo-managed config is out of date
    Hint: Run `jj config review-managed`
    [EOF]
    "###);
}
