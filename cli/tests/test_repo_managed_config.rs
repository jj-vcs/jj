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
    "###);

    test_env.add_config(r"repo-managed-config.enabled = true");

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

    work_dir.write_file(
        ".jj/repo/config.toml",
        r#"repo-managed-config.enabled = false"#,
    );

    other_dir.write_file(
        ".jj/repo/config.toml",
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
    Warning: repo-managed-config is disabled.
    Hint: Enable it with `jj config set <--user|--repo> repo-managed-config.enabled true`
    Updated repo config file
    [EOF]
    "###);
    let output = work_dir.run_jj(["config", "get", "ui.pager"]);
    insta::assert_snapshot!(output, @r###"
    repo-managed pager
    [EOF]
    "###);
}
