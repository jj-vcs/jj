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

use std::ffi::OsStr;

use clap_complete::Shell;
use itertools::Itertools as _;
use test_case::test_case;

use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

impl TestEnvironment {
    /// Runs `jj` as if shell completion had been triggered for the argument of
    /// the given index.
    #[must_use = "either snapshot the output or assert the exit status with .success()"]
    fn complete_at<I>(&self, shell: Shell, index: usize, args: I) -> CommandOutput
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: AsRef<OsStr>,
    {
        self.work_dir("").complete_at(shell, index, args)
    }

    /// Run `jj` as if fish shell completion had been triggered at the last item
    /// in `args`.
    #[must_use = "either snapshot the output or assert the exit status with .success()"]
    fn complete_fish<I>(&self, args: I) -> CommandOutput
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: AsRef<OsStr>,
    {
        self.work_dir("").complete_fish(args)
    }
}

impl TestWorkDir<'_> {
    /// Runs `jj` as if shell completion had been triggered for the argument of
    /// the given index.
    #[must_use = "either snapshot the output or assert the exit status with .success()"]
    fn complete_at<I>(&self, shell: Shell, index: usize, args: I) -> CommandOutput
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: AsRef<OsStr>,
    {
        let args = args.into_iter();
        assert!(
            index <= args.len(),
            "index out of bounds: append empty string to complete after the last argument"
        );
        self.run_jj_with(|cmd| {
            let cmd = cmd.env("COMPLETE", shell.to_string()).args(["--", "jj"]);

            match shell {
                // Bash passes the whole command line except for intermediate empty tokens but
                // preserves trailing empty tokens:
                // `jj log <CURSOR>`            => ["--", "jj", "log", ""]
                //                                 with _CLAP_COMPLETE_INDEX=2
                // `jj log <CURSOR> --no-graph` => ["--", "jj", "log", "--no-graph"]
                //                                 with _CLAP_COMPLETE_INDEX=2
                // `jj log --no-graph<CURSOR>`  => ["--", "jj", "log", "--no-graph"]
                //                                 with _CLAP_COMPLETE_INDEX=2
                //                                 (indistinguishable from the above)
                // `jj log --no-graph <CURSOR>` => ["--", "jj", "log", "--no-graph", ""]
                //                                 with _CLAP_COMPLETE_INDEX=3
                Shell::Bash => {
                    cmd.env("_CLAP_COMPLETE_INDEX", index.to_string())
                        .args(args.coalesce(|a, b| {
                            if a.as_ref().is_empty() {
                                Ok(b)
                            } else {
                                Err((a, b))
                            }
                        }))
                }

                // Zsh passes the whole command line except for empty tokens, including trailing
                // empty tokens:
                // `jj log <CURSOR>`            => ["--", "jj", "log"]
                //                                 with _CLAP_COMPLETE_INDEX=2
                // `jj log <CURSOR> --no-graph` => ["--", "jj", "log", "--no-graph"]
                //                                 with _CLAP_COMPLETE_INDEX=2
                // `jj log --no-graph<CURSOR>`  => ["--", "jj", "log", "--no-graph"]
                //                                 with _CLAP_COMPLETE_INDEX=2
                //                                 (indistinguishable from the above)
                // `jj log --no-graph <CURSOR>` => ["--", "jj", "log", "--no-graph"]
                //                                 with _CLAP_COMPLETE_INDEX=3
                Shell::Zsh => cmd
                    .env("_CLAP_COMPLETE_INDEX", index.to_string())
                    .args(args.filter(|a| !a.as_ref().is_empty())),

                // Fish truncates the command line at the cursor; empty tokens are preserved:
                // `jj log <CURSOR>`            => ["--", "jj", "log", ""]
                // `jj log <CURSOR> --no-graph` => ["--", "jj", "log", ""]
                // `jj log --no-graph<CURSOR>`  => ["--", "jj", "log", "--no-graph"]
                // `jj log --no-graph <CURSOR>` => ["--", "jj", "log", "--no-graph", ""]
                Shell::Fish => cmd.args(args.take(index)),

                _ => todo!("{shell} completion behavior not implemented yet"),
            }
        })
    }

    /// Run `jj` as if fish shell completion had been triggered at the last item
    /// in `args`.
    #[must_use = "either snapshot the output or assert the exit status with .success()"]
    fn complete_fish<I>(&self, args: I) -> CommandOutput
    where
        I: IntoIterator,
        I::IntoIter: ExactSizeIterator,
        I::Item: AsRef<OsStr>,
    {
        let args = args.into_iter();
        self.complete_at(Shell::Fish, args.len(), args)
    }
}

