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
use indoc::indoc;
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
        .run_jj(["bookmark", "track", "--remote=origin", "*-tracked"])
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

    insta::assert_snapshot!(work_dir.run_jj(["bookmark", "list", "--all"]), @"
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
    insta::assert_snapshot!(output, @"
    aaa-local	x
    aaa-tracked	x
    bbb-local	x
    bbb-tracked	x
    --overwrite-existing	Allow renaming even if the new bookmark name already exists
    --help	Print help (see more with '--help')
    --repository	Path to repository to operate on
    --ignore-working-copy	Don't snapshot the working copy, and don't update it
    --no-integrate-operation	Run the command as usual but don't integrate any operations
    --ignore-immutable	Allow rewriting immutable commits
    --at-operation	Operation to load the repo at
    --debug	Enable debug logging
    --color	When to colorize output
    --quiet	Silence non-primary command output
    --no-pager	Disable the pager
    --config	Additional configuration options (can be repeated)
    --config-file	Additional configuration files (can be repeated)
    [EOF]
    ");

    let output = work_dir.complete_fish(["bookmark", "rename", "a"]);
    insta::assert_snapshot!(output, @"
    aaa-local	x
    aaa-tracked	x
    [EOF]
    ");

    let output = work_dir.complete_fish(["bookmark", "delete", "a"]);
    insta::assert_snapshot!(output, @"
    aaa-local	x
    aaa-tracked	x
    [EOF]
    ");

    let output = work_dir.complete_fish(["bookmark", "forget", "a"]);
    insta::assert_snapshot!(output, @"
    aaa-local	x
    aaa-tracked	x
    aaa-untracked
    [EOF]
    ");

    let output = work_dir.complete_fish(["bookmark", "list", "--bookmark", "a"]);
    insta::assert_snapshot!(output, @"
    aaa-local	x
    aaa-tracked	x
    aaa-untracked
    [EOF]
    ");

    let output = work_dir.complete_fish(["bookmark", "move", "a"]);
    insta::assert_snapshot!(output, @"
    aaa-local	x
    aaa-tracked	x
    [EOF]
    ");

    let output = work_dir.complete_fish(["bookmark", "set", "a"]);
    insta::assert_snapshot!(output, @"
    aaa-local	x
    aaa-tracked	x
    [EOF]
    ");

    let output = work_dir.complete_fish(["bookmark", "track", "a"]);
    insta::assert_snapshot!(output, @"
    aaa-local	 x
    aaa-untracked	 x
    [EOF]
    ");

    let output = work_dir.complete_fish(["bookmark", "untrack", "a"]);
    insta::assert_snapshot!(output, @"
    aaa-tracked	x
    [EOF]
    ");

    // TODO: Make it so this only lists untracked remotes
    let output = work_dir.complete_fish(["bookmark", "track", "a", "--remote", ""]);
    insta::assert_snapshot!(output, @"
    origin
    upstream
    [EOF]
    ");

    // TODO: Make it so this only lists tracked remotes
    let output = work_dir.complete_fish(["bookmark", "untrack", "a", "--remote", ""]);
    insta::assert_snapshot!(output, @"
    origin
    upstream
    [EOF]
    ");

    let output = work_dir.complete_fish(["git", "push", "-b", "a"]);
    insta::assert_snapshot!(output, @"
    aaa-local	x
    aaa-tracked	x
    [EOF]
    ");

    let output = work_dir.complete_fish(["git", "fetch", "-b", "a"]);
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
    aaa-local	commit1
    [EOF]
    ");

    let output = work_dir.complete_fish(["tag", "delete", "b"]);
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
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
            insta::assert_snapshot!(output, @"
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
            insta::assert_snapshot!(output, @"
            aaa	(no description set)
            [EOF]
            ");
        }
        _ => unimplemented!("unexpected shell '{shell}'"),
    }

    let output = work_dir.complete_at(shell, 2, ["rlog", "--rev"]);
    match shell {
        Shell::Bash => {
            insta::assert_snapshot!(output, @"
            --revision
            --reversed[EOF]
            ");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @"
            --revision:Which revisions to show
            --reversed:Show revisions in the opposite order (older revisions first)[EOF]
            ");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @"
            --revision	Which revisions to show
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
    insta::assert_snapshot!(output, @"
    complete --keep-order --exclusive --command jj --arguments ..
    [EOF]
    ");
    let output = test_env.run_jj_in(".", ["--"]);
    insta::assert_snapshot!(output, @"
    complete --keep-order --exclusive --command jj --arguments ..
    [EOF]
    ");
}

#[test]
fn test_bash_completion_registration() {
    let mut test_env = TestEnvironment::default();
    test_env.add_env_var("COMPLETE", "bash");
    let mut insta_settings = insta::Settings::clone_current();
    insta_settings.add_filter(r#"(?m)^\s+.* -- "\$\{words\[@\]\}" \\$"#, r"    .. \"); // omit path to jj binary
    let _guard = insta_settings.bind_to_scope();

    let output = test_env.run_jj_in(".", [""; 0]);
    insta::assert_snapshot!(output, @r#"
    # Bash splits the completed word at COMP_WORDBREAKS characters (the "@" in
    # "main@origin", the ":" in "main::", ...), which would reach the completion
    # as separate words otherwise. Rejoin them here and rebase the returned
    # candidates on the part of the word that bash will actually replace. Like
    # git-completion.bash, but unlike editing COMP_WORDBREAKS, this leaves the
    # rest of the shell session untouched.
    _clap_complete_jj_reassembled() {
        local line=$COMP_LINE
        local -a words=()
        local cword=0
        local j=0 i
        for i in "${!COMP_WORDS[@]}"; do
            if ((i > 0)) && [[ -n $line && $line == [[:space:]]* ]]; then
                j=$((j + 1))
            fi
            words[j]=${words[j]-}${COMP_WORDS[i]}
            line=${line#*"${COMP_WORDS[i]}"}
            # Skip a closing quote so that quoted words are not glued to the
            # following word in bash versions that strip quotes from COMP_WORDS.
            # Words that mix quotes with unquoted text around word break
            # characters may still be reassembled incorrectly.
            line=${line#[\'\"]}
            if ((i == COMP_CWORD)); then
                cword=$j
            fi
        done

        local cur=$2
        local word=${words[cword]}
        if [[ -n $cur && $word != *"$cur" ]]; then
            # The cursor is in the middle of a word; leave the word as bash
            # split it and let the completion handle just the typed part.
            _clap_complete_jj "$1" "$cur"
            return
        fi

        # Bash replaces only the part of the word after the last COMP_WORDBREAKS
        # character (bash's $2), or inserts at the cursor when $2 is empty, so
        # the typed part of the word has to be stripped from the candidates
        # below. Candidates that strip to the empty string are for words that
        # are already fully typed.
        local keep=$word
        if [[ -n $cur ]]; then
            keep=${word%"$cur"}
        fi

        local -a saved_words=("${COMP_WORDS[@]}")
        local saved_cword=$COMP_CWORD
        COMP_WORDS=("${words[@]}")
        COMP_CWORD=$cword
        _clap_complete_jj "${words[0]}" "$word"
        COMP_WORDS=("${saved_words[@]}")
        COMP_CWORD=$saved_cword

        local -a adjusted=()
        local candidate
        if [[ ${#COMPREPLY[@]} -gt 0 ]]; then
            for candidate in "${COMPREPLY[@]}"; do
                if [[ -n $keep && $candidate == "$keep"* ]]; then
                    candidate=${candidate#"$keep"}
                fi
                # Drop candidates that are already fully typed, so that bash
                # doesn't replace the typed part of the word with them.
                if [[ -n $candidate ]]; then
                    adjusted+=("$candidate")
                fi
            done
        fi
        COMPREPLY=()
        if [[ ${#adjusted[@]} -gt 0 ]]; then
            COMPREPLY=("${adjusted[@]}")
        fi
    }


    _clap_complete_jj() {
        local IFS=$'\013'
        local _CLAP_COMPLETE_INDEX=${COMP_CWORD}
        local _CLAP_COMPLETE_COMP_TYPE=${COMP_TYPE}
        if compopt +o nospace 2> /dev/null; then
            local _CLAP_COMPLETE_SPACE=false
        else
            local _CLAP_COMPLETE_SPACE=true
        fi
        local words=("${COMP_WORDS[@]}")
        if [[ "${BASH_VERSINFO[0]}" -ge 4 ]]; then
            words[COMP_CWORD]="$2"
        fi
        COMPREPLY=( $( \
            _CLAP_IFS="$IFS" \
            _CLAP_COMPLETE_INDEX="$_CLAP_COMPLETE_INDEX" \
            _CLAP_COMPLETE_COMP_TYPE="$_CLAP_COMPLETE_COMP_TYPE" \
            _CLAP_COMPLETE_SPACE="$_CLAP_COMPLETE_SPACE" \
            COMPLETE="bash" \
        .. \
        ) )
        if [[ $? != 0 ]]; then
            unset COMPREPLY
        elif [[ $_CLAP_COMPLETE_SPACE == false ]] && [[ "${COMPREPLY-}" =~ [=/:]$ ]]; then
            compopt -o nospace
        fi
    }
    if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
        complete -o nospace -o bashdefault -o nosort -F _clap_complete_jj_reassembled jj
    else
        complete -o nospace -o bashdefault -F _clap_complete_jj_reassembled jj
    fi


    [EOF]
    "#);
}

#[test]
fn test_bash_completion_word_reassembly() {
    let mut test_env = TestEnvironment::default();
    test_env.add_env_var("COMPLETE", "bash");
    let registration = test_env.run_jj_in(".", [""; 0]).stdout;

    // Drive the reassembly wrapper directly with the COMP_* values bash
    // would set, with the clap-generated completion function stubbed out to
    // record its inputs and return fixed candidates. Requires a usable bash;
    // skipped where there is none.
    let script = format!(
        r#"{registration}
_clap_complete_jj() {{
    printf 'inner cword=%s cur=<%s> words=' "$COMP_CWORD" "$2"
    local w
    for w in "${{COMP_WORDS[@]}}"; do printf '<%s>' "$w"; done
    printf '\n'
    COMPREPLY=("${{stub_replies[@]}}")
}}
run() {{
    COMP_LINE=$1
    COMP_POINT=${{#1}}
    COMP_CWORD=$2
    local cur=$3
    stub_replies=($4)
    shift 4
    COMP_WORDS=("$@")
    COMP_WORDBREAKS=$' \t\n"\'@><=;|&(:'
    _clap_complete_jj_reassembled jj "$cur"
    printf 'replies='
    local c
    for c in "${{COMPREPLY[@]}}"; do printf '<%s>' "$c"; done
    printf ' words='
    local w
    for w in "${{COMP_WORDS[@]}}"; do printf '<%s>' "$w"; done
    printf '\n'
}}
run 'jj rebase -d master@' 4 '@' 'master@origin' jj rebase -d master '@'
run 'jj rebase -d master@o' 5 '@o' 'master@origin' jj rebase -d master '@' o
run 'jj rebase -d master@origin' 5 'origin' 'master@origin' jj rebase -d master '@' origin
run 'jj git push --named a=master@o' 7 'o' 'a=master@origin' jj git push --named a = master '@' o
run 'jj git push --named a=master' 6 'master' 'a=master a=master@origin' jj git push --named a = master
run 'jj log -r master::' 4 '' 'master::master master::t' jj log -r master '::'
run 'jj log -r master@ ' 5 '' 'master@origin' jj log -r master '@' ''
run 'jj rebase -d master@' 5 '' 'master@origin' jj rebase -d master '@' ''
run "jj log -r 'foo bar' mas" 4 'mas' 'master' jj log -r "'foo bar'" mas
run 'jj log master' 2 'mas' 'master' jj log master
run 'jj new @' 2 '@' 'master@origin' jj new '@'
"#
    );
    // Skip where bash exists but can't run a script. The Windows CI runners
    // resolve `bash` to the WSL launcher, which bails out unless a
    // distribution is installed.
    let bash_works = std::process::Command::new("bash")
        .args(["-c", "exit 0"])
        .output()
        .is_ok_and(|output| output.status.success());
    if !bash_works {
        return;
    }

    let script_path = test_env.env_root().join("completion.sh");
    std::fs::write(&script_path, script).unwrap();
    let output = std::process::Command::new("bash")
        .arg(script_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "bash script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    insta::assert_snapshot!(String::from_utf8(output.stdout).unwrap(), @"
    inner cword=3 cur=<master@> words=<jj><rebase><-d><master@>
    replies=<@origin> words=<jj><rebase><-d><master><@>
    inner cword=3 cur=<master@o> words=<jj><rebase><-d><master@o>
    replies=<@origin> words=<jj><rebase><-d><master><@><o>
    inner cword=3 cur=<master@origin> words=<jj><rebase><-d><master@origin>
    replies=<origin> words=<jj><rebase><-d><master><@><origin>
    inner cword=4 cur=<a=master@o> words=<jj><git><push><--named><a=master@o>
    replies=<origin> words=<jj><git><push><--named><a><=><master><@><o>
    inner cword=4 cur=<a=master> words=<jj><git><push><--named><a=master>
    replies=<master><master@origin> words=<jj><git><push><--named><a><=><master>
    inner cword=3 cur=<master::> words=<jj><log><-r><master::>
    replies=<master><t> words=<jj><log><-r><master><::>
    inner cword=4 cur=<> words=<jj><log><-r><master@><>
    replies=<master@origin> words=<jj><log><-r><master><@><>
    inner cword=3 cur=<master@> words=<jj><rebase><-d><master@>
    replies=<origin> words=<jj><rebase><-d><master><@><>
    inner cword=4 cur=<mas> words=<jj><log><-r><'foo bar'><mas>
    replies=<master> words=<jj><log><-r><'foo bar'><mas>
    inner cword=2 cur=<mas> words=<jj><log><master>
    replies=<master> words=<jj><log><master>
    inner cword=2 cur=<@> words=<jj><new><@>
    replies=<master@origin> words=<jj><new><@>
    ");
}

#[test]
fn test_bad_complete_env() {
    let mut test_env = TestEnvironment::default();

    test_env.add_env_var("COMPLETE", "badshell");
    let output = test_env.run_jj_in(".", [""; 0]);
    insta::assert_snapshot!(output, @"
    ------- stderr -------
    error: unknown shell `badshell`, expected one of `bash`, `elvish`, `fish`, `powershell`, `zsh`[EOF]
    [exit status: 2]
    ");

    // Empty value of COMPLETE is ignored as is the value of "0".  This could
    // change if `clap` changes the way it interprets an empty COMPLETE env var.
    //
    // In other words, jj runs normally instead of returning completions. We get
    // an error because the default jj command needs to be run in a jj repo.
    test_env.add_env_var("COMPLETE", "");
    let output = test_env.run_jj_in(".", [""; 0]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Hint: Use `jj -h` for a list of available commands.
    Run `jj config set --user ui.default-command log` to disable this message.
    Error: There is no jj repo in "."
    [EOF]
    [exit status: 1]
    "#);
    // Same thing (normal execution) happens for a sub-command and "0"
    test_env.add_env_var("COMPLETE", "0");
    let output = test_env.run_jj_in(".", ["config", "list", "user.name"]);
    insta::assert_snapshot!(output, @r#"
    user.name = "Test User"
    [EOF]
    "#);
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
            insta::assert_snapshot!(output, @"
            --revision
            --limit
            [EOF]
            ");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @"
            --revision:Which revisions to show
            --limit:Limit number of revisions to show
            [EOF]
            ");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @"
            --revision	Which revisions to show
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
            insta::assert_snapshot!(output, @"
            --retain-bookmarks
            --restore-descendants
            [EOF]
            ");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @"
            --retain-bookmarks:Do not delete bookmarks pointing to the revisions to abandon
            --restore-descendants:Do not modify the content of the children of the abandoned commits
            [EOF]
            ");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @"
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
            insta::assert_snapshot!(output, @"
            --from
            --to
            [EOF]
            ");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @"
            --from:Move bookmarks from the given revisions
            --to:Move bookmarks to this revision
            [EOF]
            ");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @"
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
            insta::assert_snapshot!(output, @"
            abandon
            absorb
            [EOF]
            ");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @"
            abandon:Abandon a revision
            absorb:Move changes from a revision into the stack of mutable revisions
            [EOF]
            ");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @"
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
            insta::assert_snapshot!(output, @"
            abandon
            absorb
            [EOF]
            ");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @"
            abandon:Abandon a revision
            absorb:Move changes from a revision into the stack of mutable revisions
            [EOF]
            ");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @"
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
            insta::assert_snapshot!(output, @"
            bisect:Find a bad revision by bisection
            bookmark:Manage bookmarks [default alias: b][EOF]
            ");
        }
        Shell::Bash => {
            insta::assert_snapshot!(output, @"
            bisect
            bookmark[EOF]
            ");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @"
            bisect	Find a bad revision by bisection
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
            insta::assert_snapshot!(output, @"
            abandon	Abandon a revision
            [EOF]
            ");
        }
        _ => unimplemented!("unexpected shell '{shell}'"),
    }
}

#[test]
fn test_command_completion_short_name() {
    let test_env = TestEnvironment::default();

    // Short command names should be omitted
    let output = test_env.complete_fish(["config", ""]);
    insta::assert_snapshot!(output, @"
    edit	Start an editor on a jj config file
    gc	Find and optionally delete repo-level config directories whose repo path no longer exists
    get	Get the value of a given config option.
    list	List variables set in config files, along with their values
    path	Print the paths to the config files
    set	Update a config file to set the given option to a given value
    unset	Update a config file to unset the given option
    --repository	Path to repository to operate on
    --ignore-working-copy	Don't snapshot the working copy, and don't update it
    --no-integrate-operation	Run the command as usual but don't integrate any operations
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

    // Long command name should be suggested
    let output = test_env.complete_fish(["config", "e"]);
    insta::assert_snapshot!(output, @"
    edit	Start an editor on a jj config file
    [EOF]
    ");

    // Command arguments should be suggested for the short name
    let output = test_env.complete_fish(["config", "e", "--u"]);
    insta::assert_snapshot!(output, @"
    --user	Target the user-level config
    [EOF]
    ");

    // Command arguments should be suggested for the long name
    let output = test_env.complete_fish(["config", "edit", "--u"]);
    insta::assert_snapshot!(output, @"
    --user	Target the user-level config
    [EOF]
    ");
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
    insta::assert_snapshot!(output, @"
    origin
    [EOF]
    ");

    let output = test_env.complete_fish(["git", "remote", "rename", "o"]);
    insta::assert_snapshot!(output, @"
    origin
    [EOF]
    ");

    let output = test_env.complete_fish(["git", "remote", "set-url", "o"]);
    insta::assert_snapshot!(output, @"
    origin
    [EOF]
    ");

    let output = test_env.complete_fish(["git", "push", "--remote", "o"]);
    insta::assert_snapshot!(output, @"
    origin
    [EOF]
    ");

    let output = test_env.complete_fish(["git", "fetch", "--remote", "o"]);
    insta::assert_snapshot!(output, @"
    origin
    [EOF]
    ");

    let output = test_env.complete_fish(["bookmark", "list", "--remote", "o"]);
    insta::assert_snapshot!(output, @"
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

    // completion of incomplete alias
    let output = work_dir.complete_at(shell, 1, ["user-al"]);
    match shell {
        Shell::Bash => {
            insta::assert_snapshot!(output, @"user-alias[EOF]");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @"user-alias[EOF]");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @"
            user-alias
            [EOF]
            ");
        }
        _ => unimplemented!("unexpected shell '{shell}'"),
    }

    // completion of complete alias confirms it
    let output = work_dir.complete_at(shell, 1, ["user-alias"]);
    match shell {
        Shell::Bash => {
            insta::assert_snapshot!(output, @"user-alias[EOF]");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @"user-alias[EOF]");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @"
            user-alias
            [EOF]
            ");
        }
        _ => unimplemented!("unexpected shell '{shell}'"),
    }

    // completion after alias is based on resolved alias
    let output = work_dir
        .complete_at(shell, 2, ["user-alias", ""])
        .take_stdout_n_lines(2);
    match shell {
        Shell::Bash => {
            insta::assert_snapshot!(output, @"
            advance
            create
            [EOF]
            ");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @"
            advance:Advance the closest bookmarks to a target revision
            create:Create a new bookmark
            [EOF]
            ");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @"
            advance	Advance the closest bookmarks to a target revision
            create	Create a new bookmark
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
            insta::assert_snapshot!(output, @"
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

    // Disable an alias.
    work_dir
        .run_jj([
            "config",
            "set",
            "--repo",
            "aliases.user-alias.enabled",
            "false",
        ])
        .success();
    let output = work_dir.complete_at(shell, 1, ["user-"]);
    match shell {
        Shell::Bash => {
            insta::assert_snapshot!(output, @"");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @"");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @"");
        }
        _ => unimplemented!("unexpected shell '{shell}'"),
    }

    // Unknown alias is not completed.
    let output = work_dir.complete_at(shell, 1, ["unknown"]);
    match shell {
        Shell::Bash => {
            insta::assert_snapshot!(output, @"");
        }
        Shell::Zsh => {
            insta::assert_snapshot!(output, @"");
        }
        Shell::Fish => {
            insta::assert_snapshot!(output, @"");
        }
        _ => unimplemented!("unexpected shell '{shell}'"),
    }
}

#[test]
fn test_alias_descriptions_in_completions() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    // alias with a description
    test_env.add_config(indoc! {r#"
        [aliases]
        'compact-log'.doc = 'Show the log in a compact format'
        'compact-log'.definition = ["log", "--no-graph"]
    "#});

    // fish displays help text after a tab
    let output = work_dir.complete_fish(["compact-l"]);
    insta::assert_snapshot!(output, @"
    compact-log	Show the log in a compact format
    [EOF]
    ");

    // alias with multi-line string description
    test_env.add_config(indoc! {r#"
        [aliases]
        'status-short'.doc = 'Show the status of the working copy in short format'
        'status-short'.definition = ["status", "--format=summary"]
    "#});

    let output = work_dir.complete_fish(["status-shor"]);
    insta::assert_snapshot!(output, @"
    status-short	Show the status of the working copy in short format
    [EOF]
    ");

    // alias without a .doc property should have no description
    test_env.add_config(indoc! {r#"
        [aliases]
        plain-alias = ["status"]
    "#});

    let output = work_dir.complete_fish(["plain-alia"]);
    insta::assert_snapshot!(output, @"
    plain-alias
    [EOF]
    ");

    // revset alias with doc
    test_env.add_config(indoc! {r#"
        [revset-aliases]
        'mine'.doc = 'All my work'
        'mine'.definition = "author(foo)"
    "#});

    let output = work_dir.complete_fish(["log", "-r", "min"]);
    insta::assert_snapshot!(output, @"
    mine	All my work
    [EOF]
    ");

    // template alias with doc
    test_env.add_config(indoc! {r#"
        [template-aliases]
        'sh'.doc = 'Short hash'
        'sh'.definition = "commit_id.short()"
    "#});

    let output = work_dir.complete_fish(["log", "-T", "s"]);
    insta::assert_snapshot!(output, @"
    sh	Short hash
    [EOF]
    ");

    // fileset alias with doc
    test_env.add_config(indoc! {r#"
        [fileset-aliases]
        'LOCK'.doc = 'Lockfiles'
        'LOCK'.definition = '**/Cargo.lock'
    "#});

    let output = work_dir.complete_fish(["log", "-r", "all()", "L"]);
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
        .run_jj(["bookmark", "create", "-r@", "remote_bookmark"])
        .success();
    origin_dir
        .run_jj(["commit", "-m", "remote_commit"])
        .success();
    origin_dir
        .run_jj(["bookmark", "create", "-r@", "deleted_bookmark"])
        .success();
    origin_dir
        .run_jj(["commit", "-m", "deleted_remote_commit"])
        .success();
    origin_dir.run_jj(["git", "export"]).success();
    work_dir.run_jj(["git", "fetch"]).success();

    work_dir
        .run_jj(["bookmark", "track", "deleted_bookmark"])
        .success();
    work_dir
        .run_jj(["bookmark", "delete", "deleted_bookmark"])
        .success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "immutable_bookmark"])
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
    work_dir.run_jj(["describe", "-m", "mutable 1"]).success();
    work_dir.run_jj(["describe", "-m", "mutable 2"]).success();
    // Create divergent change
    work_dir
        .run_jj([
            "bookmark",
            "create",
            "-r=at_operation(@-, @)",
            "mutable_bookmark",
        ])
        .success();

    work_dir.run_jj(["new", "immutable_bookmark"]).success();
    work_dir.write_file("file", "B");
    work_dir.run_jj(["describe", "-m", "mutable 3"]).success();
    work_dir.run_jj(["new", "@", "mutable_bookmark"]).success();
    work_dir
        .run_jj(["bookmark", "create", "-r@", "conflicted_bookmark"])
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
    insta::assert_snapshot!(output, @"
    conflicted_bookmark	conflicted
    deleted_bookmark	(deleted bookmark)
    immutable_bookmark	immutable
    mutable_bookmark	mutable 1
    wv	working_copy
    x	conflicted
    u	mutable 3
    wq/0	mutable 2
    wq/1	mutable 1
    q	immutable
    m	deleted_remote_commit
    r	remote_commit
    z	(no description set)
    deleted_bookmark@origin	deleted_remote_commit
    remote_bookmark@origin	remote_commit
    alias_with_newline	    roots(
    siblings	@-+ ~@
    [EOF]
    ");

    // complete all revisions in a revset expression
    let output = work_dir.complete_fish(["log", "-r", ".."]);
    insta::assert_snapshot!(output, @"
    ..conflicted_bookmark	conflicted
    ..deleted_bookmark	(deleted bookmark)
    ..immutable_bookmark	immutable
    ..mutable_bookmark	mutable 1
    ..wv	working_copy
    ..x	conflicted
    ..u	mutable 3
    ..wq/0	mutable 2
    ..wq/1	mutable 1
    ..q	immutable
    ..m	deleted_remote_commit
    ..r	remote_commit
    ..z	(no description set)
    ..deleted_bookmark@origin	deleted_remote_commit
    ..remote_bookmark@origin	remote_commit
    ..alias_with_newline	    roots(
    ..siblings	@-+ ~@
    [EOF]
    ");

    // complete only mutable revisions
    let output = work_dir.complete_fish(["squash", "--into", ""]);
    insta::assert_snapshot!(output, @"
    conflicted_bookmark	conflicted
    mutable_bookmark	mutable 1
    wv	working_copy
    x	conflicted
    u	mutable 3
    wq/0	mutable 2
    wq/1	mutable 1
    m	deleted_remote_commit
    r	remote_commit
    alias_with_newline	    roots(
    siblings	@-+ ~@
    [EOF]
    ");

    // complete only mutable revisions in a revset expression
    let output = work_dir.complete_fish(["abandon", "y::"]);
    insta::assert_snapshot!(output, @"
    y::conflicted_bookmark	conflicted
    y::mutable_bookmark	mutable 1
    y::wv	working_copy
    y::x	conflicted
    y::u	mutable 3
    y::wq/0	mutable 2
    y::wq/1	mutable 1
    y::m	deleted_remote_commit
    y::r	remote_commit
    y::alias_with_newline	    roots(
    y::siblings	@-+ ~@
    [EOF]
    ");

    // same, but through the bash code path, which strips help text
    let output = work_dir.complete_at(Shell::Bash, 2, ["abandon", "y::"]);
    insta::assert_snapshot!(output, @"
    y::conflicted_bookmark
    y::mutable_bookmark
    y::wv
    y::x
    y::u
    y::wq/0
    y::wq/1
    y::m
    y::r
    y::alias_with_newline
    y::siblings[EOF]
    ");

    // complete remote bookmarks in a revset expression
    let output = work_dir.complete_fish(["log", "-r", "remote_bookmark@"]);
    insta::assert_snapshot!(output, @"
    remote_bookmark@origin	remote_commit
    [EOF]
    ");

    // same, but through the bash code path, which strips help text
    let output = work_dir.complete_at(Shell::Bash, 3, ["log", "-r", "remote_bookmark@"]);
    insta::assert_snapshot!(output, @"remote_bookmark@origin[EOF]");

    // complete conflicted revisions in a revset expression
    let output = work_dir.complete_fish(["resolve", "-r", ""]);
    insta::assert_snapshot!(output, @"
    conflicted_bookmark	conflicted
    x	conflicted
    alias_with_newline	    roots(
    siblings	@-+ ~@
    [EOF]
    ");

    // complete args of the default command
    test_env.add_config("ui.default-command = 'log'");
    let output = work_dir.complete_fish(["-r", ""]);
    insta::assert_snapshot!(output, @"
    conflicted_bookmark	conflicted
    deleted_bookmark	(deleted bookmark)
    immutable_bookmark	immutable
    mutable_bookmark	mutable 1
    wv	working_copy
    x	conflicted
    u	mutable 3
    wq/0	mutable 2
    wq/1	mutable 1
    q	immutable
    m	deleted_remote_commit
    r	remote_commit
    z	(no description set)
    deleted_bookmark@origin	deleted_remote_commit
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
    insta::assert_snapshot!(output, @"
    a=conflicted_bookmark	conflicted
    a=deleted_bookmark	(deleted bookmark)
    a=immutable_bookmark	immutable
    a=mutable_bookmark	mutable 1
    a=wv	working_copy
    a=x	conflicted
    a=u	mutable 3
    a=wq/0	mutable 2
    a=wq/1	mutable 1
    a=q	immutable
    a=m	deleted_remote_commit
    a=r	remote_commit
    a=z	(no description set)
    a=deleted_bookmark@origin	deleted_remote_commit
    a=remote_bookmark@origin	remote_commit
    a=alias_with_newline	    roots(
    a=siblings	@-+ ~@
    [EOF]
    ");

    let output = work_dir.complete_fish(["git", "push", "--named", "a=a"]);
    insta::assert_snapshot!(output, @"
    a=alias_with_newline	    roots(
    [EOF]
    ");

    // a value with both "=" and "@" in it, through the bash code path
    let output = work_dir.complete_at(
        Shell::Bash,
        4,
        ["git", "push", "--named", "a=remote_bookmark@"],
    );
    insta::assert_snapshot!(output, @"a=remote_bookmark@origin[EOF]");
}

#[test]
fn test_revisions_workspace_symbols() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir
        .run_jj(["describe", "-m", "main workspace"])
        .success();

    // workspace working-copy symbols are not completed when there is only one
    // workspace
    let output = work_dir.complete_fish(["show", "def"]);
    insta::assert_snapshot!(output, @"");

    work_dir
        .run_jj(["workspace", "add", "--name", "secondary", "../secondary"])
        .success();

    // workspace working-copy symbols are completed
    let output = work_dir.complete_fish(["show", "def"]);
    insta::assert_snapshot!(output, @"
    default@	The working copy for workspace `default`
    [EOF]
    ");

    // they are also completed within revset expressions and respect the
    // mutable-revisions filter
    let output = work_dir.complete_fish(["abandon", "..def"]);
    insta::assert_snapshot!(output, @"
    ..default@	The working copy for workspace `default`
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
    let num_ops = 9;
    for i in 0..num_ops {
        work_dir
            .run_jj(["describe", "-m", &format!("description {i}")])
            .success();
    }

    let work_dir = test_env.work_dir("repo");

    let output = work_dir.complete_fish(["op", "show", ""]).success();
    insta::assert_snapshot!(output.take_stdout_n_lines(num_ops + 2), @"
    c5fcd0d80cb7	(2001-02-03 08:05:16) describe commit e0e6c0a964c024a49605805925672044dfae4181
    5bdb507f19ba	(2001-02-03 08:05:15) describe commit 37df8a6c1874ff45621dee0f2b7a77169b65d257
    8a4a92e5ecff	(2001-02-03 08:05:14) describe commit c3588cff852e44b68297f51705d6e61888806ddd
    aac41ddc75bf	(2001-02-03 08:05:13) describe commit aa0b3230e3787076f232a08c8b1c7f54948a2d7a
    c915cd6b4c8d	(2001-02-03 08:05:12) describe commit 96157804fd41363cb2ff8ff957ff1df1a2a1109a
    26d5ef64718f	(2001-02-03 08:05:11) describe commit 3725536d0ae06d69e46911258cee591dbdb66478
    1c97a622e394	(2001-02-03 08:05:10) describe commit dd7390802e3ca4467ffa43f2e0c0374463d056f3
    3ff36c586388	(2001-02-03 08:05:09) describe commit 3ae22e7f50a15d393e412cca72d09a61165d0c84
    7749eb4df93f	(2001-02-03 08:05:08) describe commit e8849ae12c709f2321908879bc724fdb2ab8a781
    f63ee16f9553	(2001-02-03 08:05:07) add workspace 'default'
    000000000000	(1970-01-01 11:00:00)
    [EOF]
    ");

    let output = work_dir.complete_fish(["op", "show", "c"]);
    insta::assert_snapshot!(output, @"
    c5fcd0d80cb7	(2001-02-03 08:05:16) describe commit e0e6c0a964c024a49605805925672044dfae4181
    c915cd6b4c8d	(2001-02-03 08:05:12) describe commit 96157804fd41363cb2ff8ff957ff1df1a2a1109a
    [EOF]
    ");
    // make sure global --at-op flag is respected (should not include later
    // operations)
    let output = work_dir.complete_fish(["--at-op", "f63ee16f9553", "op", "show", "f"]);
    insta::assert_snapshot!(output, @"
    f63ee16f9553	(2001-02-03 08:05:07) add workspace 'default'
    [EOF]
    ");

    let output = work_dir.complete_fish(["--at-op", "c5"]);
    insta::assert_snapshot!(output, @"
    c5fcd0d80cb7	(2001-02-03 08:05:16) describe commit e0e6c0a964c024a49605805925672044dfae4181
    [EOF]
    ");

    let output = work_dir.complete_fish(["op", "abandon", "c5"]);
    insta::assert_snapshot!(output, @"
    c5fcd0d80cb7	(2001-02-03 08:05:16) describe commit e0e6c0a964c024a49605805925672044dfae4181
    [EOF]
    ");

    let output = work_dir.complete_fish(["op", "diff", "--op", "c5"]);
    insta::assert_snapshot!(output, @"
    c5fcd0d80cb7	(2001-02-03 08:05:16) describe commit e0e6c0a964c024a49605805925672044dfae4181
    [EOF]
    ");
    let output = work_dir.complete_fish(["op", "diff", "--from", "c5"]);
    insta::assert_snapshot!(output, @"
    c5fcd0d80cb7	(2001-02-03 08:05:16) describe commit e0e6c0a964c024a49605805925672044dfae4181
    [EOF]
    ");
    let output = work_dir.complete_fish(["op", "diff", "--to", "c5"]);
    insta::assert_snapshot!(output, @"
    c5fcd0d80cb7	(2001-02-03 08:05:16) describe commit e0e6c0a964c024a49605805925672044dfae4181
    [EOF]
    ");

    let output = work_dir.complete_fish(["op", "restore", "c5"]);
    insta::assert_snapshot!(output, @"
    c5fcd0d80cb7	(2001-02-03 08:05:16) describe commit e0e6c0a964c024a49605805925672044dfae4181
    [EOF]
    ");

    let output = work_dir.complete_fish(["op", "revert", "c5"]);
    insta::assert_snapshot!(output, @"
    c5fcd0d80cb7	(2001-02-03 08:05:16) describe commit e0e6c0a964c024a49605805925672044dfae4181
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
    insta::assert_snapshot!(output, @"
    def-second	(no description set)
    default	initial
    [EOF]
    ");
}

#[test]
fn test_config() {
    let test_env = TestEnvironment::default();

    let output = test_env.complete_fish(["config", "get", "f"]);
    insta::assert_snapshot!(output, @"
    fsmonitor.backend	Whether to use an external filesystem monitor, useful for large repos
    fsmonitor.watchman.register-snapshot-trigger	Whether to use triggers to monitor for changes in the background.
    [EOF]
    ");

    let output = test_env.complete_fish(["config", "list", "fs"]);
    insta::assert_snapshot!(output, @"
    fsmonitor	External filesystem monitor settings, useful for large repos
    fsmonitor.backend	Whether to use an external filesystem monitor, useful for large repos
    fsmonitor.watchman
    fsmonitor.watchman.register-snapshot-trigger	Whether to use triggers to monitor for changes in the background.
    [EOF]
    ");

    let output = test_env.complete_fish(["log", "--config", "f"]);
    insta::assert_snapshot!(output, @"
    fsmonitor.backend=	Whether to use an external filesystem monitor, useful for large repos
    fsmonitor.watchman.register-snapshot-trigger=	Whether to use triggers to monitor for changes in the background.
    [EOF]
    ");

    let output = test_env.complete_fish(["log", "--config", "ui.conflict-marker-style="]);
    insta::assert_snapshot!(output, @"
    ui.conflict-marker-style=diff
    ui.conflict-marker-style=diff-experimental
    ui.conflict-marker-style=snapshot
    ui.conflict-marker-style=git
    [EOF]
    ");

    let output = test_env.complete_fish(["log", "--config", "ui.conflict-marker-style=g"]);
    insta::assert_snapshot!(output, @"
    ui.conflict-marker-style=git
    [EOF]
    ");

    let output = test_env.complete_fish(["log", "--config", "git.abandon-unreachable-commits="]);
    insta::assert_snapshot!(output, @"
    git.abandon-unreachable-commits=false
    git.abandon-unreachable-commits=true
    [EOF]
    ");
}

#[test]
fn test_config_unset() {
    let test_env = TestEnvironment::default();

    // Create a repo with repo and workspace config
    let repo_dir = test_env.work_dir("repo");
    test_env
        .run_jj_in(test_env.env_root(), ["git", "init", "repo"])
        .success();
    repo_dir
        .run_jj(["config", "set", "--workspace", "ui.pager", "delta"])
        .success();
    repo_dir
        .run_jj(["config", "set", "--repo", "ui.diff-formatter", ":git"])
        .success();

    // Only config options set in TestEnvironment are suggested initially + other
    // viable flags
    let output = test_env.complete_fish(["config", "unset", ""]);
    insta::assert_snapshot!(output.take_stdout_n_lines(5), @r#"
    template-aliases."format_time_range(time_range)"	user: 'time_range.start() ++ " - " ++ time_range.end()'
    git.colocate	user: false
    --user	Target the user-level config
    --repo	Target the repo-level config
    --workspace	Target the workspace-level config
    [EOF]
    "#);

    test_env.add_config(indoc! {r#"
        [ui]
        editor = "nvim"
        default-command = ["log", "--stat"]

        [revset-aliases]
        'closest_bookmark(to)' = 'heads(::to & bookmarks())'
        'closest_pushable(to)' = 'heads(::to & ~description(exact:"") & (~empty() | merges()))'

        [aliases]
        tug = ["bookmark", "move", "--from", "closest_bookmark(@)", "--to", "closest_pushable(@)"]
        cat = ["file", "show"]
        ll = ["log", "-T", "builtin_log_detailed", "-r", "::@"]

        [templates]
        draft_commit_description = '''
        concat(
          coalesce(description, "\n"),
          surround(
            "\nJJ: This commit contains the following changes:\n", "",
            indent("JJ:     ", diff.stat(72)),
          ),
          "\nJJ: ignore-rest\n",
          diff.git(),
        )
        '''
    "#});

    // Matching config options are completed, including aliases
    let output = test_env.complete_fish(["config", "unset", "alias"]);
    insta::assert_snapshot!(output, @r#"
    aliases.tug	user: ["bookmark", "move", "--from", "closest_bookmark(@)", "--to", "closest_pushable(@)"]
    aliases.cat	user: ["file", "show"]
    aliases.ll	user: ["log", "-T", "builtin_log_detailed", "-r", "::@"]
    [EOF]
    "#);

    // Quoted config option keys are completed (accepting double-quotes only)
    let output = test_env.complete_fish(["config", "unset", "revset-aliases.\"close"]);
    insta::assert_snapshot!(output, @r#"
    revset-aliases."closest_bookmark(to)"	user: 'heads(::to & bookmarks())'
    revset-aliases."closest_pushable(to)"	user: 'heads(::to & ~description(exact:"") & (~empty() | merges()))'
    [EOF]
    "#);
    let output = test_env.complete_fish(["config", "unset", "revset-aliases.'close"]);
    insta::assert_snapshot!(output, @"");

    // Multiline config values are squeezed onto a single line
    let output = test_env.complete_fish(["config", "unset", "templates"]);
    insta::assert_snapshot!(output, @r#"
    templates.draft_commit_description	user: ''' concat( coalesce(description, "\n"), surround( "\nJJ: This commit contains the following changes:\n", "", indent("JJ:     ", diff.stat(72)), ), "\nJJ: ignore-rest\n", diff.git(), ) '''
    [EOF]
    "#);

    // If no config source is specified yet, options from all sources are completed
    let output = repo_dir.complete_fish(["config", "unset", "ui"]);
    insta::assert_snapshot!(output, @r#"
    ui.editor	user: "nvim"
    ui.default-command	user: ["log", "--stat"]
    ui.diff-formatter	repo: ":git"
    ui.pager	workspace: "delta"
    [EOF]
    "#);

    // If a config source has already been specified, only its options as completed
    let output = repo_dir.complete_fish(["config", "unset", "--user", "ui"]);
    insta::assert_snapshot!(output, @r#"
    ui.editor	user: "nvim"
    ui.default-command	user: ["log", "--stat"]
    [EOF]
    "#);
    let output = repo_dir.complete_fish(["config", "unset", "--repo", "ui"]);
    insta::assert_snapshot!(output, @r#"
    ui.diff-formatter	repo: ":git"
    [EOF]
    "#);
    let output = repo_dir.complete_fish(["config", "unset", "--workspace", "ui"]);
    insta::assert_snapshot!(output, @r#"
    ui.pager	workspace: "delta"
    [EOF]
    "#);

    // Override an option in the repo config
    repo_dir
        .run_jj(["config", "set", "--repo", "ui.editor", "hx"])
        .success();

    // If no config source is specified yet, the overridden value is listed
    let output = repo_dir.complete_fish(["config", "unset", "ui.editor"]);
    insta::assert_snapshot!(output, @r#"
    ui.editor	repo: "hx"
    [EOF]
    "#);

    // If a config source has already been specified, the value according to that
    // source is listed
    let output = repo_dir.complete_fish(["config", "unset", "--user", "ui.editor"]);
    insta::assert_snapshot!(output, @r#"
    ui.editor	user: "nvim"
    [EOF]
    "#);
    let output = repo_dir.complete_fish(["config", "unset", "--repo", "ui.editor"]);
    insta::assert_snapshot!(output, @r#"
    ui.editor	repo: "hx"
    [EOF]
    "#);
    let output = repo_dir.complete_fish(["config", "unset", "--workspace", "ui.editor"]);
    insta::assert_snapshot!(output, @"");
}

#[test]
fn test_template_alias() {
    let test_env = TestEnvironment::default();

    let output = test_env.complete_fish(["log", "-T", ""]);
    insta::assert_snapshot!(output, @"
    builtin_config_list
    builtin_config_list_detailed
    builtin_draft_commit_description
    builtin_draft_commit_description_with_diff
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
    builtin_workspace_list
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
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output.normalize_backslash(), @"
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
    │ ×  royxmykx test.user@example.com 2001-02-03 08:05:14 conflicted 26ca82ca (conflict)
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
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
    f_dir/
    [EOF]
    ");

    let output = work_dir.complete_fish(["file", "show", "f_dir/"]);
    insta::assert_snapshot!(output, @"
    f_dir/dir_file_1
    f_dir/dir_file_2
    f_dir/dir_file_3
    f_dir/f_renamed_3
    [EOF]
    ");

    let output = work_dir.complete_fish(["file", "show", "f_dir/../"]);
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
    f_dir/../f_dir/dir_file_1
    f_dir/../f_dir/dir_file_2
    f_dir/../f_dir/dir_file_3
    f_dir/../f_dir/f_renamed_3
    [EOF]
    ");

    let subdir = work_dir.dir("f_dir");
    let output = subdir.complete_fish(["file", "show", "dir_"]);
    insta::assert_snapshot!(output, @"
    dir_file_1
    dir_file_2
    dir_file_3
    [EOF]
    ");

    let output = subdir.complete_fish(["file", "show", "./"]);
    insta::assert_snapshot!(output, @"
    ./dir_file_1
    ./dir_file_2
    ./dir_file_3
    ./f_renamed_3
    [EOF]
    ");

    let output = subdir.complete_fish(["file", "show", "../"]);
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
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

    let output = work_dir.complete_fish(["diff", "--revisions=@-", "f_"]);
    insta::assert_snapshot!(output, @"
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

    let output = work_dir.complete_fish(["diff", "-r", "first", "--revisions=second", "f_"]);
    insta::assert_snapshot!(output, @"
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

    let output = work_dir.complete_fish(["diff", "-r", "@-", "f_dir/../"]);
    insta::assert_snapshot!(output, @"
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
        @"
    f_dir→dir_file_1	Added
    f_dir→dir_file_2	Added
    f_dir→dir_file_3	Added
    f_dir→f_renamed_3	Renamed
    [EOF]
    ");

    let output = work_dir.complete_fish(["diff", "--from", "root()", "--to", "@-", "f_"]);
    insta::assert_snapshot!(output, @"
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

    let output = work_dir.complete_fish(["diff", "--to=second", "f_"]);
    insta::assert_snapshot!(output, @"
    f_added_2	Deleted
    f_modified	Modified
    [EOF]
    ");

    let output = work_dir.complete_fish(["restore", "-c", "@-", "f_"]);
    insta::assert_snapshot!(output.normalize_backslash(), @"
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
    insta::assert_snapshot!(output.normalize_backslash(), @"
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
    insta::assert_snapshot!(output, @"
    f_interdiff_only_from	Added
    f_interdiff_same	Added
    f_interdiff_only_to	Added
    f_interdiff_same	Added
    [EOF]
    ");

    // squash has a different behavior with --from and --to flags
    let output = work_dir.complete_fish(["squash", "-f=first", "f_"]);
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
    f_dir/
    f_modified
    [EOF]
    ");

    let output = work_dir.complete_fish(["file", "list", "-r=first", "f_"]);
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output, @"
    f_added_2	Added
    f_modified	Modified
    [EOF]
    ");

    let output = work_dir.complete_fish(["absorb", "-f=conflicted", "f_"]);
    insta::assert_snapshot!(output, @"
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
    insta::assert_snapshot!(output.normalize_backslash(), @"
    file1
    file2
    folder/
    [EOF]
    ");
}
