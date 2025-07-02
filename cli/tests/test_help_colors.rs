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
fn test_help_colors_default() {
    let test_env = TestEnvironment::default();
    test_env.add_config("ui.color = 'always'");

    // Test the main help - just check first few lines to verify colors are applied
    let output = test_env.run_jj_in(".", ["--help"]);
    let output =
        output.normalize_stdout_with(|s| s.lines().take(10).collect::<Vec<_>>().join("\n"));
    insta::assert_snapshot!(output, @r"
    Jujutsu (An experimental VCS)

    To get started, see the tutorial [`jj help -k tutorial`].

    [`jj help -k tutorial`]: https://jj-vcs.github.io/jj/latest/tutorial/

    [1m[33mUsage:[0m [1m[32mjj[0m [32m[OPTIONS][0m [32m<COMMAND>[0m

    [1m[33mCommands:[0m
      [1m[32mabandon[0m           Abandon a revision[EOF]
    ");
}

#[test]
fn test_help_colors_custom() {
    let test_env = TestEnvironment::default();
    test_env.add_config(
        r#"
ui.color = "always"
[colors]
"help heading" = "red"
"help usage" = { fg = "blue", bold = true }
"help literal" = { fg = "cyan" }
"help placeholder" = "magenta"
"#,
    );

    let output = test_env.run_jj_in(".", ["--help"]);
    // Custom colors: red headings, blue bold usage, cyan literals
    let output =
        output.normalize_stdout_with(|s| s.lines().take(10).collect::<Vec<_>>().join("\n"));
    insta::assert_snapshot!(output, @r"
    Jujutsu (An experimental VCS)

    To get started, see the tutorial [`jj help -k tutorial`].

    [`jj help -k tutorial`]: https://jj-vcs.github.io/jj/latest/tutorial/

    [1m[34mUsage:[0m [1m[36mjj[0m [35m[OPTIONS][0m [35m<COMMAND>[0m

    [31mCommands:[0m
      [1m[36mabandon[0m           Abandon a revision[EOF]
    ");
}

#[test]
fn test_help_colors_subcommand() {
    let test_env = TestEnvironment::default();
    test_env.add_config(
        r#"
ui.color = "always"
[colors]
"help heading" = "magenta"
"help usage" = "green"
"help literal" = "yellow"
"#,
    );

    // Test that colors work on subcommand help too
    let output = test_env.run_jj_in(".", ["help", "log"]);
    let output =
        output.normalize_stdout_with(|s| s.lines().take(12).collect::<Vec<_>>().join("\n"));
    insta::assert_snapshot!(output, @r#"
    Show revision history

    Renders a graphical view of the project's history, ordered with children before parents. By default,
    the output only includes mutable revisions, along with some additional revisions for context. Use
    `jj log -r ::` to see all revisions. See [`jj help -k revsets`] for information about the syntax.

    [`jj help -k revsets`]: https://jj-vcs.github.io/jj/latest/revsets/

    Spans of revisions that are not included in the graph per `--revisions` are rendered as a synthetic
    node labeled "(elided revisions)".

    The working-copy commit is indicated by a `@` symbol in the graph. [Immutable revisions] have a `◆`[EOF]
    "#);
}

#[test]
fn test_help_colors_bright() {
    let test_env = TestEnvironment::default();
    test_env.add_config(
        r#"
ui.color = "always"
[colors]
"help heading" = "bright red"
"help usage" = "bright blue"
"help literal" = "bright green"
"#,
    );

    let output = test_env.run_jj_in(".", ["--help"]);
    // Bright colors use codes 91-97 instead of 31-37
    let output =
        output.normalize_stdout_with(|s| s.lines().take(10).collect::<Vec<_>>().join("\n"));
    insta::assert_snapshot!(output, @r"
    Jujutsu (An experimental VCS)

    To get started, see the tutorial [`jj help -k tutorial`].

    [`jj help -k tutorial`]: https://jj-vcs.github.io/jj/latest/tutorial/

    [94mUsage:[0m [92mjj[0m [32m[OPTIONS][0m [32m<COMMAND>[0m

    [91mCommands:[0m
      [92mabandon[0m           Abandon a revision[EOF]
    ");
}

#[test]
fn test_help_colors_with_attributes() {
    let test_env = TestEnvironment::default();
    test_env.add_config(
        r#"
ui.color = "always"
[colors]
"help heading" = { fg = "red", bold = true, italic = true }
"help usage" = { fg = "blue", underline = true }
"#,
    );

    let output = test_env.run_jj_in(".", ["--help"]);
    // Multiple attributes are combined: bold=1, italic=3, underline=4
    let output =
        output.normalize_stdout_with(|s| s.lines().take(10).collect::<Vec<_>>().join("\n"));
    insta::assert_snapshot!(output, @r"
    Jujutsu (An experimental VCS)

    To get started, see the tutorial [`jj help -k tutorial`].

    [`jj help -k tutorial`]: https://jj-vcs.github.io/jj/latest/tutorial/

    [1m[4m[34mUsage:[0m [1m[32mjj[0m [32m[OPTIONS][0m [32m<COMMAND>[0m

    [1m[3m[31mCommands:[0m
      [1m[32mabandon[0m           Abandon a revision[EOF]
    ");
}

#[test]
fn test_help_colors_hex_fallback() {
    let test_env = TestEnvironment::default();
    test_env.add_config(
        r##"
ui.color = "always"
[colors]
"help heading" = "#ff0000"
"##,
    );

    let output = test_env.run_jj_in(".", ["help"]);
    // Hex colors should produce a warning and fall back to defaults
    insta::assert_snapshot!(output, @r"
    Jujutsu (An experimental VCS)

    To get started, see the tutorial [`jj help -k tutorial`].

    [`jj help -k tutorial`]: https://jj-vcs.github.io/jj/latest/tutorial/

    [1m[33mUsage:[0m [1m[32mjj[0m [32m[OPTIONS][0m [32m<COMMAND>[0m

    [1m[33mCommands:[0m
      [1m[32mabandon[0m           Abandon a revision
      [1m[32mabsorb[0m            Move changes from a revision into the stack of mutable revisions
      [1m[32mbookmark[0m          Manage bookmarks [default alias: b]
      [1m[32mcommit[0m            Update the description and create a new change on top
      [1m[32mconfig[0m            Manage config options
      [1m[32mdescribe[0m          Update the change description or other metadata [aliases: desc]
      [1m[32mdiff[0m              Compare file contents between two revisions
      [1m[32mdiffedit[0m          Touch up the content changes in a revision with a diff editor
      [1m[32mduplicate[0m         Create new changes with the same content as existing ones
      [1m[32medit[0m              Sets the specified revision as the working-copy revision
      [1m[32mevolog[0m            Show how a change has evolved over time [aliases: evolution-log]
      [1m[32mfile[0m              File operations
      [1m[32mfix[0m               Update files with formatting fixes or other changes
      [1m[32mgit[0m               Commands for working with Git remotes and the underlying Git repo
      [1m[32mhelp[0m              Print this message or the help of the given subcommand(s)
      [1m[32minterdiff[0m         Compare the changes of two commits
      [1m[32mlog[0m               Show revision history
      [1m[32mnew[0m               Create a new, empty change and (by default) edit it in the working copy
      [1m[32mnext[0m              Move the working-copy commit to the child revision
      [1m[32moperation[0m         Commands for working with the operation log [aliases: op]
      [1m[32mparallelize[0m       Parallelize revisions by making them siblings
      [1m[32mprev[0m              Change the working copy revision relative to the parent revision
      [1m[32mrebase[0m            Move revisions to different parent(s)
      [1m[32mresolve[0m           Resolve conflicted files with an external merge tool
      [1m[32mrestore[0m           Restore paths from another revision
      [1m[32mrevert[0m            Apply the reverse of the given revision(s)
      [1m[32mroot[0m              Show the current workspace root directory (shortcut for `jj workspace root`)
      [1m[32mshow[0m              Show commit description and changes in a revision
      [1m[32msign[0m              Cryptographically sign a revision
      [1m[32msimplify-parents[0m  Simplify parent edges for the specified revision(s)
      [1m[32msparse[0m            Manage which paths from the working-copy commit are present in the working copy
      [1m[32msplit[0m             Split a revision in two
      [1m[32msquash[0m            Move changes from a revision into another revision
      [1m[32mstatus[0m            Show high-level repo status [aliases: st]
      [1m[32mtag[0m               Manage tags
      [1m[32mundo[0m              Undo an operation (shortcut for `jj op undo`)
      [1m[32munsign[0m            Drop a cryptographic signature
      [1m[32mutil[0m              Infrequently used commands such as for generating shell completions
      [1m[32mversion[0m           Display version information
      [1m[32mworkspace[0m         Commands for working with workspaces

    [1m[33mOptions:[0m
      [1m[32m-h[0m, [1m[32m--help[0m
              Print help (see a summary with '-h')

      [1m[32m-V[0m, [1m[32m--version[0m
              Print version

    [1m[33mGlobal Options:[0m
      [1m[32m-R[0m, [1m[32m--repository[0m[32m [0m[32m<REPOSITORY>[0m
              Path to repository to operate on
              
              By default, Jujutsu searches for the closest .jj/ directory in an ancestor of the current
              working directory.

          [1m[32m--ignore-working-copy[0m
              Don't snapshot the working copy, and don't update it
              
              By default, Jujutsu snapshots the working copy at the beginning of every command. The
              working copy is also updated at the end of the command, if the command modified the
              working-copy commit (`@`). If you want to avoid snapshotting the working copy and instead
              see a possibly stale working-copy commit, you can use `--ignore-working-copy`. This may be
              useful e.g. in a command prompt, especially if you have another process that commits the
              working copy.
              
              Loading the repository at a specific operation with `--at-operation` implies
              `--ignore-working-copy`.

          [1m[32m--ignore-immutable[0m
              Allow rewriting immutable commits
              
              By default, Jujutsu prevents rewriting commits in the configured set of immutable commits.
              This option disables that check and lets you rewrite any commit but the root commit.
              
              This option only affects the check. It does not affect the `immutable_heads()` revset or
              the `immutable` template keyword.

          [1m[32m--at-operation[0m[32m [0m[32m<AT_OPERATION>[0m
              Operation to load the repo at
              
              Operation to load the repo at. By default, Jujutsu loads the repo at the most recent
              operation, or at the merge of the divergent operations if any.
              
              You can use `--at-op=<operation ID>` to see what the repo looked like at an earlier
              operation. For example `jj --at-op=<operation ID> st` will show you what `jj st` would
              have shown you when the given operation had just finished. `--at-op=@` is pretty much the
              same as the default except that divergent operations will never be merged.
              
              Use `jj op log` to find the operation ID you want. Any unambiguous prefix of the operation
              ID is enough.
              
              When loading the repo at an earlier operation, the working copy will be ignored, as if
              `--ignore-working-copy` had been specified.
              
              It is possible to run mutating commands when loading the repo at an earlier operation.
              Doing that is equivalent to having run concurrent commands starting at the earlier
              operation. There's rarely a reason to do that, but it is possible.
              
              [aliases: --at-op]

          [1m[32m--debug[0m
              Enable debug logging

          [1m[32m--color[0m[32m [0m[32m<WHEN>[0m
              When to colorize output
              
              [possible values: always, never, debug, auto]

          [1m[32m--quiet[0m
              Silence non-primary command output
              
              For example, `jj file list` will still list files, but it won't tell you if the working
              copy was snapshotted or if descendants were rebased.
              
              Warnings and errors will still be printed.

          [1m[32m--no-pager[0m
              Disable the pager

          [1m[32m--config[0m[32m [0m[32m<NAME=VALUE>[0m
              Additional configuration options (can be repeated)
              
              The name should be specified as TOML dotted keys. The value should be specified as a TOML
              expression. If string value isn't enclosed by any TOML constructs (such as array
              notation), quotes can be omitted.

          [1m[32m--config-file[0m[32m [0m[32m<PATH>[0m
              Additional configuration files (can be repeated)

    [1m'jj help --help'[0m lists available keywords. Use [1m'jj help -k'[0m to show help for one of these keywords.
    [EOF]
    ------- stderr -------
    [1m[38;5;3mWarning: [39mHex color #ff0000 is not supported for help text. Only ANSI colors are supported.[0m
    [1m[38;5;3mWarning: [39mHex color #ff0000 is not supported for help text. Only ANSI colors are supported.[0m
    [EOF]
    ");
}

#[test]
fn test_help_colors_ansi256_fallback() {
    let test_env = TestEnvironment::default();
    test_env.add_config(
        r#"
ui.color = "always"
[colors]
"help usage" = "ansi-color-196"
"#,
    );

    let output = test_env.run_jj_in(".", ["help", "status"]);
    // ANSI 256 colors should produce a warning and fall back to defaults
    insta::assert_snapshot!(output, @r"
    Show high-level repo status

    This includes:

    * The working copy commit and its parents, and a summary of the changes in the working copy
    (compared to the merged parents) * Conflicts in the working copy * [Conflicted bookmarks]

    [Conflicted bookmarks]: https://jj-vcs.github.io/jj/latest/bookmarks/#conflicts

    [1m[33mUsage:[0m [1m[32mjj status[0m [32m[OPTIONS][0m [32m[FILESETS]...[0m

    [1m[33mArguments:[0m
      [32m[FILESETS]...[0m
              Restrict the status display to these paths

    [1m[33mOptions:[0m
      [1m[32m-h[0m, [1m[32m--help[0m
              Print help (see a summary with '-h')

    [1m[33mGlobal Options:[0m
      [1m[32m-R[0m, [1m[32m--repository[0m[32m [0m[32m<REPOSITORY>[0m
              Path to repository to operate on
              
              By default, Jujutsu searches for the closest .jj/ directory in an ancestor of the current
              working directory.

          [1m[32m--ignore-working-copy[0m
              Don't snapshot the working copy, and don't update it
              
              By default, Jujutsu snapshots the working copy at the beginning of every command. The
              working copy is also updated at the end of the command, if the command modified the
              working-copy commit (`@`). If you want to avoid snapshotting the working copy and instead
              see a possibly stale working-copy commit, you can use `--ignore-working-copy`. This may be
              useful e.g. in a command prompt, especially if you have another process that commits the
              working copy.
              
              Loading the repository at a specific operation with `--at-operation` implies
              `--ignore-working-copy`.

          [1m[32m--ignore-immutable[0m
              Allow rewriting immutable commits
              
              By default, Jujutsu prevents rewriting commits in the configured set of immutable commits.
              This option disables that check and lets you rewrite any commit but the root commit.
              
              This option only affects the check. It does not affect the `immutable_heads()` revset or
              the `immutable` template keyword.

          [1m[32m--at-operation[0m[32m [0m[32m<AT_OPERATION>[0m
              Operation to load the repo at
              
              Operation to load the repo at. By default, Jujutsu loads the repo at the most recent
              operation, or at the merge of the divergent operations if any.
              
              You can use `--at-op=<operation ID>` to see what the repo looked like at an earlier
              operation. For example `jj --at-op=<operation ID> st` will show you what `jj st` would
              have shown you when the given operation had just finished. `--at-op=@` is pretty much the
              same as the default except that divergent operations will never be merged.
              
              Use `jj op log` to find the operation ID you want. Any unambiguous prefix of the operation
              ID is enough.
              
              When loading the repo at an earlier operation, the working copy will be ignored, as if
              `--ignore-working-copy` had been specified.
              
              It is possible to run mutating commands when loading the repo at an earlier operation.
              Doing that is equivalent to having run concurrent commands starting at the earlier
              operation. There's rarely a reason to do that, but it is possible.
              
              [aliases: --at-op]

          [1m[32m--debug[0m
              Enable debug logging

          [1m[32m--color[0m[32m [0m[32m<WHEN>[0m
              When to colorize output
              
              [possible values: always, never, debug, auto]

          [1m[32m--quiet[0m
              Silence non-primary command output
              
              For example, `jj file list` will still list files, but it won't tell you if the working
              copy was snapshotted or if descendants were rebased.
              
              Warnings and errors will still be printed.

          [1m[32m--no-pager[0m
              Disable the pager

          [1m[32m--config[0m[32m [0m[32m<NAME=VALUE>[0m
              Additional configuration options (can be repeated)
              
              The name should be specified as TOML dotted keys. The value should be specified as a TOML
              expression. If string value isn't enclosed by any TOML constructs (such as array
              notation), quotes can be omitted.

          [1m[32m--config-file[0m[32m [0m[32m<PATH>[0m
              Additional configuration files (can be repeated)
    [EOF]
    ------- stderr -------
    [1m[38;5;3mWarning: [39mANSI 256 color (ansi-color-196) is not supported for help text. Only ANSI colors are supported.[0m
    [1m[38;5;3mWarning: [39mANSI 256 color (ansi-color-196) is not supported for help text. Only ANSI colors are supported.[0m
    [EOF]
    ");
}

#[test]
fn test_help_colors_partial_config() {
    let test_env = TestEnvironment::default();
    test_env.add_config(
        r#"
ui.color = "always"
[colors]
"help heading" = "red"
"help literal" = "cyan"
"#,
    );

    let output = test_env.run_jj_in(".", ["--help"]);
    // Only configured colors change, others remain default
    let output =
        output.normalize_stdout_with(|s| s.lines().take(10).collect::<Vec<_>>().join("\n"));
    insta::assert_snapshot!(output, @r"
    Jujutsu (An experimental VCS)

    To get started, see the tutorial [`jj help -k tutorial`].

    [`jj help -k tutorial`]: https://jj-vcs.github.io/jj/latest/tutorial/

    [1m[33mUsage:[0m [36mjj[0m [32m[OPTIONS][0m [32m<COMMAND>[0m

    [31mCommands:[0m
      [36mabandon[0m           Abandon a revision[EOF]
    ");
}

#[test]
fn test_help_colors_invalid_config_error() {
    let test_env = TestEnvironment::default();
    test_env.add_config(
        r#"
ui.color = "always"
[colors]
"help heading" = "invalid-color"
"#,
    );

    let output = test_env.run_jj_in(".", ["help"]);
    // Invalid colors in config should cause a config error
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Config error: Invalid type or value for colors."help heading"
    Caused by: Invalid color: invalid-color

    Hint: Check the config file: $TEST_ENV/config/config0002.toml
    For help, see https://jj-vcs.github.io/jj/latest/config/ or use `jj help -k config`.
    [EOF]
    [exit status: 1]
    "#);
}
