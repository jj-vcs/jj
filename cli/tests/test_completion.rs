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

use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

#[test]
fn test_bookmark_names() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    test_env.run_jj_in(".", ["git", "init", "origin"]).success();
    let origin_dir = test_env.work_dir("origin");
    let origin_git_repo_path = origin_dir
        .root()
        .join(".jj")
        .join("repo")
        .join("store")
        .join("git");

    work_dir
        .run_jj(["bookmark", "create", "-r@", "aaa-local"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bbb-local"])
        .success();

    // add various remote branches
    work_dir
        .run_jj([
            "git",
            "remote",
            "add",
            "origin",
            origin_git_repo_path.to_str().unwrap(),
        ])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "aaa-tracked"])
        .success();
    work_dir
        .run_jj(["desc", "-r", "aaa-tracked", "-m", "x"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bbb-tracked"])
        .success();
    work_dir
        .run_jj(["desc", "-r", "bbb-tracked", "-m", "x"])
        .success();
    work_dir
        .run_jj(["git", "push", "--allow-new", "--bookmark", "glob:*-tracked"])
        .success();

    origin_dir
        .run_jj(["bookmark", "create", "-r@", "aaa-untracked"])
        .success();
    origin_dir
        .run_jj(["desc", "-r", "aaa-untracked", "-m", "x"])
        .success();
    origin_dir
        .run_jj(["bookmark", "create", "-r@", "bbb-untracked"])
        .success();
    origin_dir
        .run_jj(["desc", "-r", "bbb-untracked", "-m", "x"])
        .success();
    origin_dir.run_jj(["git", "export"]).success();
    work_dir.run_jj(["git", "fetch"]).success();

    let mut test_env = test_env;
    // Every shell hook is a little different, e.g. the zsh hooks add some
    // additional environment variables. But this is irrelevant for the purpose
    // of testing our own logic, so it's fine to test a single shell only.
    test_env.add_env_var("COMPLETE", "fish");
    let test_env = test_env;
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["--", "jj", "bookmark", "rename", ""]);
    insta::assert_snapshot!(output, @r"
    aaa-local	x
    aaa-tracked	x
    bbb-local	x
    bbb-tracked	x
    --repository	Path to repository to operate on
    --ignore-working-copy	Don't snapshot the working copy, and don't update it
    --ignore-immutable	Allow rewriting immutable commits
    --at-operation	Operation to load the repo at
    --debug	Enable debug logging
    --color	When to colorize output
    --quiet	Silence non-primary command output
    --no-pager	Disable the pager
    --config	Additional configuration options (can be repeated)
    --config-file	Additional configuration files (can be repeated)
    --help	Print help (see more with '--help')
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "bookmark", "rename", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-local	x
    aaa-tracked	x
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "bookmark", "delete", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-local	x
    aaa-tracked	x
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "bookmark", "forget", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-local	x
    aaa-tracked	x
    aaa-untracked
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "bookmark", "list", "--bookmark", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-local	x
    aaa-tracked	x
    aaa-untracked
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "bookmark", "move", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-local	x
    aaa-tracked	x
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "bookmark", "set", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-local	x
    aaa-tracked	x
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "bookmark", "track", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-untracked@origin	x
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "bookmark", "untrack", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-tracked@origin	x
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "git", "push", "-b", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-local	x
    aaa-tracked	x
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "git", "fetch", "-b", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-local	x
    aaa-tracked	x
    aaa-untracked
    [EOF]
    ");
}

#[test]
fn test_global_arg_repository_is_respected() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["bookmark", "create", "-r@", "aaa"])
        .success();

    let mut test_env = test_env;
    test_env.add_env_var("COMPLETE", "fish");
    let test_env = test_env;

    let output = test_env.run_jj_in(
        ".",
        [
            "--",
            "jj",
            "--repository",
            "repo",
            "bookmark",
            "rename",
            "a",
        ],
    );
    insta::assert_snapshot!(output, @r"
    aaa	(no description set)
    [EOF]
    ");
}