#[test]
fn test_bookmark_names() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "origin"])
        .success();
    let origin_dir = test_env.work_dir("origin");
    test_env
        .run_jj_in(".", ["git", "init", "--colocate", "upstream"])
        .success();
    let _upstream_dir = test_env.work_dir("upstream");

    work_dir
        .run_jj(["bookmark", "create", "-r@", "aaa-local"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "bbb-local"])
        .success();

    // add various remote branches
    work_dir
        .run_jj(["git", "remote", "add", "origin", "../origin"])
        .success();
    work_dir
        .run_jj(["git", "remote", "add", "upstream", "../upstream"])
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
        .run_jj(["bookmark", "track", "--remote=origin", "glob:*-tracked"])
        .success();
    work_dir
        .run_jj(["bookmark", "track", "--remote=upstream", "aaa-tracked"])
        .success();
    work_dir
        .run_jj(["git", "push", "--remote=origin", "--tracked"])
        .success();
    work_dir
        .run_jj(["git", "push", "--remote=upstream", "--tracked"])
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
    work_dir.run_jj(["git", "fetch", "--all-remotes"]).success();

    insta::assert_snapshot!(work_dir.run_jj(["bookmark", "list", "--all"]), @r"
    aaa-local: qpvuntsm fe38a82d (empty) x
    aaa-tracked: qpvuntsm fe38a82d (empty) x
      @origin: qpvuntsm fe38a82d (empty) x
      @upstream: qpvuntsm fe38a82d (empty) x
    aaa-untracked@origin: rlvkpnrz 434ae005 (empty) x
    bbb-local: qpvuntsm fe38a82d (empty) x
    bbb-tracked: qpvuntsm fe38a82d (empty) x
      @origin: qpvuntsm fe38a82d (empty) x
    bbb-untracked@origin: rlvkpnrz 434ae005 (empty) x
    [EOF]
    ");

    // Every shell hook is a little different, e.g. the zsh hooks add some
    // additional environment variables. But this is irrelevant for the purpose
    // of testing our own logic, so it's fine to test a single shell only.
    let work_dir = test_env.work_dir("repo");

    let output = work_dir.complete_fish(["bookmark", "rename", ""]);
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

    let output = work_dir.complete_fish(["bookmark", "rename", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-local	x
    aaa-tracked	x
    [EOF]
    ");

    let output = work_dir.complete_fish(["bookmark", "delete", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-local	x
    aaa-tracked	x
    [EOF]
    ");

    let output = work_dir.complete_fish(["bookmark", "forget", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-local	x
    aaa-tracked	x
    aaa-untracked
    [EOF]
    ");

    let output = work_dir.complete_fish(["bookmark", "list", "--bookmark", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-local	x
    aaa-tracked	x
    aaa-untracked
    [EOF]
    ");

    let output = work_dir.complete_fish(["bookmark", "move", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-local	x
    aaa-tracked	x
    [EOF]
    ");

    let output = work_dir.complete_fish(["bookmark", "set", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-local	x
    aaa-tracked	x
    [EOF]
    ");

    let output = work_dir.complete_fish(["bookmark", "track", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-local	 x
    aaa-untracked	 x
    [EOF]
    ");

    let output = work_dir.complete_fish(["bookmark", "untrack", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-tracked	x
    [EOF]
    ");

    let output = work_dir.complete_fish(["git", "push", "-b", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-local	x
    aaa-tracked	x
    [EOF]
    ");

    let output = work_dir.complete_fish(["git", "fetch", "-b", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-local	x
    aaa-tracked	x
    aaa-untracked
    [EOF]
    ");
}

#[test]
fn test_tag_names() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["commit", "-mcommit1"]).success();
    work_dir
        .run_jj(["tag", "set", "-r@-", "aaa-local"])
        .success();
    work_dir
        .run_jj(["tag", "set", "-r@-", "bbb-local"])
        .success();

    let output = work_dir.complete_fish(["tag", "set", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa-local	commit1
    [EOF]
    ");

    let output = work_dir.complete_fish(["tag", "delete", "b"]);
    insta::assert_snapshot!(output, @r"
    bbb-local	commit1
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

    let output = test_env.complete_fish(["--repository", "repo", "bookmark", "rename", "a"]);
    insta::assert_snapshot!(output, @r"
    aaa	(no description set)
    [EOF]
    ");
}

#[test_case(Shell::Bash; "bash")]
#[test_case(Shell::Zsh; "zsh")]
#[test_case(Shell::Fish; "fish")]
fn test_aliases_are_resolved(shell: Shell) {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["bookmark", "create", "-r@", "aaa"])
        .success();

    // user config alias
    test_env.add_config(r#"aliases.b = ["bookmark"]"#);
    test_env.add_config(r#"aliases.rlog = ["log", "--reversed"]"#);
    // repo config alias
    work_dir
        .run_jj(["config", "set", "--repo", "aliases.b2", "['bookmark']"])
        .success();

    let output = work_dir.complete_at(shell, 3, ["b", "rename", "a"]);
    match shell {
        Shell::Bash => {
            insta::assert_snapshot!(output, @"aaa[EOF]");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @"aaa:(no description set)[EOF]");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @r"
            aaa	(no description set)
            [EOF]
            ");
        }
        _ => unimplemented!("unexpected shell '{shell}'"),
    }

    let output = work_dir.complete_at(shell, 3, ["b2", "rename", "a"]);
    match shell {
        Shell::Bash => {
            insta::assert_snapshot!(output, @"aaa[EOF]");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @"aaa:(no description set)[EOF]");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @r"
            aaa	(no description set)
            [EOF]
            ");
        }
        _ => unimplemented!("unexpected shell '{shell}'"),
    }

    let output = work_dir.complete_at(shell, 2, ["rlog", "--rev"]);
    match shell {
        Shell::Bash => {
            insta::assert_snapshot!(output, @r"
            --revisions
            --reversed[EOF]
            ");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @r"
            --revisions:Which revisions to show
            --reversed:Show revisions in the opposite order (older revisions first)[EOF]
            ");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @r"
            --revisions	Which revisions to show
            --reversed	Show revisions in the opposite order (older revisions first)
            [EOF]
            ");
        }
        _ => unimplemented!("unexpected shell '{shell}'"),
    }
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

#[test_case(Shell::Bash; "bash")]
#[test_case(Shell::Zsh; "zsh")]
#[test_case(Shell::Fish; "fish")]
fn test_default_command_is_resolved(shell: Shell) {
    let test_env = TestEnvironment::default();

    let output = test_env
        .complete_at(shell, 1, ["--"])
        .take_stdout_n_lines(2);
    match shell {
        Shell::Bash => {
            insta::assert_snapshot!(output, @r"
            --revisions
            --limit
            [EOF]
            ");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @r"
            --revisions:Which revisions to show
            --limit:Limit number of revisions to show
            [EOF]
            ");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @r"
            --revisions	Which revisions to show
            --limit	Limit number of revisions to show
            [EOF]
            ");
        }
        _ => unimplemented!("unexpected shell '{shell}'"),
    }

    test_env.add_config("ui.default-command = ['abandon']");
    let output = test_env
        .complete_at(shell, 1, ["--"])
        .take_stdout_n_lines(2);
    match shell {
        Shell::Bash => {
            insta::assert_snapshot!(output, @r"
            --retain-bookmarks
            --restore-descendants
            [EOF]
            ");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @r"
            --retain-bookmarks:Do not delete bookmarks pointing to the revisions to abandon
            --restore-descendants:Do not modify the content of the children of the abandoned commits
            [EOF]
            ");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @r"
            --retain-bookmarks	Do not delete bookmarks pointing to the revisions to abandon
            --restore-descendants	Do not modify the content of the children of the abandoned commits
            [EOF]
            ");
        }
        _ => unimplemented!("unexpected shell '{shell}'"),
    }

    test_env.add_config("ui.default-command = ['bookmark', 'move']");
    let output = test_env
        .complete_at(shell, 1, ["--"])
        .take_stdout_n_lines(2);
    match shell {
        Shell::Bash => {
            insta::assert_snapshot!(output, @r"
            --from
            --to
            [EOF]
            ");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @r"
            --from:Move bookmarks from the given revisions
            --to:Move bookmarks to this revision
            [EOF]
            ");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @r"
            --from	Move bookmarks from the given revisions
            --to	Move bookmarks to this revision
            [EOF]
            ");
        }
        _ => unimplemented!("unexpected shell '{shell}'"),
    }
}

#[test_case(Shell::Bash; "bash")]
#[test_case(Shell::Zsh; "zsh")]
#[test_case(Shell::Fish; "fish")]
fn test_command_completion(shell: Shell) {
    let test_env = TestEnvironment::default();

    // Command names should be suggested. If the default command were expanded,
    // only "log" would be listed.
    let output = test_env.complete_at(shell, 1, [""]).take_stdout_n_lines(2);
    match shell {
        Shell::Bash => {
            insta::assert_snapshot!(output, @r"
            abandon
            absorb
            [EOF]
            ");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @r"
            abandon:Abandon a revision
            absorb:Move changes from a revision into the stack of mutable revisions
            [EOF]
            ");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @r"
            abandon	Abandon a revision
            absorb	Move changes from a revision into the stack of mutable revisions
            [EOF]
            ");
        }
        _ => unimplemented!("unexpected shell '{shell}'"),
    }

    let output = test_env
        .complete_at(shell, 2, ["--no-pager", ""])
        .take_stdout_n_lines(2);
    match shell {
        Shell::Bash => {
            insta::assert_snapshot!(output, @r"
            abandon
            absorb
            [EOF]
            ");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @r"
            abandon:Abandon a revision
            absorb:Move changes from a revision into the stack of mutable revisions
            [EOF]
            ");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @r"
            abandon	Abandon a revision
            absorb	Move changes from a revision into the stack of mutable revisions
            [EOF]
            ");
        }
        _ => unimplemented!("unexpected shell '{shell}'"),
    }

    let output = test_env.complete_at(shell, 1, ["b"]);
    match shell {
        Shell::Zsh => {
            insta::assert_snapshot!(output, @"bookmark:Manage bookmarks [default alias: b][EOF]");
        }
        Shell::Bash => {
            insta::assert_snapshot!(output, @"bookmark[EOF]");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @r"
            bookmark	Manage bookmarks [default alias: b]
            [EOF]
            ");
        }
        _ => unimplemented!("unexpected shell '{shell}'"),
    }

    let output = test_env.complete_at(shell, 1, ["aban", "-r", "@"]);
    match shell {
        Shell::Bash => {
            insta::assert_snapshot!(output, @"abandon[EOF]");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @"abandon:Abandon a revision[EOF]");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @r"
            abandon	Abandon a revision
            [EOF]
            ");
        }
        _ => unimplemented!("unexpected shell '{shell}'"),
    }
}

#[test]
fn test_remote_names() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init"]).success();

    test_env
        .run_jj_in(
            ".",
            ["git", "remote", "add", "origin", "git@git.local:user/repo"],
        )
        .success();

    let output = test_env.complete_fish(["git", "remote", "remove", "o"]);
    insta::assert_snapshot!(output, @r"
    origin
    [EOF]
    ");

    let output = test_env.complete_fish(["git", "remote", "rename", "o"]);
    insta::assert_snapshot!(output, @r"
    origin
    [EOF]
    ");

    let output = test_env.complete_fish(["git", "remote", "set-url", "o"]);
    insta::assert_snapshot!(output, @r"
    origin
    [EOF]
    ");

    let output = test_env.complete_fish(["git", "push", "--remote", "o"]);
    insta::assert_snapshot!(output, @r"
    origin
    [EOF]
    ");

    let output = test_env.complete_fish(["git", "fetch", "--remote", "o"]);
    insta::assert_snapshot!(output, @r"
    origin
    [EOF]
    ");

    let output = test_env.complete_fish(["bookmark", "list", "--remote", "o"]);
    insta::assert_snapshot!(output, @r"
    origin
    [EOF]
    ");
}

#[test_case(Shell::Bash; "bash")]
#[test_case(Shell::Zsh; "zsh")]
#[test_case(Shell::Fish; "fish")]
fn test_aliases_are_completed(shell: Shell) {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");
    let repo_path = work_dir.root().to_str().unwrap();

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

    let output = work_dir.complete_at(shell, 1, ["user-al"]);
    match shell {
        Shell::Bash => {
            insta::assert_snapshot!(output, @"user-alias[EOF]");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @"user-alias[EOF]");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @r"
            user-alias
            [EOF]
            ");
        }
        _ => unimplemented!("unexpected shell '{shell}'"),
    }

    // make sure --repository flag is respected
    let output = test_env.complete_at(shell, 3, ["--repository", repo_path, "repo-al"]);
    match shell {
        Shell::Bash => {
            insta::assert_snapshot!(output, @"repo-alias[EOF]");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @"repo-alias[EOF]");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @r"
            repo-alias
            [EOF]
            ");
        }
        _ => unimplemented!("unexpected shell '{shell}'"),
    }

    // cannot load aliases from --config flag
    let output = test_env.complete_at(
        shell,
        2,
        ["--config=aliases.cli-alias=['bookmark']", "cli-al"],
    );
    assert!(
        output.status.success() && output.stdout.is_empty(),
        "completion expected to come back empty, but got: {output}"
    );
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

    work_dir.write_file("file", "A");
    work_dir
        .run_jj(["b", "c", "-r@", "mutable_bookmark"])
        .success();
    work_dir.run_jj(["describe", "-m", "mutable 1"]).success();

    work_dir.run_jj(["new", "immutable_bookmark"]).success();
    work_dir.write_file("file", "B");
    work_dir.run_jj(["describe", "-m", "mutable 2"]).success();
    work_dir.run_jj(["new", "@", "mutable_bookmark"]).success();
    work_dir
        .run_jj(["b", "c", "-r@", "conflicted_bookmark"])
        .success();
    work_dir.run_jj(["describe", "-m", "conflicted"]).success();

    work_dir.run_jj(["new", "mutable_bookmark"]).success();
    work_dir
        .run_jj(["describe", "-m", "working_copy"])
        .success();

    let work_dir = test_env.work_dir("repo");

    // There are _a lot_ of commands and arguments accepting revisions.
    // Let's not test all of them. Having at least one test per variation of
    // completion function should be sufficient.

    // complete all revisions
    let output = work_dir.complete_fish(["diff", "--from", ""]);
    insta::assert_snapshot!(output, @r"
    conflicted_bookmark	conflicted
    immutable_bookmark	immutable
    mutable_bookmark	mutable 1
    u	working_copy
    l	conflicted
    k	mutable 2
    y	mutable 1
    q	immutable
    r	remote_commit
    z	(no description set)
    remote_bookmark@origin	remote_commit
    alias_with_newline	    roots(
    siblings	@-+ ~@
    [EOF]
    ");

    // complete all revisions in a revset expression
    let output = work_dir.complete_fish(["log", "-r", ".."]);
    insta::assert_snapshot!(output, @r"
    ..conflicted_bookmark	conflicted
    ..immutable_bookmark	immutable
    ..mutable_bookmark	mutable 1
    ..u	working_copy
    ..l	conflicted
    ..k	mutable 2
    ..y	mutable 1
    ..q	immutable
    ..r	remote_commit
    ..z	(no description set)
    ..remote_bookmark@origin	remote_commit
    ..alias_with_newline	    roots(
    ..siblings	@-+ ~@
    [EOF]
    ");

    // complete only mutable revisions
    let output = work_dir.complete_fish(["squash", "--into", ""]);
    insta::assert_snapshot!(output, @r"
    conflicted_bookmark	conflicted
    mutable_bookmark	mutable 1
    u	working_copy
    l	conflicted
    k	mutable 2
    y	mutable 1
    r	remote_commit
    alias_with_newline	    roots(
    siblings	@-+ ~@
    [EOF]
    ");

    // complete only mutable revisions in a revset expression
    let output = work_dir.complete_fish(["abandon", "y::"]);
    insta::assert_snapshot!(output, @r"
    y::conflicted_bookmark	conflicted
    y::mutable_bookmark	mutable 1
    y::u	working_copy
    y::l	conflicted
    y::k	mutable 2
    y::y	mutable 1
    y::r	remote_commit
    y::alias_with_newline	    roots(
    y::siblings	@-+ ~@
    [EOF]
    ");

    // complete remote bookmarks in a revset expression
    let output = work_dir.complete_fish(["log", "-r", "remote_bookmark@"]);
    insta::assert_snapshot!(output, @r"
    remote_bookmark@origin	remote_commit
    [EOF]
    ");

    // complete conflicted revisions in a revset expression
    let output = work_dir.complete_fish(["resolve", "-r", ""]);
    insta::assert_snapshot!(output, @r"
    conflicted_bookmark	conflicted
    l	conflicted
    alias_with_newline	    roots(
    siblings	@-+ ~@
    [EOF]
    ");

    // complete args of the default command
    test_env.add_config("ui.default-command = 'log'");
    let output = work_dir.complete_fish(["-r", ""]);
    insta::assert_snapshot!(output, @r"
    conflicted_bookmark	conflicted
    immutable_bookmark	immutable
    mutable_bookmark	mutable 1
    u	working_copy
    l	conflicted
    k	mutable 2
    y	mutable 1
    q	immutable
    r	remote_commit
    z	(no description set)
    remote_bookmark@origin	remote_commit
    alias_with_newline	    roots(
    siblings	@-+ ~@
    [EOF]
    ");

    // Begin testing `jj git push --named`

    // The name of a bookmark does not get completed, since we want to create a new
    // bookmark
    let output = work_dir.complete_fish(["git", "push", "--named", ""]);
    insta::assert_snapshot!(output, @"");
    let output = work_dir.complete_fish(["git", "push", "--named", "a"]);
    insta::assert_snapshot!(output, @"");

    let output = work_dir.complete_fish(["git", "push", "--named", "a="]);
    insta::assert_snapshot!(output, @r"
    a=conflicted_bookmark	conflicted
    a=immutable_bookmark	immutable
    a=mutable_bookmark	mutable 1
    a=u	working_copy
    a=l	conflicted
    a=k	mutable 2
    a=y	mutable 1
    a=q	immutable
    a=r	remote_commit
    a=z	(no description set)
    a=remote_bookmark@origin	remote_commit
    a=alias_with_newline	    roots(
    a=siblings	@-+ ~@
    [EOF]
    ");

    let output = work_dir.complete_fish(["git", "push", "--named", "a=a"]);
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
    work_dir
        .run_jj(["describe", "-m", "description 5"])
        .success();

    let work_dir = test_env.work_dir("repo");

    let output = work_dir.complete_fish(["op", "show", ""]).success();
    let add_workspace_id = output
        .stdout
        .raw()
        .lines()
        .nth(5)
        .unwrap()
        .split('\t')
        .next()
        .unwrap();
    insta::assert_snapshot!(add_workspace_id, @"12f7cbba4278");

    let output = work_dir.complete_fish(["op", "show", "8"]);
    insta::assert_snapshot!(output, @r"
    8ed8c16786e6	(2001-02-03 08:05:11) describe commit 3725536d0ae06d69e46911258cee591dbdb66478
    8f47435a3990	(2001-02-03 08:05:07) add workspace 'default'
    [EOF]
    ");
    // make sure global --at-op flag is respected
    let output = work_dir.complete_fish(["--at-op", "8ed8c16786e6", "op", "show", "8"]);
    insta::assert_snapshot!(output, @r"
    8ed8c16786e6	(2001-02-03 08:05:11) describe commit 3725536d0ae06d69e46911258cee591dbdb66478
    8f47435a3990	(2001-02-03 08:05:07) add workspace 'default'
    [EOF]
    ");

    let output = work_dir.complete_fish(["--at-op", "8e"]);
    insta::assert_snapshot!(output, @r"
    8ed8c16786e6	(2001-02-03 08:05:11) describe commit 3725536d0ae06d69e46911258cee591dbdb66478
    [EOF]
    ");

    let output = work_dir.complete_fish(["op", "abandon", "8e"]);
    insta::assert_snapshot!(output, @r"
    8ed8c16786e6	(2001-02-03 08:05:11) describe commit 3725536d0ae06d69e46911258cee591dbdb66478
    [EOF]
    ");

    let output = work_dir.complete_fish(["op", "diff", "--op", "8e"]);
    insta::assert_snapshot!(output, @r"
    8ed8c16786e6	(2001-02-03 08:05:11) describe commit 3725536d0ae06d69e46911258cee591dbdb66478
    [EOF]
    ");
    let output = work_dir.complete_fish(["op", "diff", "--from", "8e"]);
    insta::assert_snapshot!(output, @r"
    8ed8c16786e6	(2001-02-03 08:05:11) describe commit 3725536d0ae06d69e46911258cee591dbdb66478
    [EOF]
    ");
    let output = work_dir.complete_fish(["op", "diff", "--to", "8e"]);
    insta::assert_snapshot!(output, @r"
    8ed8c16786e6	(2001-02-03 08:05:11) describe commit 3725536d0ae06d69e46911258cee591dbdb66478
    [EOF]
    ");

    let output = work_dir.complete_fish(["op", "restore", "8e"]);
    insta::assert_snapshot!(output, @r"
    8ed8c16786e6	(2001-02-03 08:05:11) describe commit 3725536d0ae06d69e46911258cee591dbdb66478
    [EOF]
    ");

    let output = work_dir.complete_fish(["op", "revert", "8e"]);
    insta::assert_snapshot!(output, @r"
    8ed8c16786e6	(2001-02-03 08:05:11) describe commit 3725536d0ae06d69e46911258cee591dbdb66478
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

    let main_dir = test_env.work_dir("main");

    let output = main_dir.complete_fish(["workspace", "forget", "def"]);
    insta::assert_snapshot!(output, @r"
    def-second	(no description set)
    default	initial
    [EOF]
    ");
}

#[test]
fn test_config() {
    let test_env = TestEnvironment::default();

    let output = test_env.complete_fish(["config", "get", "f"]);
    insta::assert_snapshot!(output, @r"
    fsmonitor.backend	Whether to use an external filesystem monitor, useful for large repos
    fsmonitor.watchman.register-snapshot-trigger	Whether to use triggers to monitor for changes in the background.
    [EOF]
    ");

    let output = test_env.complete_fish(["config", "list", "fs"]);
    insta::assert_snapshot!(output, @r"
    fsmonitor	External filesystem monitor settings, useful for large repos
    fsmonitor.backend	Whether to use an external filesystem monitor, useful for large repos
    fsmonitor.watchman
    fsmonitor.watchman.register-snapshot-trigger	Whether to use triggers to monitor for changes in the background.
    [EOF]
    ");

    let output = test_env.complete_fish(["log", "--config", "f"]);
    insta::assert_snapshot!(output, @r"
    fsmonitor.backend=	Whether to use an external filesystem monitor, useful for large repos
    fsmonitor.watchman.register-snapshot-trigger=	Whether to use triggers to monitor for changes in the background.
    [EOF]
    ");

    let output = test_env.complete_fish(["log", "--config", "ui.conflict-marker-style="]);
    insta::assert_snapshot!(output, @r"
    ui.conflict-marker-style=diff
    ui.conflict-marker-style=diff-experimental
    ui.conflict-marker-style=snapshot
    ui.conflict-marker-style=git
    [EOF]
    ");

    let output = test_env.complete_fish(["log", "--config", "ui.conflict-marker-style=g"]);
    insta::assert_snapshot!(output, @r"
    ui.conflict-marker-style=git
    [EOF]
    ");

    let output = test_env.complete_fish(["log", "--config", "git.abandon-unreachable-commits="]);
    insta::assert_snapshot!(output, @r"
    git.abandon-unreachable-commits=false
    git.abandon-unreachable-commits=true
    [EOF]
    ");
}

#[test]
fn test_template_alias() {
    let test_env = TestEnvironment::default();

    let output = test_env.complete_fish(["log", "-T", ""]);
    insta::assert_snapshot!(output, @r"
    builtin_config_list
    builtin_config_list_detailed
    builtin_draft_commit_description
    builtin_evolog_compact
    builtin_log_comfortable
    builtin_log_compact
    builtin_log_compact_full_description
    builtin_log_detailed
    builtin_log_node
    builtin_log_node_ascii
    builtin_log_oneline
    builtin_log_redacted
    builtin_op_log_comfortable
    builtin_op_log_compact
    builtin_op_log_node
    builtin_op_log_node_ascii
    builtin_op_log_oneline
    builtin_op_log_redacted
    commit_summary_separator
    default_commit_description
    description_placeholder
    email_placeholder
    empty_commit_marker
    git_format_patch_email_headers
    name_placeholder
    [EOF]
    ");
}

#[test]
fn test_merge_tools() {
    let mut test_env = TestEnvironment::default();
    // A tool without configured arguments is assumed to function as a diff
    // editor and formatter, but not as a merge editor.
    test_env.add_config("merge-tools.abracadabra={}");
    test_env.add_env_var("COMPLETE", "fish");
    let dir = test_env.env_root();

    let output = test_env.run_jj_in(dir, ["--", "jj", "diff", "--tool", ""]);
    // Includes `difft`, excludes merge tools like `mergiraf`
    insta::assert_snapshot!(output, @r"
    :summary
    :stat
    :types
    :name-only
    :git
    :color-words
    diffedit3
    diffedit3-ssh
    difft
    kdiff3
    meld
    meld-3
    vscode
    vscodium
    abracadabra
    [EOF]
    ");
    // Excludes `difft` and `mergiraf`
    let output = test_env.run_jj_in(dir, ["--", "jj", "diffedit", "--tool", ""]);
    insta::assert_snapshot!(output, @r"
    :builtin
    diffedit3
    diffedit3-ssh
    kdiff3
    meld
    meld-3
    vimdiff
    abracadabra
    [EOF]
    ");
    // Includes `mergiraf`, but not `difft` or `abracadabra`
    let output = test_env.run_jj_in(dir, ["--", "jj", "resolve", "--tool", ""]);
    insta::assert_snapshot!(output, @r"
    :builtin
    :ours
    :theirs
    kdiff3
    meld
    mergiraf
    smerge
    vimdiff
    vscode
    vscodium
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
            ("f_not_yet_copied", Some("copied\n")),
            ("f_not_yet_renamed", Some("renamed\n")),
            ("f_not_yet_renamed_2", Some("renamed_2\n")),
            ("f_not_yet_renamed_3", Some("renamed_3\n")),
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
            ("f_not_yet_copied", Some("copied\n\n")),
            ("f_not_yet_renamed", None),
            ("f_not_yet_renamed_2", None),
            ("f_not_yet_renamed_3", None),
            ("f_copied", Some("copied\n")),
            // f_not_yet_renamed < f_renamed
            ("f_renamed", Some("renamed\n")),
            // f_another_renamed_2 < f_not_yet_renamed_2
            ("f_another_renamed_2", Some("renamed_2\n")),
            ("f_deleted", None),
            ("f_added", Some("added\n")),
            ("f_dir/dir_file_1", Some("foo\n")),
            ("f_dir/dir_file_2", Some("foo\n")),
            ("f_dir/dir_file_3", Some("foo\n")),
            ("f_dir/f_renamed_3", Some("renamed_3\n")),
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
    @  wqnwkozp test.user@example.com 2001-02-03 08:05:20 working_copy 5e0882cf
    │  working_copy
    │  A f_added_2
    │  M f_modified
    ○  zsuskuln test.user@example.com 2001-02-03 08:05:11 second 5d65dc93
    │  second
    │  A f_added
    │  R {f_not_yet_renamed_2 => f_another_renamed_2}
    │  C {f_not_yet_copied => f_copied}
    │  D f_deleted
    │  A f_dir/dir_file_1
    │  A f_dir/dir_file_2
    │  A f_dir/dir_file_3
    │  R {f_not_yet_renamed_3 => f_dir/f_renamed_3}
    │  M f_modified
    │  M f_not_yet_copied
    │  R {f_not_yet_renamed => f_renamed}
    │ ×  royxmykx test.user@example.com 2001-02-03 08:05:14 conflicted a24d7569 conflict
    ├─╯  conflicted
    │    A f_added_2
    │    A f_dir/dir_file_1
    │    A f_dir/dir_file_2
    │    A f_dir/dir_file_3
    │    M f_modified
    ○  rlvkpnrz test.user@example.com 2001-02-03 08:05:09 first 221854a7
    │  first
    │  A f_deleted
    │  A f_modified
    │  A f_not_yet_copied
    │  A f_not_yet_renamed
    │  A f_not_yet_renamed_2
    │  A f_not_yet_renamed_3
    │  A f_unchanged
    │ ○  kpqxywon test.user@example.com 2001-02-03 08:05:18 interdiff_to 5e448a34
    ├─╯  interdiff_to
    │    A f_interdiff_only_to
    │    A f_interdiff_same
    │ ○  yostqsxw test.user@example.com 2001-02-03 08:05:16 interdiff_from 039b07b8
    ├─╯  interdiff_from
    │    A f_interdiff_only_from
    │    A f_interdiff_same
    ◆  zzzzzzzz root() 00000000
    [EOF]
    ");

    let work_dir = test_env.work_dir("repo");

    let output = work_dir.complete_fish(["file", "show", "f_"]);
    insta::assert_snapshot!(output, @r"
    f_added
    f_added_2
    f_another_renamed_2
    f_copied
    f_dir/
    f_modified
    f_not_yet_copied
    f_renamed
    f_unchanged
    [EOF]
    ");

    let output = work_dir.complete_fish(["file", "show", "./f_"]);
    insta::assert_snapshot!(output, @r"
    ./f_added
    ./f_added_2
    ./f_another_renamed_2
    ./f_copied
    ./f_dir/
    ./f_modified
    ./f_not_yet_copied
    ./f_renamed
    ./f_unchanged
    [EOF]
    ");

    let output = work_dir.complete_fish(["file", "show", "f_dir"]);
    insta::assert_snapshot!(output, @r"
    f_dir/
    [EOF]
    ");

    let output = work_dir.complete_fish(["file", "show", "f_dir/"]);
    insta::assert_snapshot!(output, @r"
    f_dir/dir_file_1
    f_dir/dir_file_2
    f_dir/dir_file_3
    f_dir/f_renamed_3
    [EOF]
    ");

    let output = work_dir.complete_fish(["file", "show", "f_dir/../"]);
    insta::assert_snapshot!(output, @r"
    f_dir/../f_added
    f_dir/../f_added_2
    f_dir/../f_another_renamed_2
    f_dir/../f_copied
    f_dir/../f_dir/
    f_dir/../f_modified
    f_dir/../f_not_yet_copied
    f_dir/../f_renamed
    f_dir/../f_unchanged
    [EOF]
    ");

    let output = work_dir.complete_fish(["file", "show", "f_dir/../f_dir/"]);
    insta::assert_snapshot!(output, @r"
    f_dir/../f_dir/dir_file_1
    f_dir/../f_dir/dir_file_2
    f_dir/../f_dir/dir_file_3
    f_dir/../f_dir/f_renamed_3
    [EOF]
    ");

    let subdir = work_dir.dir("f_dir");
    let output = subdir.complete_fish(["file", "show", "dir_"]);
    insta::assert_snapshot!(output, @r"
    dir_file_1
    dir_file_2
    dir_file_3
    [EOF]
    ");

    let output = subdir.complete_fish(["file", "show", "./"]);
    insta::assert_snapshot!(output, @r"
    ./dir_file_1
    ./dir_file_2
    ./dir_file_3
    ./f_renamed_3
    [EOF]
    ");

    let output = subdir.complete_fish(["file", "show", "../"]);
    insta::assert_snapshot!(output, @r"
    ../f_added
    ../f_added_2
    ../f_another_renamed_2
    ../f_copied
    ../f_modified
    ../f_not_yet_copied
    ../f_renamed
    ../f_unchanged
    [EOF]
    ");

    let output = work_dir.complete_fish(["file", "annotate", "-r@-", "f_"]);
    insta::assert_snapshot!(output, @r"
    f_added
    f_another_renamed_2
    f_copied
    f_dir/
    f_modified
    f_not_yet_copied
    f_renamed
    f_unchanged
    [EOF]
    ");

    let output = work_dir.complete_fish(["diff", "-r", "@-", "f_"]);
    insta::assert_snapshot!(output, @r"
    f_added	Added
    f_another_renamed_2	Renamed
    f_copied	Copied
    f_deleted	Deleted
    f_dir/
    f_modified	Modified
    f_not_yet_copied	Modified
    f_not_yet_renamed	Renamed
    f_not_yet_renamed_2	Renamed
    f_not_yet_renamed_3	Renamed
    f_renamed	Renamed
    [EOF]
    ");

    let output = work_dir.complete_fish(["diff", "-r", "@-", "f_dir/../"]);
    insta::assert_snapshot!(output, @r"
    f_dir/../f_added	Added
    f_dir/../f_another_renamed_2	Renamed
    f_dir/../f_copied	Copied
    f_dir/../f_deleted	Deleted
    f_dir/../f_dir/
    f_dir/../f_modified	Modified
    f_dir/../f_not_yet_copied	Modified
    f_dir/../f_not_yet_renamed	Renamed
    f_dir/../f_not_yet_renamed_2	Renamed
    f_dir/../f_not_yet_renamed_3	Renamed
    f_dir/../f_renamed	Renamed
    [EOF]
    ");

    // Given that the path prefix uses the main separator (e.g. `\` on Windows),
    // check that the completion continues to use the same separator.
    // The assertion maps the main separator to some arbitrary fictitious separator
    // (`→`) which is not used by real OSes (yet) to check that the main separator
    // is preserved on platforms where it differs from `/`.
    let output = work_dir.complete_fish([
        "diff",
        "-r",
        "@-",
        &format!("f_dir{}", std::path::MAIN_SEPARATOR),
    ]);
    insta::assert_snapshot!(
        output.normalize_stdout_with(|s| s.replace(std::path::MAIN_SEPARATOR, "→")),
        @r"
    f_dir→dir_file_1	Added
    f_dir→dir_file_2	Added
    f_dir→dir_file_3	Added
    f_dir→f_renamed_3	Renamed
    [EOF]
    ");

    let output = work_dir.complete_fish(["diff", "--from", "root()", "--to", "@-", "f_"]);
    insta::assert_snapshot!(output, @r"
    f_added	Added
    f_another_renamed_2	Added
    f_copied	Added
    f_dir/
    f_modified	Added
    f_not_yet_copied	Added
    f_renamed	Added
    f_unchanged	Added
    [EOF]
    ");

    let output = work_dir.complete_fish(["restore", "-c", "@-", "f_"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    f_added	Added
    f_another_renamed_2	Renamed
    f_copied	Copied
    f_deleted	Deleted
    f_dir/
    f_modified	Modified
    f_not_yet_copied	Modified
    f_not_yet_renamed	Renamed
    f_not_yet_renamed_2	Renamed
    f_not_yet_renamed_3	Renamed
    f_renamed	Renamed
    [EOF]
    ");

    let output = work_dir.complete_fish(["restore", "--from", "root()", "--to", "@-", "f_"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    f_added	Added
    f_another_renamed_2	Added
    f_copied	Added
    f_dir/
    f_modified	Added
    f_not_yet_copied	Added
    f_renamed	Added
    f_unchanged	Added
    [EOF]
    ");

    // interdiff has a different behavior with --from and --to flags
    let output = work_dir.complete_fish([
        "interdiff",
        "--to=interdiff_to",
        "--from=interdiff_from",
        "f_",
    ]);
    insta::assert_snapshot!(output, @r"
    f_interdiff_only_from	Added
    f_interdiff_same	Added
    f_interdiff_only_to	Added
    f_interdiff_same	Added
    [EOF]
    ");

    // squash has a different behavior with --from and --to flags
    let output = work_dir.complete_fish(["squash", "-f=first", "f_"]);
    insta::assert_snapshot!(output, @r"
    f_deleted	Added
    f_modified	Added
    f_not_yet_copied	Added
    f_not_yet_renamed	Added
    f_not_yet_renamed_2	Added
    f_not_yet_renamed_3	Added
    f_unchanged	Added
    [EOF]
    ");

    let output = work_dir.complete_fish(["resolve", "-r=conflicted", "f_"]);
    insta::assert_snapshot!(output, @r"
    f_dir/
    f_modified
    [EOF]
    ");

    let output = work_dir.complete_fish(["file", "list", "-r=first", "f_"]);
    insta::assert_snapshot!(output, @r"
    f_deleted
    f_modified
    f_not_yet_copied
    f_not_yet_renamed
    f_not_yet_renamed_2
    f_not_yet_renamed_3
    f_unchanged
    [EOF]
    ");

    let output = work_dir.complete_fish(["log", "f_"]);
    insta::assert_snapshot!(output, @r"
    f_added
    f_added_2
    f_another_renamed_2
    f_copied
    f_dir/
    f_modified
    f_not_yet_copied
    f_renamed
    f_unchanged
    [EOF]
    ");

    let output = work_dir.complete_fish(["log", "-r=first", "--revisions", "conflicted", "f_"]);
    insta::assert_snapshot!(output, @r"
    f_added_2
    f_deleted
    f_dir/
    f_modified
    f_not_yet_copied
    f_not_yet_renamed
    f_not_yet_renamed_2
    f_not_yet_renamed_3
    f_unchanged
    [EOF]
    ");

    let outside_repo = test_env.env_root();
    let output = test_env.work_dir(outside_repo).complete_fish(["log", "f_"]);
    insta::assert_snapshot!(output, @"");

    let output = work_dir.complete_fish(["absorb", "f_"]);
    insta::assert_snapshot!(output, @r"
    f_added_2	Added
    f_modified	Modified
    [EOF]
    ");

    let output = work_dir.complete_fish(["absorb", "-f=conflicted", "f_"]);
    insta::assert_snapshot!(output, @r"
    f_added_2	Added
    f_dir/
    f_modified	Modified
    [EOF]
    ");
}

#[test]
fn test_command_alias_with_exec() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    test_env.add_config(r#"aliases.my-script = ["util", "exec", "--", "my-jj-script"]"#);

    work_dir.write_file("file1", "contents");
    work_dir.write_file("file2", "contents");
    work_dir.create_dir("folder");
    work_dir.write_file("folder/subfile", "contents");

    let output = work_dir.complete_fish(["my-script", "f"]);
    insta::assert_snapshot!(output.normalize_backslash(), @r"
    file1
    file2
    folder/
    [EOF]
    ");
}