#[test]
fn test_aliases_are_resolved() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["bookmark", "create", "-r@", "aaa"])
        .success();

    // user config alias
    test_env.add_config(r#"aliases.b = ["bookmark"]"#);
    // repo config alias
    work_dir
        .run_jj(["config", "set", "--repo", "aliases.b2", "['bookmark']"])
        .success();

    let mut test_env = test_env;
    test_env.add_env_var("COMPLETE", "fish");
    let test_env = test_env;
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["--", "jj", "b", "rename", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa	(no description set)
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "b2", "rename", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa	(no description set)
    [EOF]
    ");
}

#[test]
fn test_completions_are_generated() {
    let mut test_env = TestEnvironment::default();
    test_env.add_env_var("COMPLETE", "fish");
    let mut insta_settings = insta::Settings::clone_current();
    insta_settings.add_filter(r"(--arguments) .*", "$1 .."); // omit path to jj binary
    let _guard = insta_settings.bind_to_scope();

    let output = test_env.run_jj_in(".", [""; 0]);
    insta::assert_snapshot!(output, @r"
    complete --keep-order --exclusive --command jj --arguments ..
    [EOF]
    ");
    let output = test_env.run_jj_in(".", ["--"]);
    insta::assert_snapshot!(output, @r"
    complete --keep-order --exclusive --command jj --arguments ..
    [EOF]
    ");
}

#[test]
fn test_zsh_completion() {
    let mut test_env = TestEnvironment::default();
    test_env.add_env_var("COMPLETE", "zsh");

    // ["--", "jj"]
    //        ^^^^ index = 0
    let complete_at = |index: usize, args: &[&str]| {
        test_env.run_jj_with(|cmd| {
            cmd.args(args)
                .env("_CLAP_COMPLETE_INDEX", index.to_string())
        })
    };

    // Command names should be suggested. If the default command were expanded,
    // only "log" would be listed.
    let output = complete_at(1, &["--", "jj"]);
    insta::assert_snapshot!(
        output.normalize_stdout_with(|s| s.split_inclusive('\n').take(2).collect()), @r"
    abandon:Abandon a revision
    absorb:Move changes from a revision into the stack of mutable revisions
    [EOF]
    ");
    let output = complete_at(2, &["--", "jj", "--no-pager"]);
    insta::assert_snapshot!(
        output.normalize_stdout_with(|s| s.split_inclusive('\n').take(2).collect()), @r"
    abandon:Abandon a revision
    absorb:Move changes from a revision into the stack of mutable revisions
    [EOF]
    ");

    let output = complete_at(1, &["--", "jj", "b"]);
    insta::assert_snapshot!(output, @"bookmark:Manage bookmarks [default alias: b][EOF]");
}

#[test]
fn test_remote_names() {
    let mut test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init"]).success();

    test_env
        .run_jj_in(
            ".",
            ["git", "remote", "add", "origin", "git@git.local:user/repo"],
        )
        .success();

    test_env.add_env_var("COMPLETE", "fish");

    let output = test_env.run_jj_in(".", ["--", "jj", "git", "remote", "remove", "o"]);
    insta::assert_snapshot!(output, @r"
    origin
    [EOF]
    ");

    let output = test_env.run_jj_in(".", ["--", "jj", "git", "remote", "rename", "o"]);
    insta::assert_snapshot!(output, @r"
    origin
    [EOF]
    ");

    let output = test_env.run_jj_in(".", ["--", "jj", "git", "remote", "set-url", "o"]);
    insta::assert_snapshot!(output, @r"
    origin
    [EOF]
    ");

    let output = test_env.run_jj_in(".", ["--", "jj", "git", "push", "--remote", "o"]);
    insta::assert_snapshot!(output, @r"
    origin
    [EOF]
    ");

    let output = test_env.run_jj_in(".", ["--", "jj", "git", "fetch", "--remote", "o"]);
    insta::assert_snapshot!(output, @r"
    origin
    [EOF]
    ");

    let output = test_env.run_jj_in(".", ["--", "jj", "bookmark", "list", "--remote", "o"]);
    insta::assert_snapshot!(output, @r"
    origin
    [EOF]
    ");
}

#[test]
fn test_aliases_are_completed() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // user config alias
    test_env.add_config(r#"aliases.user-alias = ["bookmark"]"#);
    // repo config alias
    work_dir
        .run_jj([
            "config",
            "set",
            "--repo",
            "aliases.repo-alias",
            "['bookmark']",
        ])
        .success();

    let mut test_env = test_env;
    test_env.add_env_var("COMPLETE", "fish");
    let test_env = test_env;
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["--", "jj", "user-al"]);
    insta::assert_snapshot!(output, @r"
    user-alias
    [EOF]
    ");

    // make sure --repository flag is respected
    let output = test_env.run_jj_in(
        ".",
        [
            "--",
            "jj",
            "--repository",
            work_dir.root().to_str().unwrap(),
            "repo-al",
        ],
    );
    insta::assert_snapshot!(output, @r"
    repo-alias
    [EOF]
    ");

    // cannot load aliases from --config flag
    let output = test_env.run_jj_in(
        ".",
        [
            "--",
            "jj",
            "--config=aliases.cli-alias=['bookmark']",
            "cli-al",
        ],
    );
    insta::assert_snapshot!(output, @"");
}

#[test]
fn test_revisions() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // create remote to test remote branches
    test_env.run_jj_in(".", ["git", "init", "origin"]).success();
    let origin_dir = test_env.work_dir("origin");
    let origin_git_repo_path = origin_dir
        .root()
        .join(".jj")
        .join("repo")
        .join("store")
        .join("git");
    work_dir
        .run_jj([
            "git",
            "remote",
            "add",
            "origin",
            origin_git_repo_path.to_str().unwrap(),
        ])
        .success();
    origin_dir
        .run_jj(["b", "c", "-r@", "remote_bookmark"])
        .success();
    origin_dir
        .run_jj(["commit", "-m", "remote_commit"])
        .success();
    origin_dir.run_jj(["git", "export"]).success();
    work_dir.run_jj(["git", "fetch"]).success();

    work_dir
        .run_jj(["b", "c", "-r@", "immutable_bookmark"])
        .success();
    work_dir.run_jj(["commit", "-m", "immutable"]).success();
    test_env.add_config(r#"revset-aliases."immutable_heads()" = "immutable_bookmark""#);
    test_env.add_config(r#"revset-aliases."siblings" = "@-+ ~@""#);
    test_env.add_config(
        r#"revset-aliases."alias_with_newline" = '''
    roots(
        conflicts()
    )
    '''"#,
    );

    work_dir
        .run_jj(["b", "c", "-r@", "mutable_bookmark"])
        .success();
    work_dir.run_jj(["commit", "-m", "mutable"]).success();

    work_dir
        .run_jj(["describe", "-m", "working_copy"])
        .success();

    let mut test_env = test_env;
    test_env.add_env_var("COMPLETE", "fish");
    let test_env = test_env;
    let work_dir = test_env.work_dir("repo");

    // There are _a lot_ of commands and arguments accepting revisions.
    // Let's not test all of them. Having at least one test per variation of
    // completion function should be sufficient.

    // complete all revisions
    let output = work_dir.run_jj(["--", "jj", "diff", "--from", ""]);
    insta::assert_snapshot!(output, @r"
    immutable_bookmark	immutable
    mutable_bookmark	mutable
    k	working_copy
    y	mutable
    q	immutable
    zq	remote_commit
    zz	(no description set)
    remote_bookmark@origin	remote_commit
    alias_with_newline	    roots(
    siblings	@-+ ~@
    [EOF]
    ");

    // complete only mutable revisions
    let output = work_dir.run_jj(["--", "jj", "squash", "--into", ""]);
    insta::assert_snapshot!(output, @r"
    mutable_bookmark	mutable
    k	working_copy
    y	mutable
    zq	remote_commit
    alias_with_newline	    roots(
    siblings	@-+ ~@
    [EOF]
    ");

    // complete args of the default command
    test_env.add_config("ui.default-command = 'log'");
    let output = work_dir.run_jj(["--", "jj", "-r", ""]);
    insta::assert_snapshot!(output, @r"
    immutable_bookmark	immutable
    mutable_bookmark	mutable
    k	working_copy
    y	mutable
    q	immutable
    zq	remote_commit
    zz	(no description set)
    remote_bookmark@origin	remote_commit
    alias_with_newline	    roots(
    siblings	@-+ ~@
    [EOF]
    ");

    // Begin testing `jj git push --named`

    // The name of a bookmark does not get completed, since we want to create a new
    // bookmark
    let output = work_dir.run_jj(["--", "jj", "git", "push", "--named", ""]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.run_jj(["--", "jj", "git", "push", "--named", "a"]);
    insta::assert_snapshot!(output, @"");

    let output = work_dir.run_jj(["--", "jj", "git", "push", "--named", "a="]);
    insta::assert_snapshot!(output, @r"
    a=immutable_bookmark	immutable
    a=mutable_bookmark	mutable
    a=k	working_copy
    a=y	mutable
    a=q	immutable
    a=zq	remote_commit
    a=zz	(no description set)
    a=remote_bookmark@origin	remote_commit
    a=alias_with_newline	    roots(
    a=siblings	@-+ ~@
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "git", "push", "--named", "a=a"]);
    insta::assert_snapshot!(output, @r"
    a=alias_with_newline	    roots(
    [EOF]
    ");
}

#[test]
fn test_operations() {
    let test_env = TestEnvironment::default();

    // suppress warnings on stderr of completions for invalid args
    test_env.add_config("ui.default-command = 'log'");

    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    work_dir
        .run_jj(["describe", "-m", "description 0"])
        .success();
    work_dir
        .run_jj(["describe", "-m", "description 1"])
        .success();
    work_dir
        .run_jj(["describe", "-m", "description 2"])
        .success();
    work_dir
        .run_jj(["describe", "-m", "description 3"])
        .success();
    work_dir
        .run_jj(["describe", "-m", "description 4"])
        .success();

    let mut test_env = test_env;
    test_env.add_env_var("COMPLETE", "fish");
    let test_env = test_env;
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["--", "jj", "op", "show", ""]).success();
    let add_workspace_id = output
        .stdout
        .raw()
        .lines()
        .nth(5)
        .unwrap()
        .split('\t')
        .next()
        .unwrap();
    insta::assert_snapshot!(add_workspace_id, @"eac759b9ab75");

    let output = work_dir.run_jj(["--", "jj", "op", "show", "5"]);
    insta::assert_snapshot!(output, @r"
    5bbb4ca536a8	(2001-02-03 08:05:12) describe commit 968261075dddabf4b0e333c1cc9a49ce26a3f710
    518b588abbc6	(2001-02-03 08:05:09) describe commit 19611c995a342c01f525583e5fcafdd211f6d009
    [EOF]
    ");
    // make sure global --at-op flag is respected
    let output = work_dir.run_jj(["--", "jj", "--at-op", "518b588abbc6", "op", "show", "5"]);
    insta::assert_snapshot!(output, @r"
    518b588abbc6	(2001-02-03 08:05:09) describe commit 19611c995a342c01f525583e5fcafdd211f6d009
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "--at-op", "5b"]);
    insta::assert_snapshot!(output, @r"
    5bbb4ca536a8	(2001-02-03 08:05:12) describe commit 968261075dddabf4b0e333c1cc9a49ce26a3f710
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "op", "abandon", "5b"]);
    insta::assert_snapshot!(output, @r"
    5bbb4ca536a8	(2001-02-03 08:05:12) describe commit 968261075dddabf4b0e333c1cc9a49ce26a3f710
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "op", "diff", "--op", "5b"]);
    insta::assert_snapshot!(output, @r"
    5bbb4ca536a8	(2001-02-03 08:05:12) describe commit 968261075dddabf4b0e333c1cc9a49ce26a3f710
    [EOF]
    ");
    let output = work_dir.run_jj(["--", "jj", "op", "diff", "--from", "5b"]);
    insta::assert_snapshot!(output, @r"
    5bbb4ca536a8	(2001-02-03 08:05:12) describe commit 968261075dddabf4b0e333c1cc9a49ce26a3f710
    [EOF]
    ");
    let output = work_dir.run_jj(["--", "jj", "op", "diff", "--to", "5b"]);
    insta::assert_snapshot!(output, @r"
    5bbb4ca536a8	(2001-02-03 08:05:12) describe commit 968261075dddabf4b0e333c1cc9a49ce26a3f710
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "op", "restore", "5b"]);
    insta::assert_snapshot!(output, @r"
    5bbb4ca536a8	(2001-02-03 08:05:12) describe commit 968261075dddabf4b0e333c1cc9a49ce26a3f710
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "op", "undo", "5b"]);
    insta::assert_snapshot!(output, @r"
    5bbb4ca536a8	(2001-02-03 08:05:12) describe commit 968261075dddabf4b0e333c1cc9a49ce26a3f710
    [EOF]
    ");
}

#[test]
fn test_workspaces() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "main"]).success();
    let main_dir = test_env.work_dir("main");

    main_dir.write_file("file", "contents");
    main_dir.run_jj(["describe", "-m", "initial"]).success();

    // same prefix as "default" workspace
    main_dir
        .run_jj(["workspace", "add", "--name", "def-second", "../secondary"])
        .success();

    let mut test_env = test_env;
    test_env.add_env_var("COMPLETE", "fish");
    let test_env = test_env;
    let main_dir = test_env.work_dir("main");

    let output = main_dir.run_jj(["--", "jj", "workspace", "forget", "def"]);
    insta::assert_snapshot!(output, @r"
    def-second	(no description set)
    default	initial
    [EOF]
    ");
}

#[test]
fn test_config() {
    let mut test_env = TestEnvironment::default();
    test_env.add_env_var("COMPLETE", "fish");
    let dir = test_env.env_root();

    let output = test_env.run_jj_in(dir, ["--", "jj", "config", "get", "c"]);
    insta::assert_snapshot!(output, @r"
    core.fsmonitor	Whether to use an external filesystem monitor, useful for large repos
    core.watchman.register-snapshot-trigger	Whether to use triggers to monitor for changes in the background.
    [EOF]
    ");

    let output = test_env.run_jj_in(dir, ["--", "jj", "config", "list", "c"]);
    insta::assert_snapshot!(output, @r"
    colors	Mapping from jj formatter labels to colors
    core
    core.fsmonitor	Whether to use an external filesystem monitor, useful for large repos
    core.watchman
    core.watchman.register-snapshot-trigger	Whether to use triggers to monitor for changes in the background.
    [EOF]
    ");

    let output = test_env.run_jj_in(dir, ["--", "jj", "log", "--config", "c"]);
    insta::assert_snapshot!(output, @r"
    core.fsmonitor=	Whether to use an external filesystem monitor, useful for large repos
    core.watchman.register-snapshot-trigger=	Whether to use triggers to monitor for changes in the background.
    [EOF]
    ");

    let output = test_env.run_jj_in(
        dir,
        ["--", "jj", "log", "--config", "ui.conflict-marker-style="],
    );
    insta::assert_snapshot!(output, @r"
    ui.conflict-marker-style=diff
    ui.conflict-marker-style=snapshot
    ui.conflict-marker-style=git
    [EOF]
    ");
    let output = test_env.run_jj_in(
        dir,
        ["--", "jj", "log", "--config", "ui.conflict-marker-style=g"],
    );
    insta::assert_snapshot!(output, @r"
    ui.conflict-marker-style=git
    [EOF]
    ");

    let output = test_env.run_jj_in(
        dir,
        [
            "--",
            "jj",
            "log",
            "--config",
            "git.abandon-unreachable-commits=",
        ],
    );
    insta::assert_snapshot!(output, @r"
    git.abandon-unreachable-commits=false
    git.abandon-unreachable-commits=true
    [EOF]
    ");
}

#[test]
fn test_template_alias() {
    let mut test_env = TestEnvironment::default();
    test_env.add_env_var("COMPLETE", "fish");
    let dir = test_env.env_root();

    let output = test_env.run_jj_in(dir, ["--", "jj", "log", "-T", ""]);
    insta::assert_snapshot!(output, @r"
    builtin_config_list
    builtin_config_list_detailed
    builtin_draft_commit_description
    builtin_log_comfortable
    builtin_log_compact
    builtin_log_compact_full_description
    builtin_log_detailed
    builtin_log_node
    builtin_log_node_ascii
    builtin_log_oneline
    builtin_op_log_comfortable
    builtin_op_log_compact
    builtin_op_log_node
    builtin_op_log_node_ascii
    builtin_op_log_oneline
    commit_summary_separator
    description_placeholder
    email_placeholder
    name_placeholder
    [EOF]
    ");
}

fn create_commit(
    work_dir: &TestWorkDir,
    name: &str,
    parents: &[&str],
    files: &[(&str, Option<&str>)],
) {
    let parents = match parents {
        [] => &["root()"],
        parents => parents,
    };
    work_dir
        .run_jj_with(|cmd| cmd.args(["new", "-m", name]).args(parents))
        .success();
    for (name, content) in files {
        if let Some((dir, _)) = name.rsplit_once('/') {
            work_dir.create_dir_all(dir);
        }
        match content {
            Some(content) => work_dir.write_file(name, content),
            None => work_dir.remove_file(name),
        }
    }
    work_dir
        .run_jj(["bookmark", "create", "-r@", name])
        .success();
}

#[test]
fn test_files() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    create_commit(
        &work_dir,
        "first",
        &[],
        &[
            ("f_unchanged", Some("unchanged\n")),
            ("f_modified", Some("not_yet_modified\n")),
            ("f_not_yet_renamed", Some("renamed\n")),
            ("f_deleted", Some("not_yet_deleted\n")),
            // not yet: "added" file
        ],
    );
    create_commit(
        &work_dir,
        "second",
        &["first"],
        &[
            // "unchanged" file
            ("f_modified", Some("modified\n")),
            ("f_not_yet_renamed", None),
            ("f_renamed", Some("renamed\n")),
            ("f_deleted", None),
            ("f_added", Some("added\n")),
            ("f_dir/dir_file_1", Some("foo\n")),
            ("f_dir/dir_file_2", Some("foo\n")),
            ("f_dir/dir_file_3", Some("foo\n")),
        ],
    );

    // create a conflicted commit to check the completions of `jj restore`
    create_commit(
        &work_dir,
        "conflicted",
        &["second"],
        &[
            ("f_modified", Some("modified_again\n")),
            ("f_added_2", Some("added_2\n")),
            ("f_dir/dir_file_1", Some("bar\n")),
            ("f_dir/dir_file_2", Some("bar\n")),
            ("f_dir/dir_file_3", Some("bar\n")),
        ],
    );
    work_dir.run_jj(["rebase", "-r=@", "-d=first"]).success();

    // two commits that are similar but not identical, for `jj interdiff`
    create_commit(
        &work_dir,
        "interdiff_from",
        &[],
        &[
            ("f_interdiff_same", Some("same in both commits\n")),
            (("f_interdiff_only_from"), Some("only from\n")),
        ],
    );
    create_commit(
        &work_dir,
        "interdiff_to",
        &[],
        &[
            ("f_interdiff_same", Some("same in both commits\n")),
            (("f_interdiff_only_to"), Some("only to\n")),
        ],
    );

    // "dirty worktree"
    create_commit(
        &work_dir,
        "working_copy",
        &["second"],
        &[
            ("f_modified", Some("modified_again\n")),
            ("f_added_2", Some("added_2\n")),
        ],
    );

    let output = work_dir.run_jj(["log", "-r", "all()", "--summary"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    @  wqnwkozp test.user@example.com 2001-02-03 08:05:20 working_copy cb594eba
    │  working_copy
    │  A f_added_2
    │  M f_modified
    ○  zsuskuln test.user@example.com 2001-02-03 08:05:11 second 24242473
    │  second
    │  A f_added
    │  D f_deleted
    │  A f_dir/dir_file_1
    │  A f_dir/dir_file_2
    │  A f_dir/dir_file_3
    │  M f_modified
    │  R {f_not_yet_renamed => f_renamed}
    │ ×  royxmykx test.user@example.com 2001-02-03 08:05:14 conflicted 0ba6786b conflict
    ├─╯  conflicted
    │    A f_added_2
    │    A f_dir/dir_file_1
    │    A f_dir/dir_file_2
    │    A f_dir/dir_file_3
    │    M f_modified
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 first 2a2f433c
    │  first
    │  A f_deleted
    │  A f_modified
    │  A f_not_yet_renamed
    │  A f_unchanged
    │ ○  kpqxywon test.user@example.com 2001-02-03 08:05:18 interdiff_to 302c4041
    ├─╯  interdiff_to
    │    A f_interdiff_only_to
    │    A f_interdiff_same
    │ ○  yostqsxw test.user@example.com 2001-02-03 08:05:16 interdiff_from 083d1cc6
    ├─╯  interdiff_from
    │    A f_interdiff_only_from
    │    A f_interdiff_same
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    let mut test_env = test_env;
    test_env.add_env_var("COMPLETE", "fish");
    let test_env = test_env;
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.run_jj(["--", "jj", "file", "show", "f_"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    f_added
    f_added_2
    f_dir/
    f_modified
    f_renamed
    f_unchanged
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "file", "annotate", "-r@-", "f_"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    f_added
    f_dir/
    f_modified
    f_renamed
    f_unchanged
    [EOF]
    ");
    let output = work_dir.run_jj(["--", "jj", "diff", "-r", "@-", "f_"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    f_added	Added
    f_deleted	Deleted
    f_dir/
    f_modified	Modified
    f_not_yet_renamed	Renamed
    f_renamed	Renamed
    [EOF]
    ");

    let output = work_dir.run_jj([
        "--",
        "jj",
        "diff",
        "-r",
        "@-",
        &format!("f_dir{}", std::path::MAIN_SEPARATOR),
    ]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    f_dir/dir_file_1	Added
    f_dir/dir_file_2	Added
    f_dir/dir_file_3	Added
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "diff", "--from", "root()", "--to", "@-", "f_"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    f_added	Added
    f_dir/
    f_modified	Added
    f_renamed	Added
    f_unchanged	Added
    [EOF]
    ");

    // interdiff has a different behavior with --from and --to flags
    let output = work_dir.run_jj([
        "--",
        "jj",
        "interdiff",
        "--to=interdiff_to",
        "--from=interdiff_from",
        "f_",
    ]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    f_interdiff_only_from	Added
    f_interdiff_same	Added
    f_interdiff_only_to	Added
    f_interdiff_same	Added
    [EOF]
    ");

    // squash has a different behavior with --from and --to flags
    let output = work_dir.run_jj(["--", "jj", "squash", "-f=first", "f_"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    f_deleted	Added
    f_modified	Added
    f_not_yet_renamed	Added
    f_unchanged	Added
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "resolve", "-r=conflicted", "f_"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    f_dir/
    f_modified
    [EOF]
    ");

    let output = work_dir.run_jj(["--", "jj", "log", "f_"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    f_added
    f_added_2
    f_dir/
    f_modified
    f_renamed
    f_unchanged
    [EOF]
    ");
    let output = work_dir.run_jj([
        "--",
        "jj",
        "log",
        "-r=first",
        "--revisions",
        "conflicted",
        "f_",
    ]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    f_added_2
    f_deleted
    f_dir/
    f_modified
    f_not_yet_renamed
    f_unchanged
    [EOF]
    ");

    let outside_repo = test_env.env_root();
    let output = test_env.run_jj_in(outside_repo, ["--", "jj", "log", "f_"]);
    insta::assert_snapshot!(output, @"");
}
