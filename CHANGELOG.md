# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Breaking changes

* The deprecated flag `--summary` of `jj abandon` has been removed.

* The deprecated command `jj backout` has been removed, use `jj revert` instead.

* The following deprecated config options have been removed:
  - `signing.sign-all`
  - `core.watchman.register_snapshot_trigger`
  - `diff.format`

### Deprecations

 * `jj bisect run --command <cmd>` is deprecated in favor of
   `jj bisect run -- <cmd>`.

### New features

* Templates now support a `.split(separator, [limit])` method on strings to
  split a string into a list of substrings.

* `-G` is now available as a short form of `--no-graph` in `jj log`, `jj evolog`,
  `jj op log`, `jj op show` and `jj op diff`.

* `jj metaedit` now accepts `-m`/`--message` option to non-interactively update
  the change description.

* The `CryptographicSignature.key()` template method now also works for SSH
  signatures and returns the corresponding public key fingerprint.

* Added `template-aliases.empty_commit_marker`. Users can override this value in
  their config to change the "(empty)" label on empty commits.

* Workspaces may have an additional layered configuration, located at
  `.jj/workspace-config.toml`. `jj config` subcommands which took layer options like
  `--repo` now also support `--workspace`.

* Add support for `--when.workspaces` config scopes.

* `jj bisect run` accepts the command and arguments to pass to the command
  directly as positional arguments, such as
  `jj bisect --range=..main -- cargo check --all-targets`.

* Divergent changes are no longer marked red in immutable revisions. Since the
  revision is immutable, the user shouldn't take any action, so the red color
  was unnecessarily alarming.

* `jj bookmark track` can now associate new local bookmarks with remote.
  Tracked bookmarks can be pushed without `--allow-new`.
  [#7072](https://github.com/jj-vcs/jj/issues/7072)

* New `jj tag set`/`delete` commands to create/update/delete tags locally.
  Updated tags will be exported to Git as lightweight tags.

* New commit template keywords `local`/`remote_tags` to show only local/remote
  tags. These keywords may be useful in non-colocated Git repositories where
  local and exported `@git` tags can point to different revisions.

*  `jj git clone` now supports the `--branch` option to specify the branch(es)
  to fetch during clone. If present, the first matching branch is used as the
  working-copy parent.

* The new `jj git colocation` command provides sub-commands to show the
  colocation state (`status`) and to convert a non-colocated git repo into
  a colocated repo (`enable`) and vice-versa `disable`.

### Fixed bugs

* `jj metaedit --author-timestamp` twice with the same value no longer
edits the change twice in some cases.

* `jj squash`: fixed improper revision rebase when both `--insert-after` and
  `--insert-before` were used.

* `jj undo` can now revert "fetch"/"import" operation that involves tag updates.
  [#6325](https://github.com/jj-vcs/jj/issues/6325)

* Fixed parsing of `files(expr)` revset expression including parentheses.
  [#7747](https://github.com/jj-vcs/jj/issues/7747)

## [0.34.0] - 2025-10-01

### Release highlights

* Support for uploading changes to Gerrit Code Review with `jj gerrit upload`.
  This lets you submit trees or multiple stacks of work at once. Support
  for fetching changes, submitting changes, and other operations is not yet
  implemented. This should be considered experimental, and we welcome feedback
  on using it.

* Support for automated bisection using `jj bisect run`; this will continuously
  bisect a commit range until a given commit is found to trigger a bug.

### Breaking changes

* Git-based repositories are now colocated by default. Configure `git.colocate =
  false` to keep the previous behavior.

* Conflicts written by jj < 0.11 are no longer supported. They will now appear
  as regular files with a `.jjconflict` suffix and JSON contents.

* The minimum supported Rust version (MSRV) is now 1.88.

### Deprecations

* Various flags on `jj describe` and `jj commit` have been deprecated in favor
  of `jj metaedit`. They are:
  * `describe`: `--author`, `--reset-author`, `--no-edit`
  * `commit`:   `--author`, `--reset-author`

* The storage format of remote bookmarks and tags has changed. Remote bookmarks
  will be written in both old and new formats to retain forward compatibility.
  Git-tracking tags are also recorded for future native tagging support. This
  change should be transparent, but let us know if you see anything odd.

### New features

* The new command `jj bisect run` uses binary search to find a commit that
  introduced a bug.

* The default editor on Unix is now `nano` instead of `pico`.

* New config option `merge.hunk-level = "word"` to enable word-level merging.

* New config option `merge.same-change = "keep"` to disable lossy resolution
  rule for same-change conflicts.
  [#6369](https://github.com/jj-vcs/jj/issues/6369)

* Templates now support a `replace()` method on strings for pattern-based
  string replacement with optional limits. Supports all string patterns, including
  regex with capture groups (e.g. `"hello world".replace(regex:'(\w+) (\w+)', "$2 $1")`).

* A new builtin `hyperlink(url, text)` template alias creates clickable
  hyperlinks using [OSC8 escape sequences](https://github.com/Alhadis/OSC8-Adoption)
  for terminals that support them.

* Added a new conditional configuration `--when.platforms` to include
  settings only on certain platforms.

* External diff commands now support substitution variable `$width` for the
  number of available terminal columns.

* The new `jj gerrit upload` command allows you to upload given revisions to an
  instance of Gerrit Code Review.

* `jj bookmark create/set/move` use the working copy as a default again and
  no longer require an explicit revision argument. This walks back a
  deprecation from `jj 0.26`, as the community feedback was mostly negative.
  Instead, bookmarking an empty revision now produces a warning, to help you
  catch the case where you meant to bookmark the parent revision.

* The revset function `exactly(x, n)` will now evaluate `x` and error if it does
  not have exactly `n` elements.

* `jj util exec` now matches the exit status of the program it runs, and
  doesn't print anything.

* `jj config get` now supports displaying array and table config values.

* `jj util exec` sets the environment variable `JJ_WORKSPACE_ROOT`

### Fixed bugs

* Fetching repositories that have submodules no longer errors even if
  `submodule.recurse=true` is set in `.gitconfig` (but jj still isn't able to
  fetch the submodules or to operate on them).

### Contributors

Thanks to the people who made this release happen!

* Angel Ezquerra (@AngelEzquerra)
* Anton Älgmyr (@algmyr)
* Antonin Delpeuch (@wetneb)
* Austin Seipp (@thoughtpolice)
* Benjamin Tan (@bnjmnt4n)
* Conner Petzold (@ConnerPetzold)
* Daniel Luz (@mernen)
* Daniele Sassoli (@DanieleSassoli)
* David Barsky (@davidbarsky)
* Dinu Blanovschi (@dnbln)
* Gaëtan Lehmann (@glehmann)
* George Christou (@gechr)
* Ian Wrzesinski (@isuffix)
* Ilya Grigoriev (@ilyagr)
* Isaac Corbrey (@icorbrey)
* Ivan Petkov (@ipetkov)
* Jonas Fierlings (@PigeonF)
* loudgolem (@phanirithvij)
* Martin von Zweigbergk (@martinvonz)
* Matt Stark (@matts1)
* Matt T. Proud (@matttproud)
* Michael Pratt (@prattmic)
* MochikoNyan (@MochikoNyan)
* Philip Metzger (@PhilipMetzger)
* Reilly Brogan (@ReillyBrogan)
* Remo Senekowitsch (@senekor)
* Reuven Lazarus (@rlazarus)
* Scott Taylor (@scott2000)
* Stephen Jennings (@jennings)
* Steven Sherry (@Steven0351)
* Theo Buehler (@botovq)
* Yuya Nishihara (@yuja)

## [0.33.0] - 2025-09-03

### Release highlights

* `jj undo` is now *sequential*: invoking it multiple times in sequence
  repeatedly undoes actions in the operation log. Previously, `jj undo` would
  only undo *the most recent* operation in the operation log. As a result, a new
  `jj redo` command has been added.

* Experimental support for improving query performance over filesets and file
  queries (like `jj log path/to/file.txt`) has been added. This is not enabled
  by default. To enable this, you must use the `jj debug index-changed-paths`
  command.

### Breaking changes

* `jj evolog` templates now accept `CommitEvolutionEntry` as context type. To
  get `Commit` properties, use `commit.<method>()`. To customize the default
  output, set `templates.evolog` instead of `templates.log`.

* `jj op show` now uses `templates.op_show` configuration for its default template
  instead of `templates.op_log`.

* The deprecated config option `git.auto-local-branch` has been removed. Use
  `git.auto-local-bookmark` instead.

* The deprecated `Signature.username()` template method has been removed. Use
  `Signature.email().local()` instead.

* The deprecated `--config-toml` flag has been removed. Use
  `--config=NAME=VALUE` or `--config-file=PATH` instead.

* `jj undo` can now undo multiple operations progressively by calling it
  repeatedly, whereas previously, running `jj undo` twice was previously a no-op
  (it only undid the last change).

* `jj git fetch` will now only fetch the refspec patterns configured on remotes
  when the `--bookmark` option is omitted. Only simple refspec patterns are
  currently supported, and anything else (like refspecs which rename branches)
  will be ignored.

* The `conflict` label used for coloring log graph nodes was renamed to
  `conflicted`.

### Deprecations

* The on-disk index format has changed. `jj` will write index files in both old
  and new formats, so old `jj` versions should be able to read these index
  files. This compatibility layer will be removed in a future release.

* `jj op undo` is deprecated in favor of `jj op revert`. (`jj undo` is still
  available, but with new semantics. See also the breaking changes above.)

* The argument `<operation>` of `jj undo` is deprecated in favor of
  `jj op revert <operation>`.

* The `--what` flag on `jj undo` is deprecated. Consider using
  `jj op restore --what` instead.

### New features

* The new command `jj redo` can progressively redo operations that were
  previously undone by multiple calls to `jj undo`.

* Templates now support `any()` and `all()` methods on lists to check whether
  any or all elements satisfy a predicate. Example: `parents.any(|c| c.mine())`
  returns true if any parent commit is authored by the user.

* Add experimental support for indexing changed paths, which will speed up `jj
  log PATH` query, `jj file annotate`, etc. The changed-path index can be
  enabled by `jj debug index-changed-paths` command. Indexing may take tens of
  minutes depending on the number of merge commits. The indexing command UI is
  subject to change. [#4674](https://github.com/jj-vcs/jj/issues/4674)

* `jj config list` now supports `-T'json(self) ++ "\n"'` serialization output.

* `jj file show` now accepts `-T`/`--template` option to insert file metadata.

* The template language now allows arbitrary whitespace between any operators.

* The new configuration option `git.colocate=boolean` controls whether or not
  Git repositories are colocated by default.

* Both `jj git clone` and `jj git init` now take a `--no-colocate` flag to
  disable colocation (in case `git.colocate` is set to `true`.)

* `jj git remote add` and `jj git clone` now support `--fetch-tags` to control
  when tags are fetched for all subsequent fetches.

* `jj git fetch` now supports `--tracked` to fetch only tracked bookmarks.

* `jj diff --stat` now shows the change in size to binary files.

* `jj interdiff`, `jj evolog -p`, and `jj op log -p` now show diff of commit
  descriptions.

* `jj log` and `jj op log` output can now be anonymized with the
  `builtin_log_redacted` and `builtin_op_log_redacted` templates.

* `jj git init` now checks for an `upstream` remote in addition to `origin` when
  setting the repository-level `trunk()` alias. The `upstream` remote takes
  precedence over `origin` if both exist.

* Add the `jj metaedit` command, which modifies a revision's metadata. This can
  be used to generate a new change-id, which may help resolve some divergences.
  It also has options to modify author name, email and timestamp, as well as to
  modify committer timestamp.

* Filesets now support case-insensitive glob patterns with the `glob-i:`,
  `cwd-glob-i:`, and `root-glob-i:` pattern kinds. For example, `glob-i:"*.rs"`
  will match both `file.rs` and `FILE.RS`.

* `jj op show` now accepts `-T`/`--template` option to customize the operation
  output using template expressions, similar to `jj op log`. Also added
  `--no-op-diff` flag to suppress the operation diff.

* A nearly identical string pattern system as revsets is now supported in the
  template language, and is exposed as `string.match(pattern)`.

* Merge tools can use the `$path` argument to learn where the file they
  are merging will end up in the repository.

### Fixed bugs

* `jj git clone` now correctly fetches all tags, unless `--fetch-tags` is
  explicitly specified, in which case the specified option will apply for both
  the initial clone and subsequent fetches.

* Operation and working-copy state files are now synchronized to disk on save.
  This will mitigate data corruption on system crash.
  [#4423](https://github.com/jj-vcs/jj/issues/4423)

### Packaging changes

* The test suite no longer optionally uses Taplo CLI or jq, and packagers can
  remove them as dependencies if present.

### Contributors

* Austin Seipp (@thoughtpolice)
* Benjamin Tan (@bnjmnt4n)
* Christian Hufnagel (@OvidiusCicero)
* Clément (@drawbu)
* Daniel Luz (@mernen)
* Emily (@emilazy)
* Evan Martin (@evmar)
* Gaëtan Lehmann (@glehmann)
* George Christou (@gechr)
* Graham Christensen (@grahamc)
* Hegui Dai (@Natural-selection1)
* Ian Wrzesinski (@isuffix)
* Ilya Grigoriev (@ilyagr)
* Isaac Corbrey (@icorbrey)
* Ivan Petkov (@ipetkov)
* Joaquín Triñanes (@JoaquinTrinanes)
* Kaiyi Li (@06393993)
* Martin von Zweigbergk (@martinvonz)
* Nigthknight (@nigthknight)
* Nikhil Marathe (@nikhilm)
* Remo Senekowitsch (@senekor)
* Tijs-B (@Tijs-B)
* Yuya Nishihara (@yuja)

## [0.32.0] - 2025-08-06

### Breaking changes

* In revsets, symbol expressions (such as change ID prefix) no longer resolve to
  multiple revisions, and error out if resolved to more than one revisions. Use
  `change_id(prefix)` or `bookmarks(exact:name)` to query divergent changes or
  conflicted bookmarks. Commands like `jj rebase` no longer require `all:` to
  specify multiple destination revisions.

* `jj op abandon` now discards previous versions of a change (or predecessors)
  if they become unreachable from the operation history. The evolution history
  is truncated accordingly.

  Once `jj op abandon` and `jj util gc` are run in a repository, old versions of
  `jj` might get "commit not found" error on `jj evolog`.

* `commit.working_copies()` template method now returns `List<WorkspaceRef>`

* The previously predefined `amend` alias has been removed. You can restore it
  by setting the config `aliases.amend = ["squash"]`.

### Deprecations

* The `all:` revset modifier and `ui.always-allow-large-revsets` setting is
  planned to be removed in a future release.
  [#6016](https://github.com/jj-vcs/jj/issues/6016)

* Rename the `core.fsmonitor` and `core.watchman` settings to
  `fsmonitor.backend`, and `fsmonitor.watchman` respectively.

### New features

* `jj workspace list` now accepts `-T`/`--template` option to customize its
  output via templates.

* Added `templates.workspace_list` template to customize the output of
  `jj workspace list`.

* `jj fix` now buffers the standard error stream from subprocesses and emits
  the output from each all at once. The file name is printed before the output.

* `jj status` now collapses fully untracked directories into one line.
  It still fully traverses them while snapshotting but they won't clutter up
  the output with all of their contents.

* Add the `working-copy.eol-conversion` config which is similar to the git
  `core.autocrlf` config. A heuristics is used to detect if a file is a binary
  file to prevent the EOL conversion from changing binary files unexpectedly.

* Add a `.parents()` method to the
  [`Operation`](docs/templates.md#operation-type) type in the templating
  language.

* Merge tools config can now explicitly forbid using them as diff editors or
  diff formatters. Built-in tools that do not function well as diff editing
  tools or as diff formatters will now report an error when used as such.

* `jj diffedit` now accepts filesets to edit only the specified paths.

* AnnotationLine objects in templates now have a `original_line_number() ->
  Integer` method.

* Commit templates now support `.files()` to list all existing files at that
  revision.

* Glob patterns now support `{foo,bar}` syntax. There may be subtle behavior
  changes as we use the [globset](https://crates.io/crates/globset) library now.

* The new `bisect(x)` revset function can help bisect a range of commits to
  find when a bug was introduced.

* New `first_parent()` and `first_ancestors()` revset functions which are
  similar to `parents()` and `ancestors()`, but only traverse the first parent
  of each commit (similar to Git's `--first-parent` option).

* New `signing.backends.ssh.revocation-list` config for specifying a list of revoked
  public keys for commit signature verification.

* `jj fix` commands now replace `$root` with the workspace's root path. This is
  useful for tools stored inside the workspace.

### Fixed bugs

* Fixed an error in `jj util gc` caused by the empty blob being missing from
  the Git store. [#7062](https://github.com/jj-vcs/jj/issues/7062)

* `jj op diff -p` and `jj op log -p` now show content diffs from the first
  predecessor only. [#7090](https://github.com/jj-vcs/jj/issues/7090)

* `jj git fetch` no longer shows `NaN%` progress when connecting to slow remotes.
  [#7155](https://github.com/jj-vcs/jj/issues/7155)

### Contributors

Thanks to the people who made this release happen!

* adamnemecek (@adamnemecek)
* Alexander Kobjolke (@jakalx)
* Apromixately (@Apromixately)
* Austin Seipp (@thoughtpolice)
* Bryce Berger (@bryceberger)
* Daniel Danner (@dnnr)
* Daniel Luz (@mernen)
* Evan Martin (@evmar)
* George Christou (@gechr)
* George Elliott-Hunter (@george-palmsens)
* Hubert Głuchowski (@afishhh)
* Ilya Grigoriev (@ilyagr)
* Jade Lovelace (@lf-)
* Jake Martin (@jake-m-commits)
* Jan Klass (@Kissaki)
* Joaquín Triñanes (@JoaquinTrinanes)
* Josh Steadmon (@steadmon)
* Kaiyi Li (@06393993)
* Martin von Zweigbergk (@martinvonz)
* Nigthknight (@nigthknight)
* Ori Avtalion (@salty-horse)
* Pablo Brasero (@pablobm)
* Pavan Kumar Sunkara (@pksunkara)
* Philip Metzger (@PhilipMetzger)
* phoebe (@phoreverpheebs)
* Remo Senekowitsch (@senekor)
* Scott Taylor (@scott2000)
* Stephen Jennings (@jennings)
* Theo Buehler (@botovq)
* Tyarel8 (@Tyarel8)
* Yuya Nishihara (@yuja)

## [0.31.0] - 2025-07-02

### Breaking changes

* Revset expressions like `hidden_id | description(x)` now [search the specified
  hidden revision and its ancestors](docs/revsets.md#hidden-revisions) as well
  as all visible revisions.

* Commit templates no longer normalize `description` by appending final newline
  character. Use `description.trim_end() ++ "\n"` if needed.

### Deprecations

* The `git.push-bookmark-prefix` setting is deprecated in favor of
  `templates.git_push_bookmark`, which supports templating. The old setting can
  be expressed in template as `"<prefix>" ++ change_id.short()`.

### New features

* New `change_id(prefix)`/`commit_id(prefix)` revset functions to explicitly
  query commits by change/commit ID prefix.

* The `parents()` and `children()` revset functions now accept an optional
  `depth` argument. For instance, `parents(x, 3)` is equivalent to `x---`, and
  `children(x, 3)` is equivalent to `x+++`.

* `jj evolog` can now follow changes from multiple revisions such as divergent
  revisions.

* `jj diff` now accepts `-T`/`--template` option to customize summary output.

* Log node templates are now specified in toml rather than hardcoded.

* Templates now support `json(x)` function to serialize values in JSON format.

* The ANSI 256-color palette can be used when configuring colors. For example,
  `colors."diff removed token" = { bg = "ansi-color-52", underline = false }`
  will apply a dark red background on removed words in diffs.

### Fixed bugs

* `jj file annotate` can now process files at a hidden revision.

* `jj op log --op-diff` no longer fails at displaying "reconcile divergent
  operations." [#4465](https://github.com/jj-vcs/jj/issues/4465)

* `jj util gc --expire=now` now passes the corresponding flag to `git gc`.

* `change_id`/`commit_id.shortest()` template functions now take conflicting
  bookmark and tag names into account.
  [#2416](https://github.com/jj-vcs/jj/issues/2416)

* Fixed lockfile issue on stale file handles observed with NFS.

### Packaging changes

* `aarch64-windows` builds (release binaries and `main` snapshots) are now provided.

### Contributors

Thanks to the people who made this release happen!

* Anton Älgmyr (@algmyr)
* Austin Seipp (@thoughtpolice)
* Benjamin Brittain (@benbrittain)
* Cyril Plisko (@imp)
* Daniel Luz (@mernen)
* Gaëtan Lehmann (@glehmann)
* Gilad Woloch (@giladwo)
* Greg Morenz (@gmorenz)
* Igor Velkov (@iav)
* Ilya Grigoriev (@ilyagr)
* Jade Lovelace (@lf-)
* Jonas Greitemann (@jgreitemann)
* Josh Steadmon (@steadmon)
* juemrami (@juemrami)
* Kaiyi Li (@06393993)
* Lars Francke (@lfrancke)
* Martin von Zweigbergk (@martinvonz)
* Osama Qarem (@osamaqarem)
* Philip Metzger (@PhilipMetzger)
* raylu (@raylu)
* Scott Taylor (@scott2000)
* Vincent Ging Ho Yim (@cenviity)
* Yuya Nishihara (@yuja)


## [0.30.0] - 2025-06-04

### Release highlights

* The experimental support from release 0.29.0 for transferring the change ID
  to/from Git remotes has been enabled by default. The change ID is stored in
  the Git commit itself (in a commit header called `change-id`), which means
  it will be transferred by regular `git push` etc. Please let us know if you
  run into any problems with it. You can disable it setting
  `git.write-change-id-header`. Note that some Git remotes (e.g GitLab) and
  some Git commands (e.g. `git rebase`) do not preserve the change ids when
  they rewrite commits.

* `jj rebase` now automatically abandons divergent commits if another commit
  with the same change ID is already present in the destination with identical
  changes.

* `jj split` has gained `--message`, `--insert-before`, `--insert-after`, and
  `--destination` options.

* `jj evolog` can show associated operations for commits created by new jj
  versions.

### Breaking changes

* The old `libgit2` code path for fetches and pushes has been removed,
  and the `git.subprocess` setting along with it.

* In templates, bookmark/tag/remote names are now formatted in revset symbol
  notation. The type of `bookmark.remote()` is changed to `Option<_>`.
  `bookmark.remote() == "foo"` still works, but `bookmark.remote().<method>()`
  might need `if(bookmark.remote(), ..)` to suppress error.

* `jj rebase` now automatically abandons divergent commits if another commit
  with the same change ID is already present in the destination with identical
  changes. To keep these divergent commits, use the `--keep-divergent` flag.

* The deprecated `--skip-empty` flag for `jj rebase` has been removed. Use the
  `--skip-emptied` flag instead.

* The deprecated `jj branch` subcommands have been removed. Use the `jj
  bookmark` subcommands instead.

* `jj util completion` now requires the name of the shell as a positional
  argument and no longer produces Bash completions by default. The deprecated
  optional arguments for different shells have been removed.

* External diff tools are now run in the temporary directory containing
  the before (`left`) and after (`right`) directories, making diffs appear
  more pleasing for tools that display file paths prominently. Users can
  opt out of this by setting `merge-tools.<tool>.diff-do-chdir = false`,
  but this will likely be removed in a future release. Please report any
  issues you run into.

* The minimum supported Rust version (MSRV) is now 1.85.0.

### Deprecations

* The `ui.diff.format` and `ui.diff.tool` config options have been merged as
  `ui.diff-formatter`. The builtin format can be specified as `:<format>`
  (e.g. `ui.diff-formatter=":git"` for Git diffs.)

* The `.normal_hex()` method will be removed from the `CommitId` template type.
  It's useful only for the `ChangeId` type.

### New features

* `jj split` has gained a `--message` option to set the description of the
  commit with the selected changes.

* `jj split` has gained the ability to place the revision with the selected
  changes anywhere in the revision tree with the `--insert-before`,
  `--insert-after` and `--destination` command line flags.

* Added `git.track-default-bookmark-on-clone` setting to control whether to
  track the default remote bookmark on `jj git clone`.

* Templates can now do arithmetic on integers with the `+`, `-`, `*`, `/`, and
  `%` infix operators.

* Evolution history is now stored in the operation log. `jj evolog` can show
  associated operations for commits created by new jj versions.

### Fixed bugs

* Work around a git issue that could cause subprocess operations to hang if the
  `core.fsmonitor` gitconfig is set in the global or system gitconfigs.
  [#6440](https://github.com/jj-vcs/jj/issues/6440)

* `jj parallelize` can now parallelize groups of changes that _start_ with an
  immutable change, but do not contain any other immutable changes.

* `jj` will no longer warn about deprecated paths on macOS if the configured
  XDG directory is the deprecated one (~/Library/Application Support).

* The builtin diff editor now correctly handles splitting changes where a file
  is replaced by a directory of the same name.
  [#5189](https://github.com/jj-vcs/jj/issues/5189)

### Packaging changes

* Due to the removal of the `libgit2` code path, packagers should remove any
  dependencies on `libgit2`, `libssh2`, Zlib, OpenSSL, and `pkg-config`, and
  ensure they are not setting the Cargo `git2` or `vendored-openssl` features.

### Contributors

Thanks to the people who made this release happen!

* Alper Cugun (@alper)
* Austin Seipp (@thoughtpolice)
* Benjamin Brittain (@benbrittain)
* Benjamin Tan (@bnjmnt4n)
* Bryce Berger (@bryceberger)
* Colin Nelson (@orthros)
* Doug Stephen (@dljsjr)
* Emily (@emilazy)
* Eyvind Bernhardsen (@eyvind)
* Felix Geisendörfer (@felixge)
* Gaëtan Lehmann (@glehmann)
* Ilya Grigoriev (@ilyagr)
* Isaac Corbrey (@icorbrey)
* Jonas Greitemann (@jgreitemann)
* Josep Mengual (@truita)
* kkoang (@kkoang)
* Manuel Mendez (@mmlb)
* Marshall Bowers (@maxdeviant)
* Martin von Zweigbergk (@martinvonz)
* Mateus Auler (@mateusauler)
* Michael Pratt (@prattmic)
* Nicole Patricia Mazzuca (@strega-nil)
* Philip Metzger (@PhilipMetzger)
* Scott Taylor (@scott2000)
* T6 (@tjjfvi)
* Vincent Ging Ho Yim (@cenviity)
* Winter (@winterqt)
* Yuya Nishihara (@yuja)

## [0.29.0] - 2025-05-07

### Release highlights

* Experimental support for transferring the change ID to/from Git remotes behind configuration
  setting `git.write-change-id-header`. If this is enabled, the change ID will be stored in the Git
  commit itself (in a commit header called `change-id`), which means it will be transferred by
  regular `git push` etc. This is an evolving feature that currently defaults to "false". This
  default will likely change in the future as we gain confidence with forge support and user
  expectations.

### Breaking changes

* `jj git push -c`/`--change` no longer moves existing local bookmarks.

* The `editor-*.jjdescription` files passed to your editor by e.g. `jj describe`
  are now written to your system's temporary directory instead of `.jj/repo/`.

### Deprecations

* `git.subprocess = false` has been deprecated, and the old `libgit2`
  code path for fetches and pushes will be removed entirely in 0.30.
  Please report any remaining issues you have with the Git
  subprocessing path.

* `ui.default-description` has been deprecated, and will be migrated to
  `template-aliases.default_commit_description`. Please also consider using
  [`templates.draft_commit_description`](docs/config.md#default-description),
  and/or [`templates.commit_trailers`](docs/config.md#commit-trailers).

* On macOS, config.toml files in `~/Library/Application Support/jj` are
  deprecated; one should instead use `$XDG_CONFIG_HOME/jj`
  (defaults to `~/.config/jj`)

### New features

* Color-words diff has gained [an option to compare conflict pairs without
  materializing](docs/config.md#color-words-diff-options).

* `jj show` patches can now be suppressed with `--no-patch`.

* Added `ui.bookmark-list-sort-keys` setting to configure default sort keys for the
  `jj bookmark list` command.

* New `signed` revset function to filter for cryptographically signed commits.

* `jj describe`, `jj commit`, `jj new`, `jj squash` and `jj split` add the
  commit trailers, configured in the `commit_trailers` template, to the commit
  description. Use cases include DCO Sign Off and Gerrit Change Id.

* Added `duplicate_description` template, which allows [customizing the descriptions
  of the commits `jj duplicate` creates](docs/config.md#duplicate-commit-description).

* `jj absorb` can now squash a deleted file if it was added by one of the
  destination revisions.

* Added `ui.streampager.show-ruler` setting to configure whether the ruler should be
  shown when the builtin pager starts up.

* `jj git fetch` now warns instead of erroring for unknown `git.fetch` remotes
  if other remotes are available.

* Commit objects in templates now have `trailers() -> List<Trailer>`, the Trailer
  objects have `key() -> String` and `value() -> String`.

* `jj config edit` will now roll back to previous version if a syntax error has been introduced in the new config.

* When using dynamic command-line completion, revision names will be completed
  in more complex expressions. For example, typing
  `jj log -r first-bookmark..sec` and then pressing Tab could complete the
  expression to `first-bookmark..second-bookmark`.

### Fixed bugs

* Fixed crash on change-delete conflict resolution.
  [#6250](https://github.com/jj-vcs/jj/issues/6250)

* The builtin diff editor now tries to preserve unresolved conflicts.
  [#4963](https://github.com/jj-vcs/jj/issues/4963)

* Fixed bash and zsh shell completion when completing aliases of multiple arguments.
  [#5377](https://github.com/jj-vcs/jj/issues/5377)

### Packaging changes

* Jujutsu now uses
  [`zlib-rs`](https://github.com/trifectatechfoundation/zlib-rs), a
  fast compression library written in Rust. Packagers should remove any
  dependency on CMake and drop the `packaging` Cargo feature.

### Contributors

Thanks to the people who made this release happen!

* Aleksey Kuznetsov (@zummenix)
* Austin Seipp (@thoughtpolice)
* Benjamin Brittain (@benbrittain)
* Benjamin Tan (@bnjmnt4n)
* Caleb White (@calebdw)
* Daniel Luz (@mernen)
* Emily (@emilazy)
* Emily (@neongreen)
* Gaëtan Lehmann (@glehmann)
* George Christou (@gechr)
* Ilya Grigoriev (@ilyagr)
* Jacob Hayes (@JacobHayes)
* Jonas Greitemann (@jgreitemann)
* Josh Steadmon (@steadmon)
* Martin von Zweigbergk (@martinvonz)
* Mateus Auler (@mateusauler)
* Nicole Patricia Mazzuca (@strega-nil)
* Nils Koch (@nilskch)
* Philip Metzger (@PhilipMetzger)
* Remo Senekowitsch (@senekor)
* Sam (@Samasaur1)
* Steve Fink (@hotsphink)
* Théo Daron (@tdaron)
* TimerErTim (@TimerErTim)
* Vincent Ging Ho Yim (@cenviity)
* Winter (@winterqt)
* Yuya Nishihara (@yuja)

## [0.28.2] - 2025-04-07

### Fixed bugs

* Fixed problem that old commits could be re-imported from Git.
  https://github.com/GitoxideLabs/gitoxide/issues/1928

## [0.28.1] - 2025-04-04

### Security fixes

* Fixed SHA-1 collision attacks not being detected.
  ([GHSA-794x-2rpg-rfgr](https://github.com/jj-vcs/jj/security/advisories/GHSA-794x-2rpg-rfgr))

### Fixed bugs

* Resolved some potential build issues for packagers.
  [#6232](https://github.com/jj-vcs/jj/pull/6232)

* Fix a bug with `:ours` and `:theirs` merge tools involving conflicted trees
  with more than two sides. [#6227](https://github.com/jj-vcs/jj/pull/6227)

### Contributors

Thanks to the people who made this release happen!

* Emily (@emilazy)
* Ilya Grigoriev (@ilyagr)
* Nicole Patricia Mazzuca (@strega-nil)
* Scott Taylor (@scott2000)
* Yuya Nishihara (@yuja)

## [0.28.0] - 2025-04-02

### Release highlights

* jj's configuration can now be split into multiple files more easily.

* `jj resolve` now accepts built-in tools `:ours` and `:theirs`.

* In colocated repos, newly-created files will now appear in `git diff`.

* A long-standing bug relating to empty files in the built-in diff editor was
  fixed. [#3702](https://github.com/jj-vcs/jj/issues/3702)

### Breaking changes

* The minimum supported Rust version (MSRV) is now 1.84.0.

* The `git.push-branch-prefix` config has been removed in favor of
  `git.push-bookmark-prefix`.

* `jj abandon` no longer supports `--summary` to suppress the list of abandoned
  commits. The list won't show more than 10 commits to not clutter the console.

* `jj unsquash` has been removed in favor of `jj squash` and
  `jj diffedit --restore-descendants`.

* The `jj untrack` subcommand has been removed in favor of `jj file untrack`.

* The following deprecated revset functions have been removed:
  - `branches()`, `remote_branches()`, `tracked_remote_branches()`, and
    `untracked_remote_branches()`, which were renamed to "bookmarks".
  - `file()` and `conflict()`, which were renamed to plural forms.
  - `files(x, y, ..)` with multiple patterns. Use `files(x|y|..)` instead.

* The following deprecated template functions have been removed:
  - `branches()`, `local_branches()`, and `remote_branches()`, which were
    renamed to "bookmarks".

* The flags `--all` and `--tracked` on `jj git push` by themself do not cause
  deleted bookmarks to be pushed anymore, as an additional safety measure. They
  can now be combined with `--deleted` instead.

### Deprecations

* `core.watchman.register_snapshot_trigger` has been renamed to `core.watchman.register-snapshot-trigger` for consistency with other configuration options.

* `jj backout` is deprecated in favor of `jj revert`.

### New features

* `jj sign` can now sign with PKCS#12 certificates through the `gpgsm` backend.

* `jj sign` will automatically use the gpg key associated with the author's email
  in the absence of a `signing.key` configuration.

* Multiple user configs are now supported and are loaded in the following precedence order:
  - `$HOME/.jjconfig.toml`
  - `$XDG_CONFIG_HOME/jj/config.toml`
  - `$XDG_CONFIG_HOME/jj/conf.d/*.toml`

* The `JJ_CONFIG` environment variable can now contain multiple paths separated
  by a colon (or semicolon on Windows).

* The command `jj config list` now supports showing the origin of each variable
  via the `builtin_config_list_detailed` template.

* `jj config {edit,set,unset}` now prompt when multiple config files are found.

* `jj diff -r` now allows multiple revisions (as long as there are no gaps in
  the revset), such as `jj diff -r 'mutable()'`.

* `jj git push` now accepts a `--named NAME=REVISION` argument to create a named
  bookmark and immediately push it.

* The 'how to resolve conflicts' hint that is shown when conflicts appear can
  be hidden by setting `hints.resolving-conflicts = false`.

* `jj op diff` and `jj op log --op-diff` now show changes to which commits
  correspond to working copies.

* `jj op log -d` is now an alias for `jj op log --op-diff`.

* `jj bookmark move --to/--from` can now be abbreviated to `jj bookmark move -t/-f`

* `jj bookmark list` now supports `--sort` option. Similar to `git branch --sort`.
  See `jj bookmark list --help` for more details.

* A new command `jj revert` is added, which is similar to `jj backout` but
  adds the `--destination`, `--insert-after`, and `--insert-before` options to
  customize the location of reverted commits.

* A new command `jj git root` is added, which prints the location of the Git
  directory of a repository using the Git backend.

* In colocated repos, any files that jj considers added in the working copy will
  now show up in `git diff` (as if you had run `git add --intent-to-add` on
  them).

* Reversing colors is now supported. For example, to highlight words by
  reversing colors rather than underlining, you can set
  `colors."diff token"={ underline = false, reverse = true }` in your config.

* Added `revsets.log-graph-prioritize`, which can be used to configure
  which branch in the `jj log` graph is displayed on the left instead of `@`
  (e.g. `coalesce(description("megamerge\n"), trunk())`)

* `jj resolve` now accepts new built-in merge tools `:ours` and `:theirs`.
  These merge tools accept side #1 and side #2 of the conflict respectively.

### Fixed bugs

* `jj log -p --stat` now shows diff stats as well as the default color-words/git
  diff output. [#5986](https://github.com/jj-vcs/jj/issues/5986)

* The built-in diff editor now correctly handles deleted files.
  [#3702](https://github.com/jj-vcs/jj/issues/3702)

* The built-in diff editor now correctly retains the executable bit on newly
  added files when splitting. [#3846](https://github.com/jj-vcs/jj/issues/3846)

* `jj config set`/`--config` value parsing rule is relaxed in a way that
  unquoted apostrophes are allowed.
  [#5748](https://github.com/jj-vcs/jj/issues/5748)

* `jj fix` could previously create new conflicts when a descendant of a fixed
  revision was already correctly formatted.

### Contributors

Thanks to the people who made this release happen!

* Aleksey Kuznetsov (@zummenix)
* Anton Älgmyr (@algmyr)
* Austin Seipp (@thoughtpolice)
* Baltasar Dinis (@bsdinis)
* Benjamin Tan (@bnjmnt4n)
* Brandon Hall (@tenkabuto)
* Caleb White (@calebdw)
* Daniel Luz (@mernen)
* David Rieber (@drieber)
* demize (@demize)
* Emily (@emilazy)
* Evan Mesterhazy (@emesterhazy)
* Fedor Sheremetyev (@sheremetyev)
* George Christou (@gechr)
* Ilya Grigoriev (@ilyagr)
* Jakob Hellermann (@jakobhellermann)
* Jo Liss (@joliss)
* Joachim Desroches (@jedesroches)
* Johannes Altmanninger (@krobelus)
* Jonathan Gilchrist (@jgilchrist)
* Kenyon Ralph (@kenyon)
* Lucas Garron (@lgarron)
* Martin von Zweigbergk (@martinvonz)
* Nick Pupko (@npupko)
* Philip Metzger (@PhilipMetzger)
* Raphael Borun Das Gupta (@das-g)
* Remo Senekowitsch (@senekor)
* Robin Stocker (@robinst)
* Scott Taylor (@scott2000)
* Siva Mahadevan (@svmhdvn)
* Vincent Ging Ho Yim (@cenviity)
* Yuya Nishihara (@yuja)

## [0.27.0] - 2025-03-05

### Release highlights

* `git.subprocess` is now enabled by default, improving compatibility with Git
  fetches and pushes by spawning an external `git` process. Users can opt out
  of this by setting `git.subprocess = false`, but this will likely be removed
  in a future release. Please report any issues you run into.

### Breaking changes

* Bookmark name to be created/updated is now parsed as [a revset
  symbol](docs/revsets.md#symbols). Quotation may be needed in addition to shell
  quotes. Example: `jj bookmark create -r@- "'name with space'"`

* `jj bookmark create`, `jj bookmark set` and `jj bookmark move` onto a hidden
   commit make it visible.

* `jj bookmark forget` now untracks any corresponding remote bookmarks instead
  of forgetting them, since forgetting a remote bookmark can be unintuitive.
  The old behavior is still available with the new `--include-remotes` flag.

* `jj fix` now always sets the working directory of invoked tools to be the
  workspace root, instead of the working directory of the `jj fix`.

* The `ui.allow-filesets` configuration option has been removed.
  [The "fileset" language](docs/filesets.md) has been enabled by default since v0.20.

* `templates.annotate_commit_summary` is renamed to `templates.file_annotate`,
  and now has an implicit `self` parameter of type `AnnotationLine`, instead of
  `Commit`. All methods on `Commit` can be accessed with `commit.method()`, or
  `self.commit().method()`.

### Deprecations

* This release takes the first steps to make target revision required in
  `bookmark create`, `bookmark move` and `bookmark set`. Those commands will display
  a warning if the user does not specify target revision  explicitly. In the near
  future those commands will fail if target revision is not specified.

* The `signing.sign-all` config option has been deprecated in favor of
  `signing.behavior`. The new option accepts `drop` (never sign), `keep` (preserve
  existing signatures), `own` (sign own commits), or `force` (sign all commits).
  Existing `signing.sign-all = true` translates to `signing.behavior = "own"`, and
  `false` translates to `"keep"`. Invalid configuration is now an error.

### New features

* The new `jj sign` and `jj unsign` commands allow for signing/unsigning commits.
  `jj sign` supports configuring the default revset through `revsets.sign` when
  no `--revisions` arguments are provided.

* `jj git fetch` now supports [string pattern syntax](docs/revsets.md#string-patterns)
  on `--remote` option and `git.fetch` configuration.

* Template functions `truncate_start()` and `truncate_end()` gained an optional
  `ellipsis` parameter; passing this prepends or appends the ellipsis to the
  content if it is truncated to fit the maximum width.

* Templates now support `stringify(x)` function and string method
  `.escape_json()`. The latter serializes the string in JSON format. It is
  useful for making machine-readable templates by escaping problematic
  characters like `\n`.

* Templates now support `trim()`, `trim_start()` and `trim_end()` methods
  which remove whitespace from the start and end of a `String` type.

* The description of commits backed out by `jj backout` can now be configured
  using `templates.backout_description`.

* New `AnnotationLine` templater type. Used in `templates.file_annotate`.
  Provides `self.commit()`, `.content()`, `.line_number()`, and
  `.first_line_in_hunk()`.

* Templates now have `format_short_operation_id(id)` function for users to
  customize the default operation id representation.

* The `jj init`/`jj revert` stubs that print errors can now be overridden with
  aliases. All of `jj clone/init/revert` add a hint to a generic error.

* Help text is now colored (when stdout is a terminal).

* Commands that used to suggest `--ignore-immutable` now print the number of
  immutable commits that would be rewritten if used and a link to the docs.

* `jj undo` now shows a hint when undoing an undo operation that the user may
   be looking for `jj op restore` instead.

### Fixed bugs

* `jj status` now shows untracked files under untracked directories.
  [#5389](https://github.com/jj-vcs/jj/issues/5389)

* Added workaround for the bug that untracked files are ignored when watchman is
  enabled. [#5728](https://github.com/jj-vcs/jj/issues/5728)

* The `signing.backends.ssh.allowed-signers` configuration option will now
  expand `~/` to `$HOME/`.
  [#5626](https://github.com/jj-vcs/jj/pull/5626)

* `config-schema.json` now allows arrays of strings for the settings `ui.editor`
  and `ui.diff.tool`.

* `config-schema.json` now allows an array of strings or nested table for the
  `ui.pager` setting.

### Contributors

Thanks to the people who made this release happen!

* Alain Leufroy (@aleufroy)
* Aleksey Kuznetsov (@zummenix)
* Alexander Mikhailov (@AM5800)
* Andrew Gilbert (@andyg0808)
* Antoine Martin (@alarsyo)
* Anton Bulakh (@necauqua)
* Austin Seipp (@thoughtpolice)
* Baltasar Dinis (@bsdinis)
* Benjamin Tan (@bnjmnt4n)
* Bryce Berger (@bryceberger)
* Burak Varlı (@unexge)
* David Rieber (@drieber)
* Emily (@emilazy)
* Evan Mesterhazy (@emesterhazy)
* George Christou (@gechr)
* HKalbasi (@HKalbasi)
* Ilya Grigoriev (@ilyagr)
* Jacob Hayes (@JacobHayes)
* Jonathan Frere (@MrJohz)
* Jonathan Tan (@jonathantanmy)
* Josh Steadmon (@steadmon)
* maan2003 (@maan2003)
* Martin von Zweigbergk (@martinvonz)
* Matthew Davidson (@KingMob)
* Philip Metzger (@PhilipMetzger)
* Philipp Albrecht (@pylbrecht)
* Roman Timushev (@rtimush)
* Samuel Tardieu (@samueltardieu)
* Scott Taylor (@scott2000)
* Stephan Hügel (@urschrei)
* Vincent Ging Ho Yim (@cenviity)
* Yuya Nishihara (@yuja)

## [0.26.0] - 2025-02-05

### Release highlights

* Improved Git push/fetch compatibility by spawning an external `git` process.
  This can be enabled by the `git.subprocess=true` config knob, and will be the
  default in a future release.

* `jj log` can now show cryptographic commit signatures. The output can be
  controlled by the `ui.show-cryptographic-signatures=true` config knob.

### Breaking changes

* `jj abandon` now deletes bookmarks pointing to the revisions to be abandoned.
  Use `--retain-bookmarks` to move bookmarks backwards. If deleted bookmarks
  were tracking remote bookmarks, the associated bookmarks (or branches) will be
  deleted from the remote on `jj git push --all`.
  [#3505](https://github.com/jj-vcs/jj/issues/3505)

* `jj init --git` and `jj init --git-repo` have been removed. They were
  deprecated in early 2024. Use `jj git init` instead.

* The following deprecated commands have been removed:
  - `jj cat` is replaced by `jj file show`.
  - `jj chmod` is replaced by `jj file chmod`.
  - `jj files` is replaced by `jj file list`.

* The deprecated `-l` short alias for `--limit` in `jj log`, `jj op log`
  and `jj obslog` has been removed. The `-n` short alias can be used instead.

* The deprecated `--siblings` options for `jj split` has been removed.
  `jj split --parallel` can be used instead.

* The deprecated `fix.tool-command` config option has been removed.

* In colocated repos, the Git index now contains the changes from all parents
  of the working copy instead of just the first parent (`HEAD`). 2-sided
  conflicts from the merged parents are now added to the Git index as conflicts
  as well.

* The following change introduced in 0.25.0 is reverted:
  - `jj config list` now prints inline tables `{ key = value, .. }` literally.
    Inner items of inline tables are no longer merged across configuration
    files.

* `jj resolve` will now attempt to resolve all conflicted files instead of
  resolving the first conflicted file. To resolve a single file, pass a file
  path to `jj resolve`.

* `jj util mangen` is replaced with `jj util install-man-pages`, which can
  install man pages for all `jj` subcommands to a given path.

* In `jj config list` template, `value` is now typed as `ConfigValue`, not as
  `String` serialized in TOML syntax.

* `jj git remote add`/`set-url` now converts relative Git remote path to
  absolute path.

* `jj log`/`op log` now applies `-n`/`--limit` *before* the items are reversed.
  Rationale: It's more useful to see the N most recent commits/operations, and
  is more performant. The old behavior can be achieved by `jj log .. | head`.
  [#5403](https://github.com/jj-vcs/jj/issues/5403)

* Upgraded `scm-record` from v0.4.0 to v0.5.0. See release notes at
  <https://github.com/arxanas/scm-record/releases/tag/v0.5.0>.

* The builtin pager is switched to
  [streampager](https://github.com/markbt/streampager/). It can handle large
  inputs better and can be configured.

* Conflicts materialized in the working copy before `jj 0.19.0` may no longer
  be parsed correctly. If you are using version 0.18.0 or earlier, check out a
  non-conflicted commit before upgrading to prevent issues.

### Deprecations

### New features

* `jj git {push,clone,fetch}` can now spawn an external `git` subprocess, via
   the `git.subprocess = true` config knob. This provides an alternative that,
   when turned on, fixes SSH bugs when interacting with Git remotes due to
   `libgit2`s limitations [#4979](https://github.com/jj-vcs/jj/issues/4979).

* `jj describe` now accepts `--edit`.

* `jj evolog` and `jj op log` now accept `--reversed`.

* `jj restore` now supports `-i`/`--interactive` selection.

* `jj file list` now supports templating.

* There is a new `builtin_op_log_oneline` template alias you can pass to `jj op
  log -T` for a more compact output. You can use `format_operation_oneline` and
  `format_snapshot_operation_oneline` to customize parts of it.

* New template function `config(name)` to access to configuration variable from
  template.

* New template function `pad_centered()` to center content within a minimum
  width.

* Templater now supports `list.filter(|x| ..)` method.

* The `diff` commit template keyword now supports custom formatting via
  `diff.files()`. For example, `diff.files().map(|e| e.path().display())` prints
  changed file paths.

* The `diff.stat()` template method now provides methods to get summary values.

* `jj log` can now show cryptographic commit signatures. The output can be
  controlled by the `ui.show-cryptographic-signatures=true` config knob. The
  signature template can be customized using
  `format_detailed_cryptographic_signature(signature)` and
  `format_short_cryptographic_signature(signature)`.

* New `git.sign-on-push` config option to automatically sign commits which are
  being pushed to a Git remote.

* New `git.push-new-bookmarks` config option to push new bookmarks without
  `--allow-new`.

* `jj status` now shows untracked files when they reside directly under a
  tracked directory. There's still an issue that files under untracked
  directories aren't listed. [#5389](https://github.com/jj-vcs/jj/issues/5389)

* New `merge-tools.<TOOL>.diff-expected-exit-codes` config option to suppress
  warnings from tools exiting with non-zero exit codes.

* New `fix.tools.TOOL.enabled` config option to enable/disable tools. This is
  useful for defining disabled tools in user configuration that can be enabled
  in individual repositories with one config setting.

* Added `--into` flag to `jj restore`, similarly to `jj squash` and `jj
  absorb`. It is equivalent to `--to`, but `--into` is the recommended name.

* Italic text is now supported. You can set e.g. `colors.error = { fg = "red",
  italic = true }` in your config.

* New `author_name`/`author_email`/`committer_name`/`committer_email(pattern)`
  revset functions to match either name or email field explicitly.

* New `subject(pattern)` revset function that matches first line of commit
  descriptions.

* Conditional configuration now supports `--when.commands` to change
  configuration based on subcommand.

* The Jujutsu documentation site now publishes a schema for the official
  configuration file, which can be integrated into your editor for autocomplete,
  inline errors, and more.
  Please [see the documentation](docs/config.md#json-schema-support) for more
  on this.

### Fixed bugs

* `jj git fetch` with multiple remotes will now fetch from all remotes before
  importing refs into the jj repo. This fixes a race condition where the
  treatment of a commit that is found in multiple fetch remotes depended on the
  order the remotes were specified.

* Fixed diff selection by external tools with `jj split`/`commit -i FILESETS`.
  [#5252](https://github.com/jj-vcs/jj/issues/5252)

* Conditional configuration now applies when initializing new repository.
  [#5144](https://github.com/jj-vcs/jj/issues/5144)

* `[diff.<format>]` configuration now applies to `.diff().<format>()` commit
  template methods.

* Conflicts at the end of files which don't end with a newline character are
  now materialized in a way that can be parsed correctly.
  [#3968](https://github.com/jj-vcs/jj/issues/3968)

* Bookmark and remote names written by `jj git clone` to
  `revset-aliases.'trunk()'` are now escaped if necessary.
  [#5359](https://github.com/jj-vcs/jj/issues/5359)

### Contributors

Thanks to the people who made this release happen!

* Angel Ezquerra (@AngelEzquerra)
* Antoine Martin (@alarsyo)
* Anton Bulakh (@necauqua)
* Austin Seipp (@thoughtpolice)
* Baltasar Dinis (@bsdinis)
* Benjamin Tan (@bnjmnt4n)
* blinry (@blinry)
* Bryce Berger (@bryceberger)
* Charlie-83 (@Charlie-83)
* Christian Stoitner (@cstoitner)
* Evan Martin (@evmar)
* George Christou (@gechr)
* Ilya Grigoriev (@ilyagr)
* Jakob Hellermann (@jakobhellermann)
* JDSeiler (@JDSeiler)
* Jonathan Frere (@MrJohz)
* Jonathan Gilchrist (@jgilchrist)
* Josh Steadmon (@steadmon)
* Martin von Zweigbergk (@martinvonz)
* Matt Kulukundis (@fowles)
* Ollivier Robert (@keltia)
* Philip Metzger (@PhilipMetzger)
* Philipp Albrecht (@pylbrecht)
* Robert Jackson (@rwjblue)
* Samuel Tardieu (@samueltardieu)
* Scott Taylor (@scott2000)
* Stephen Jennings (@jennings)
* Valentin Gatien-Baron (@v-gb)
* Vincent Ging Ho Yim (@cenviity)
* Waleed Khan (@arxanas)
* Yuya Nishihara (@yuja)

## [0.25.0] - 2025-01-01

### Release highlights

It's the holidays, and this release was overall pretty quiet, without many major
changes. Two select improvements:

* Improvements to configuration management, including support for [conditional
  variables](docs/config.md#conditional-variables) in config files.

* Large files in the working copy will no longer cause commands to fail; instead
  the large files will remain intact but untracked in the working copy.

### Breaking changes

* Configuration variables are no longer "stringly" typed. For example, `true` is
  not converted to a string `"true"`, and vice versa.

* The following configuration variables are now parsed strictly:
  `colors.<labels>`, `git.abandon-unreachable-commits`,
  `git.auto-local-bookmark`, `git.push-bookmark-prefix`, `revsets.log`,
  `revsets.short-prefixes`, `signing.backend`, `operation.hostname`,
  `operation.username`, `ui.allow-init-native`, `ui.color`,
  `ui.default-description`, `ui.progress-indicator`, `ui.quiet`, `user.email`,
  `user.name`

* `jj config list` now prints inline tables `{ key = value, .. }` literally.
  Inner items of inline tables are no longer merged across configuration files.
  See [the table syntax
  documentation](docs/config.md#dotted-style-and-headings) for
  details.

* `jj config edit --user` now opens a file even if `$JJ_CONFIG` points to a
  directory. If there are multiple config files, the command will fail.

* `jj config set` no longer accepts a bare string value that looks like a TOML
  expression. For example, `jj config set NAME '[foo]'` must be quoted as `jj
  config set NAME '"[foo]"'`.

* The deprecated `[alias]` config section is no longer respected. Move command
  aliases to the `[aliases]` section.

* `jj absorb` now abandons the source commit if it becomes empty and has no
  description.

### Deprecations

* `--config-toml=TOML` is deprecated in favor of `--config=NAME=VALUE` and
  `--config-file=PATH`.

* The `Signature.username()` template method is deprecated for
  `Signature.email().local()`.

### New features

* `jj` command no longer fails due to new working-copy files larger than the
  `snapshot.max-new-file-size` config option. It will print a warning and large
  files will be left untracked.

* Configuration files now support [conditional
  variables](docs/config.md#conditional-variables).

* New command options `--config=NAME=VALUE` and `--config-file=PATH` to set
  string value without quoting and to load additional configuration from files.

* Templates now support the `>=`, `>`, `<=`, and `<` relational operators for
  `Integer` types.

* A new Email template type is added. `Signature.email()` now returns an Email
  template type instead of a String.

* Adds a new template alias `commit_timestamp(commit)` which defaults to the
  committer date.

* Conflict markers are now allowed to be longer than 7 characters, allowing
  conflicts to be materialized and parsed correctly in files which already
  contain lines that look like conflict markers.

* New `$marker_length` variable to allow merge tools to support longer conflict
  markers (equivalent to "%L" for Git merge drivers).

* `jj describe` now accepts a `JJ: ignore-rest` line that ignores everything
  below it, similar to a "scissor line" in git. When editing multiple commits,
  only ignore until the next `JJ: describe` line.

### Fixed bugs

* The `$NO_COLOR` environment variable must now be non-empty to be respected.

* Fixed incompatible rendering of empty hunks in git/unified diffs.
  [#5049](https://github.com/jj-vcs/jj/issues/5049)

* Fixed performance of progress bar rendering when fetching from Git remote.
  [#5057](https://github.com/jj-vcs/jj/issues/5057)

* `jj config path --user` no longer creates new file at the default config path.

* On Windows, workspace paths (printed by `jj root`) no longer use UNC-style
  `\\?\` paths unless necessary.

* On Windows, `jj git clone` now converts local Git remote path to
  slash-separated path.

* `jj resolve` no longer removes the executable bit on resolved files when using
  an external merge tool.

### Contributors

Thanks to the people who made this release happen!

* Alex Stefanov (@umnikos)
* Anton Älgmyr (@algmyr)
* Austin Seipp (@thoughtpolice)
* Benjamin Tan (@bnjmnt4n)
* Bryce Berger (@bryceberger)
* Daniel Ploch (@torquestomp)
* David Crespo (@david-crespo)
* George Tsiamasiotis (@gtsiam)
* Jochen Kupperschmidt (@homeworkprod)
* Keane Nguyen (@keanemind)
* Martin von Zweigbergk (@martinvonz)
* Matt Kulukundis (@fowles)
* Milo Moisson (@mrnossiom)
* petricavalry (@petricavalry)
* Philip Metzger (@PhilipMetzger)
* Remo Senekowitsch (@senekor)
* Scott Taylor (@scott2000)
* Shane Sveller (@shanesveller)
* Stephen Jennings (@jennings)
* Tim Janik (@tim-janik)
* Vamsi Avula (@avamsi)
* Waleed Khan (@arxanas)
* Yuya Nishihara (@yuja)

## [0.24.0] - 2024-12-04

### Release highlights

* New [`jj absorb`](https://jj-vcs.github.io/jj/latest/cli-reference/#jj-absorb) command automatically squashes changes from the current commit into relevant ancestor commits.

* Experimental dynamic shell completions have been added; see [the docs](https://jj-vcs.github.io/jj/latest/install-and-setup/#command-line-completion) for configuration.

* [`jj duplicate`](https://jj-vcs.github.io/jj/latest/cli-reference/#jj-duplicate) now accepts `--destination`/`--insert-before`/`--insert-after`.

* Some deprecated commands have been removed (`jj move`, `jj checkout`, `jj merge`).

### Breaking changes

* `jj move` has been removed. It was deprecated in 0.16.0.

* `jj checkout` and the built-in alias `jj co` have been removed.
  It was deprecated in 0.14.0.

* `jj merge` has been removed. It was deprecated in 0.14.0.

* `jj git push` no longer pushes new bookmarks by default. Use `--allow-new` to
  bypass this restriction.

* Lines prefixed with "JJ:" in commit descriptions and in sparse patterns (from
  `jj sparse edit`) are now stripped even if they are not immediately followed
  by a space. [#5004](https://github.com/jj-vcs/jj/issues/5004)

### Deprecations

### New features

* Templates now support the `==` and `!=` logical operators for `Boolean`,
  `Integer`, and `String` types.

* New command `jj absorb` that moves changes to stack of mutable revisions.

* New command `jj util exec` that can be used for arbitrary aliases.

* `jj rebase -b` can now be used with the `--insert-after` and `--insert-before`
  options, like `jj rebase -r` and `jj rebase -s`.

* A preview of improved shell completions was added. Please refer to the
  [documentation](https://jj-vcs.github.io/jj/latest/install-and-setup/#command-line-completion)
  to activate them. They additionally complete context-dependent, dynamic values
  like bookmarks, aliases, revisions, operations and files.

* Added the config setting `snapshot.auto-update-stale` for automatically
  running `jj workspace update-stale` when applicable.

* `jj duplicate` now accepts `--destination`, `--insert-after` and
  `--insert-before` options to customize the location of the duplicated
  revisions.

* `jj log` now displays the working-copy branch first.

* New `fork_point()` revset function can be used to obtain the fork point
  of multiple commits.

* The `tags()` revset function now takes an optional `pattern` argument,
  mirroring that of `bookmarks()`.

* Several commands now support `-f/-t` shorthands for `--from/--to`:
  - `diff`
  - `diffedit`
  - `interdiff`
  - `op diff`
  - `restore`

* New `ui.conflict-marker-style` config option to change how conflicts are
  materialized in the working copy. The default option ("diff") renders
  conflicts as a snapshot with a list of diffs to apply to the snapshot.
  The new "snapshot" option renders conflicts as a series of snapshots, showing
  each side and base of the conflict. The new "git" option replicates Git's
  "diff3" conflict style, meaning it is more likely to work with external tools,
  but it doesn't support conflicts with more than 2 sides.

* New `merge-tools.<TOOL>.conflict-marker-style` config option to override the
  conflict marker style used for a specific merge tool.

* New `merge-tools.<TOOL>.merge-conflict-exit-codes` config option to allow a
  merge tool to exit with a non-zero code to indicate that not all conflicts
  were resolved.

* `jj simplify-parents` now supports configuring the default revset when no
   `--source` or `--revisions` arguments are provided with the
   `revsets.simplify-parents` config.

### Fixed bugs

* `jj config unset <TABLE-NAME>` no longer removes a table (such as `[ui]`.)


### Contributors

Thanks to the people who made this release happen!

* Austin Seipp (@thoughtpolice)
* Benjamin Tan (@bnjmnt4n)
* Daniel Ploch (@torquestomp)
* Emily (@neongreen)
* Essien Ita Essien (@essiene)
* Herman J. Radtke III (@hjr3)
* Ilya Grigoriev (@ilyagr)
* Joaquín Triñanes (@JoaquinTrinanes)
* Lars Francke (@lfrancke)
* Luke Randall (@lukerandall)
* Martin von Zweigbergk (@martinvonz)
* Nathanael Huffman (@nathanaelhuffman)
* Philip Metzger (@PhilipMetzger)
* Remo Senekowitsch (@senekor)
* Robin Stocker (@robinst)
* Scott Taylor (@scott2000)
* Shane Sveller (@shanesveller)
* Tim Janik (@tim-janik)
* Yuya Nishihara (@yuja)

## [0.23.0] - 2024-11-06

### Security fixes

* Fixed path traversal by cloning/checking out crafted Git repository containing
  `..`, `.jj`, `.git` paths.
  ([GHSA-88h5-6w7m-5w56](https://github.com/jj-vcs/jj/security/advisories/GHSA-88h5-6w7m-5w56);CVE-2024-51990)

### Breaking changes

* Revset function names can no longer start with a number.

* Evaluation error of `revsets.short-prefixes` configuration is now reported.

* The `HEAD@git` symbol no longer resolves to the Git HEAD revision. Use
  `git_head()` or `@-` revset expression instead. The `git_head` template
  keyword now returns a boolean.

* Help command doesn't work recursively anymore, i.e. `jj workspace help root`
  doesn't work anymore.

* The color label `op_log` from the `[colors]` config section now **only**
  applies to the op log and not to the other places operations are displayed. In
  almost all cases, if you configured `op_log` before, you should use the new
  `operation` label instead.

* Default operation log template now shows end times of operations instead of
  start times.

### Deprecations

* `git.auto-local-bookmark` replaces `git.auto-local-branch`. The latter remains
  supported for now (at lower precedence than the former).

### New features

* Added diff options to ignore whitespace when comparing lines. Whitespace
  changes are still highlighted.

* New command `jj simplify-parents` will remove redundant parent edges.

* `jj squash` now supports `-f/-t` shorthands for `--from/--[in]to`.

* Initial support for shallow Git repositories has been implemented. However,
  deepening the history of a shallow repository is not yet supported.

* `jj git clone` now accepts a `--depth <DEPTH>` option, which
  allows to clone the repository with a given depth.

* New command `jj file annotate` that annotates files line by line. This is similar
  in functionality to `git blame`. Invoke the command with `jj file annotate <file_path>`.
  The output can be customized via the `templates.annotate_commit_summary`
  config variable.

* `jj bookmark list` gained a `--remote REMOTE` option to display bookmarks
   belonging to a remote. This option can be combined with `--tracked` or
   `--conflicted`.

* New command `jj config unset` that unsets config values. For example,
  `jj config unset --user user.name`.

* `jj help` now has the flag `--keyword` (shorthand `-k`), which can give help
  for some keywords (e.g. `jj help -k revsets`). To see a list of the available
  keywords you can do `jj help --help`.

* New `at_operation(op, expr)` revset can be used in order to query revisions
  based on historical state.

* String literals in filesets, revsets and templates now support hex bytes
  (with `\e` as escape / shorthand for `\x1b`).

* New `coalesce(revsets...)` revset which returns commits in the first revset
  in the `revsets` list that does not evaluate to `none()`.

* New template function `raw_escape_sequence(...)` preserves escape sequences.

* Timestamp objects in templates now have `after(date) -> Boolean` and
  `before(date) -> Boolean` methods for comparing timestamps to other dates.

* New template functions `pad_start()`, `pad_end()`, `truncate_start()`, and
  `truncate_end()` are added.

* Add a new template alias `builtin_log_compact_full_description()`.

* Added the config settings `diff.color-words.context` and `diff.git.context` to
  control the default number of lines of context shown.

### Fixed bugs

* Error on `trunk()` revset resolution is now handled gracefully.
  [#4616](https://github.com/jj-vcs/jj/issues/4616)

* Updated the built-in diff editor `scm-record` to version
  [0.4.0](https://github.com/arxanas/scm-record/releases/tag/v0.4.0), which
  includes multiple fixes.

### Contributors

Thanks to the people who made this release happen!

* Alec Snyder (@allonsy)
* Arthur Grillo (Grillo-0)
* Austin Seipp (@thoughtpolice)
* Benjamin Tan (@bnjmnt4n)
* Dave Townsend (@Mossop)
* Daniel Ploch (@torquestomp)
* Emily (@neongreen)
* Essien Ita Essien (@essiene)
* Fedor Sheremetyev (@sheremetyev)
* Ilya Grigoriev (@ilyagr)
* Jakub Okoński (@farnoy)
* Jcparkyn (@Jcparkyn)
* Joaquín Triñanes (@JoaquinTrinanes)
* Lukas Wirth (@Veykril)
* Marco Neumann (@crepererum)
* Martin von Zweigbergk (@martinvonz)
* Matt Stark (@matts1)
* Philip Metzger (@PhilipMetzger)
* Philipp Albrecht (@pylbrecht)
* Remo Senekowitsch (@senekor)
* Richard Macklin (@rmacklin)
* Robin Stocker (@robinst)
* Samuel Tardieu (@samueltardieu)
* Sora (@SoraTenshi)
* Stephen Jennings (@jennings)
* Theodore Ehrenborg (@TheodoreEhrenborg)
* Vamsi Avula (@avamsi)
* Vincent Ging Ho Yim (@cenviity)
* Yuya Nishihara (@yuja)

## [0.22.0] - 2024-10-02

### Breaking changes

* Fixing [#4239](https://github.com/jj-vcs/jj/issues/4239) means the
  ordering of some messages have changed.

* Invalid `ui.graph.style` configuration is now an error.

* The builtin template `branch_list` has been renamed to `bookmark_list` as part
  of the `jj branch` deprecation.

### Deprecations

* `jj branch` has been deprecated in favor of `jj bookmark`.

  **Rationale:** Jujutsu's branches don't behave like Git branches, which a
  confused many newcomers, as they expected a similar behavior given the name.
  We've renamed them to "bookmarks" to match the actual behavior, as we think
  that describes them better, and they also behave similar to Mercurial's
  bookmarks.

* `jj obslog` is now called `jj evolution-log`/`jj evolog`. `jj obslog` remains
  as an alias.

* `jj unsquash` has been deprecated in favor of `jj squash` and
  `jj diffedit --restore-descendants`.

  **Rationale:** `jj squash` can be used in interactive mode to pull
  changes from one commit to another, including from a parent commit
  to a child commit. For fine-grained dependent diffs, such as when
  the parent and the child commits must successively modify the same
  location in a file, `jj diffedit --restore-descendants` can be used
  to set the parent commit to the desired content without altering the
  content of the child commit.

* The `git.push-branch-prefix` config has been deprecated in favor of
  `git.push-bookmark-prefix`.

* `conflict()` and `file()` revsets have been renamed to `conflicts()` and `files()`
  respectively. The old names are still around and will be removed in a future
  release.

### New features

* The new config option `snapshot.auto-track` lets you automatically track only
  the specified paths (all paths by default). Use the new `jj file track`
  command to manually tracks path that were not automatically tracked. There is
  no way to list untracked files yet. Use `git status` in a colocated workspace
  as a workaround.
  [#323](https://github.com/jj-vcs/jj/issues/323)

* `jj fix` now allows fixing unchanged files with the `--include-unchanged-files` flag. This
  can be used to more easily introduce automatic formatting changes in a new
  commit separate from other changes.

* `jj workspace add` now accepts a `--sparse-patterns=<MODE>` option, which
  allows control of the sparse patterns for a newly created workspace: `copy`
  (inherit from parent; default), `full` (full working copy), or `empty` (the
  empty working copy).

* New command `jj workspace rename` that can rename the current workspace.

* `jj op log` gained an option to include operation diffs.

* `jj git clone` now accepts a `--remote <REMOTE NAME>` option, which
  allows to set a name for the remote instead of using the default
  `origin`.

* `jj op undo` now reports information on the operation that has been undone.

* `jj squash`: the `-k` flag can be used as a shorthand for `--keep-emptied`.

* CommitId / ChangeId template types now support `.normal_hex()`.

* `jj commit` and `jj describe` now accept `--author` option allowing to quickly change
  author of given commit.

* `jj diffedit`, `jj abandon`, and `jj restore` now accept a `--restore-descendants`
  flag. When used, descendants of the edited or deleted commits will keep their original
  content.

* `jj git fetch -b <remote-git-branch-name>` will now warn if the branch(es)
   can not be found in any of the specified/configured remotes.

* `jj split` now lets the user select all changes in interactive mode. This may be used
  to keeping all changes into the first commit while keeping the current commit
  description for the second commit (the newly created empty one).

* Author and committer names are now yellow by default.

### Fixed bugs

* Update working copy before reporting changes. This prevents errors during reporting
  from leaving the working copy in a stale state.

* Fixed panic when parsing invalid conflict markers of a particular form.
  ([#2611](https://github.com/jj-vcs/jj/pull/2611))

* Editing a hidden commit now makes it visible.

* The `present()` revset now suppresses missing working copy error. For example,
  `present(@)` evaluates to `none()` if the current workspace has no
  working-copy commit.

### Contributors

Thanks to the people who made this release happen!

* Austin Seipp (@thoughtpolice)
* Danny Hooper (@hooper)
* Emily Shaffer (@nasamuffin)
* Essien Ita Essien (@essiene)
* Ethan Brierley (@eopb)
* Ilya Grigoriev (@ilyagr)
* Kevin Liao (@kevincliao)
* Lukas Wirth (@Veykril)
* Martin von Zweigbergk (@martinvonz)
* Mateusz Mikuła (@mati865)
* mlcui (@mlcui-corp)
* Philip Metzger (@PhilipMetzger)
* Samuel Tardieu (@samueltardieu)
* Stephen Jennings (@jennings)
* Tyler Goffinet (@qubitz)
* Vamsi Avula (@avamsi)
* Yuya Nishihara (@yuja)

## [0.21.0] - 2024-09-04

### Breaking changes

* `next/prev` will no longer infer when to go into edit mode when moving from
  commit to commit. It now either follows the flags `--edit|--no-edit` or it
  gets the mode from `ui.movement.edit`.

### Deprecations

* `jj untrack` has been renamed to `jj file untrack`.

### New features

* Add new boolean config knob, `ui.movement.edit` for controlling the behavior
  of `prev/next`. The flag turns `edit` mode `on` and `off` permanently when set
  respectively to `true` or `false`.

* All diff formats except `--name-only` now include information about copies and
  moves. So do external diff tools in file-by-file mode. `jj status` also
  includes information about copies and moves.

* Color-words diff has gained [an option to display complex changes as separate
  lines](docs/config.md#color-words-diff-options). It's enabled by default. To
  restore the old behavior, set `diff.color-words.max-inline-alternation = -1`.

* A tilde (`~`) at the start of the path will now be expanded to the user's home
  directory when configuring a `signing.key` for SSH commit signing.

* When reconfiguring the author, warn that the working copy won't be updated

* `jj rebase -s` can now be used with the `--insert-after` and `--insert-before`
  options, like `jj rebase -r`.

### Fixed bugs

* Release binaries for Intel Macs have been restored. They were previously
  broken due to using a sunset version of GitHub's macOS runners (but nobody had
  previously complained.)

### Contributors

Thanks to the people who made this release happen!

* Aaron Bull Schaefer (@elasticdog)
* Austin Seipp (@thoughtpolice)
* Benjamin Tan (@bnjmnt4n)
* Raniz Daniel Raneland (@Raniz85)
* Daniel Ploch (@torquestomp)
* Essien Ita Essien (@essiene)
* Ilya Grigoriev (@ilyagr)
* Kaleb Pace (@kalebpace)
* Marie (@NyCodeGHG)
* Marijan Smetko (@InCogNiTo124)
* Martin von Zweigbergk (@martinvonz)
* Matt Kulukundis (@fowles)
* Scott Taylor (@scott2000)
* Stephen Jennings (@jennings)
* tingerrr (@tingerrr)
* Yuya Nishihara (@yuja)

## [0.20.0] - 2024-08-07

### Note to packagers

* `jj` now links `libgit2` statically by default. To use dynamic linking, you
  need to set the environment variable `LIBGIT2_NO_VENDOR=1` while compiling.
  ([#4163](https://github.com/jj-vcs/jj/pull/4163))

### Breaking changes

* `jj rebase --skip-empty` has been renamed to `jj rebase --skip-emptied`

* `jj backout --revision` has been renamed to `jj backout --revisions`.
  The short alias `-r` is still supported.

* [The default `immutable_heads()` set](docs/config.md#set-of-immutable-commits)
  now includes `untracked_remote_branches()` with the assumption that untracked
  branches aren't managed by you. Therefore, untracked branches are no longer
  displayed in `jj log` by default.

* Updated defaults for graph node symbol templates `templates.log_node` and
  `templates.op_log_node`.

* [The "fileset" language](docs/filesets.md) is now enabled by default. It can
  still be disabled by setting `ui.allow-filesets=false`.

* On `jj git fetch`/`import`, commits referred to by `HEAD@git` are no longer
  preserved. If a checked-out named branch gets deleted locally or remotely, the
  corresponding commits will be abandoned.

* `jj --at-op=@` no longer merges concurrent operations if explicitly specified.

* `jj obslog -p` no longer shows diffs at non-partial squash operations.
  Previously, it showed the same diffs as the second predecessor.

### Deprecations

* The original configuration syntax for `jj fix` is now deprecated in favor of
  one that allows defining multiple tools that can affect different filesets.
  These can be used in combination for now. See `jj help fix` for details.

### New features

* Define `immutable_heads()` revset alias in terms of a new `builtin_immutable_heads()`.
  This enables users to redefine `immutable_heads()` as they wish, but still
  have `builtin_immutable_heads()` which should not be redefined.

* External diff tools can now be configured to invoke the tool on each file
  individually instead of being passed a directory by setting
  `merge-tools.$TOOL.diff-invocation-mode="file-by-file"` in config.toml.

* In git diffs, word-level hunks are now highlighted with underline. See [diff
  colors and styles](docs/config.md#diff-colors-and-styles) for customization.

* New `.diff().<format>()` commit template methods are added. They can be used
  in order to show diffs conditionally. For example,
  `if(current_working_copy, diff.summary())`.

* `jj git clone` and `jj git init` with an existing git repository adds the
  default branch of the remote as repository settings for
  `revset-aliases."trunk()"`.`

* `jj workspace forget` now abandons the workspace's working-copy commit if it
  was empty.

* `jj backout` now includes the backed out commit's subject in the new commit
  message.

* `jj backout` can now back out multiple commits at once.

* `jj git clone some/nested/path` now creates the full directory tree for
   nested destination paths if they don't exist.

* String patterns now support case‐insensitive matching by suffixing any
  pattern kind with `-i`. `mine()` uses case‐insensitive matching on your email
  address unconditionally. Only ASCII case folding is currently implemented,
  but this will likely change in the future.

* String patterns now support `regex:"pattern"`.

* New `tracked_remote_branches()` and `untracked_remote_branches()` revset
  functions can be used to select tracked/untracked remote branches.

* The `file()` revset function now accepts fileset as argument.

* New `diff_contains()` revset function can be used to search diffs.

* New command `jj operation diff` that can compare changes made between two
  operations.

* New command `jj operation show` that can show the changes made in a single
  operation.

* New config setting `git.private-commits` to prevent commits from being pushed.

* [The default commit description template](docs/config.md#default-description)
  can now be configured by `templates.draft_commit_description`.

* `jj fix` can now be configured to run different tools on different filesets.
  This simplifies the use case of configuring code formatters for specific file
  types. See `jj help fix` for details.

* Added revset functions `author_date` and `committer_date`.

* `jj describe` can now update the description of multiple commits.

### Fixed bugs

* `jj status` will show different messages in a conflicted tree, depending
  on the state of the working commit. In particular, if a child commit fixes
  a conflict in the parent, this will be reflected in the hint provided
  by `jj status`

* `jj diff --git` no longer shows the contents of binary files.

* Windows binaries no longer require `vcruntime140.dll` to be installed
  (normally through Visual Studio.)

* On quit, the builtin pager no longer waits for all outputs to be discarded.

* `jj branch rename` no longer shows a warning in colocated repos.

### Contributors

Thanks to the people who made this release happen!

* Anton Älgmyr (@algmyr)
* Austin Seipp (@thoughtpolice)
* Benjamin Tan (@bnjmnt4n)
* Daniel Ploch (@torquestomp)
* Danny Hooper (@hooper)
* Emily (@emilazy)
* Essien Ita Essien (@essiene)
* Fedor Sheremetyev (@sheremetyev)
* Ilya Grigoriev (@ilyagr)
* Jonathan Tan (@jonathantanmy)
* Julien Vincent (@julienvincent)
* Martin von Zweigbergk (@martinvonz)
* Matt Kulukundis (@fowles)
* Matt Stark (@matts1)
* mlcui (@mlcui-corp)
* Philip Metzger (@PhilipMetzger)
* Scott Taylor (@scott2000)
* Skyler Grey (@Minion3665)
* Stephen Jennings (@jennings)
* Tim Janik (@tim-janik)
* Vincent Ging Ho Yim (@cenviity)
* Vladimír Čunát (@vcunat)
* Vladimir (@0xdeafbeef)
* Yuya Nishihara (@yuja)

## [0.19.0] - 2024-07-03

### Breaking changes

* In revset aliases, top-level `kind:pattern` expression is now parsed as
  modifier. Surround with parentheses if it should be parsed as string/file
  pattern.

* Dropped support for automatic upgrade of repo formats used by versions before
  0.12.0.

* `jj fix` now defaults to the broader revset `-s reachable(@, mutable())`
  instead of `-s @`.

* Dropped support for deprecated `jj branch delete`/`forget` `--glob` option.

* `jj branch set` now creates new branch if it doesn't exist. Use `jj branch
  move` to ensure that the target branch already exists.
  [#3584](https://github.com/jj-vcs/jj/issues/3584)

### Deprecations

* Replacing `-l` shorthand for `--limit` with `-n` in `jj log`, `jj op log`
  and `jj obslog`.

* `jj split --siblings` is deprecated in favor of `jj split --parallel` (to
  match `jj parallelize`).

* A new `jj file` subcommand now replaces several existing uncategorized
  commands, which are deprecated.
  - `jj file show` replaces `jj cat`.
  - `jj file chmod` replaces `jj chmod`.
  - `jj file list` replaces `jj files`.

### New features

* Support background filesystem monitoring via watchman triggers enabled with
  the `core.watchman.register_snapshot_trigger = true` config.

* Show paths to config files when configuration errors occur.

* `jj fix` now supports configuring the default revset for `-s` using the
  `revsets.fix` config.

* The `descendants()` revset function now accepts an optional `depth` argument;
  like the `ancestors()` depth argument, it limits the depth of the set.

* Revset/template aliases now support function overloading.
  [#2966](https://github.com/jj-vcs/jj/issues/2966)

* Conflicted files are individually simplified before being materialized.

* The `jj file` subcommand now contains several existing file utilities.
  - `jj file show`, replacing `jj cat`.
  - `jj file chmod` replacing `jj chmod`.
  - `jj file list` replacing `jj files`.

* New command `jj branch move` let you update branches by name pattern or source
  revision.

* New diff option `jj diff --name-only` allows for easier shell scripting.

* In color-words diffs, hunks are now highlighted with underline. See [diff
  colors and styles](docs/config.md#diff-colors-and-styles) for customization.

* `jj git push -c <arg>` can now accept revsets that resolve to multiple
  revisions. This means that `jj git push -c xyz -c abc` is now equivalent to
  `jj git push -c 'all:(xyz | abc)'`.

* `jj prev` and `jj next` have gained a `--conflict` flag which moves you
  to the next conflict in a child commit.

* New command `jj git remote set-url` that sets the url of a git remote.

* Author timestamp is now reset when rewriting discardable commits (empty
  commits with no description) if authored by the current user.
  [#2000](https://github.com/jj-vcs/jj/issues/2000)

* `jj commit` now accepts `--reset-author` option to match `jj describe`.

* `jj squash` now accepts a `--keep-emptied` option to keep the source commit.

### Fixed bugs

* `jj git push` now ignores immutable commits when checking whether a
  to-be-pushed commit has conflicts, or has no description / committer / author
  set. [#3029](https://github.com/jj-vcs/jj/issues/3029)

* `jj` will look for divergent changes outside the short prefix set even if it
  finds the change id inside the short prefix set.
  [#2476](https://github.com/jj-vcs/jj/issues/2476)

### Contributors

Thanks to the people who made this release happen!

* Austin Seipp (@thoughtpolice)
* Benjamin Tan (@bnjmnt4n)
* Daniel Ploch (@torquestomp)
* Danny Hooper (@hooper)
* Ilya Grigoriev (@ilyagr)
* James Sully (@sullyj3)
* Jonathan Tan (@jonathantanmy)
* Kyle J Strand (@BatmanAoD)
* Manuel Caldeira (@KiitoX)
* Martin von Zweigbergk (@martinvonz)
* Matt Kulukundis (@fowles)
* Matt Stark (@matts1)
* mlcui (@mlcui-corp)
* Philip Metzger (@PhilipMetzger)
* Scott Taylor (@scott2000)
* Simon Wollwage (@Kintaro)
* Tal Pressman (@tp-woven)
* Yuya Nishihara (@yuja)

## [0.18.0] - 2024-06-05

### Breaking changes

* Dropped support for `ui.default-revset` config (replaced by `revsets.log` in
  0.8.0).

* The `commit_summary_no_branches` template is superseded by
  `templates.branch_list`.

* `jj split` will now refuse to split an empty commit.

* `jj config list` now uses multi-line strings and single-quoted strings in the
  output when appropriate.

* `jj config get`/`list`/`set` now parse `name` argument as [TOML
  key](https://toml.io/en/v1.0.0#keys). Quote meta characters as needed.
  Example: `jj config get "revset-aliases.'trunk()'"`

* When updating the working copy away from an empty and undescribed commit, it
  is now abandoned even if it is a merge commit.

* If a new working-copy commit is created because the old one was abandoned, and
  the old commit was merge, then the new commit will now also be.
  [#2859](https://github.com/jj-vcs/jj/issues/2859)

* `jj new`'s `--insert-before`/`--insert-after` options must now be set for each
  commit the new commit will be inserted before/after. Previously, those options
  were global flags and specifying them once would insert the new commit before/
  after all the specified commits.

### Deprecations

* Attempting to alias a built-in command now gives a warning, rather than being
  silently ignored.

### New features

* `jj branch list`/`tag list` now accept `-T`/`--template` option. The tag list
  prints commit summary along with the tag name by default.

* Conflict markers now include an explanation of what each part of the conflict
  represents.

* `ui.color = "debug"` prints active labels alongside the regular colored
  output.

* `jj branch track` now show conflicts if there are some.

* A new revset `reachable(srcs, domain)` will return all commits that are
  reachable from `srcs` within `domain`.

* There are now prebuilt binaries for `aarch64-linux-unknown-musl`.
  Note, these are cross compiled and currently untested.
  We plan on providing fully tested builds later once our CI system allows it.

* Added new revsets `mutable()` and `immutable()`.

* Upgraded `scm-record` from v0.2.0 to v0.3.0. See release notes at
  <https://github.com/arxanas/scm-record/releases/tag/v0.3.0>

* New command `jj fix` that can be configured to update commits by running code
  formatters (or similar tools) on changed files. The configuration schema and
  flags are minimal for now, with a number of improvements planned (for example,
  [#3800](https://github.com/jj-vcs/jj/issues/3800) and
  [#3801](https://github.com/jj-vcs/jj/issues/3801)).

* `jj new`'s `--insert-before` and `--insert-after` options can now be used
  simultaneously.

* `jj git push` now can push commits with empty descriptions with the
  `--allow-empty-description` flag

### Fixed bugs

* Previously, `jj git push` only made sure that the branch is in the expected
  location on the remote server when pushing a branch forward (as opposed to
  sideways or backwards). Now, `jj git push` makes a safety check in all cases
  and fails whenever `jj git fetch` would have introduced a conflict.

  In other words, previously branches that moved sideways or backward were
  pushed similarly to Git's `git push --force`; now they have protections
  similar to `git push --force-with-lease` (though not identical to it, to match
  the behavior of `jj git fetch`). Note also that because of the way `jj git
  fetch` works, `jj` does not suffer from the same problems as Git's `git push
  --force-with-lease` in situations when `git fetch` is run in the background.

* When the working copy commit becomes immutable, a new one is automatically
  created
  on top of it to avoid letting the user edit the immutable one.

* `jj config list` now properly escapes TOML keys (#1322).

* Files with conflicts are now checked out as executable if all sides of the
  conflict are executable.

* The progress bar (visible when using e.g. `jj git clone`) clears the
  remainder of the cursor row after drawing rather than clearing the entire row
  before drawing, eliminating the "flicker" effect seen on some terminals.

### Contributors

Thanks to the people who made this release happen!

* Alexander Potashev (@aspotashev)
* Austin Seipp (@thoughtpolice)
* Benjamin Tan (@bnjmnt4n)
* Charles Crete (@Cretezy)
* Daniel Ploch (@torquestomp)
* Danny Hooper (@hooper)
* Eidolon (@HybridEidolon)
* Glen Choo (@chooglen)
* Gregory Anders (@gpanders)
* Ilya Grigoriev (@ilyagr)
* jyn (@jyn514)
* Martin von Zweigbergk (@martinvonz)
* Matt Stark (@matts1)
* Matthew Davidson (@KingMob)
* Michael Gattozzi (@mgattozzi)
* mlcui (@mlcui-corp)
* Philip Metzger (@PhilipMetzger)
* Remo Senekowitsch (@senekor)
* Thomas Castiglione (@gulbanana)
* Théo Daron (@tdaron)
* tinger (@tingerrr)
* Waleed Khan (@arxanas)
* Yuya Nishihara (@yuja)

## [0.17.1] - 2024-05-07

### Fixed bugs

* `jj status` no longer scans through the entire history to look for ancestors
  with conflicts.

## [0.17.0] - 2024-05-01

### Breaking changes

* The default template aliases were replaced as follows:
  * `builtin_op_log_root(op_id: OperationId)` ->
    `format_root_operation(root: Operation)`
  * `builtin_log_root(change_id: ChangeId, commit_id: CommitId)` ->
    `format_root_commit(root: Commit)`
  * `builtin_change_id_with_hidden_and_divergent_info` ->
    `format_short_change_id_with_hidden_and_divergent_info(commit: Commit)`

* The `--revision` option of `jj rebase` is renamed to `--revisions`. The short
  alias `-r` is still supported.

### New features

* The list of conflicted paths is printed whenever the working copy changes.
  This can be disabled with the `--quiet` option.

* Commit objects in templates now have a `mine() -> Boolean` method analog to
  the same function in revsets. It evaluates to true if the email of the commit
  author matches the current `user.email`.

* Commit objects in templates now have a `contained_in(revset: String) ->
  Boolean` method.

* Operation objects in templates now have a `snapshot() -> Boolean` method that
  evaluates to true if the operation was a snapshot created by a non-mutating
  command (e.g. `jj log`).

* Revsets and templates now support single-quoted raw string literals.

* A new config option `ui.always-allow-large-revsets` has been added to
  allow large revsets expressions in some commands, without the `all:` prefix.

* A new config option `ui.allow-filesets` has been added to enable ["fileset"
  expressions](docs/filesets.md). Note that filesets are currently experimental,
  but will be enabled by default in a future release.

* A new global flag `--ignore-immutable` lets you rewrite immutable commits.

* New command `jj parallelize` that rebases a set of revisions into siblings.

* `jj status` now supports filtering by paths. For example, `jj status .` will
  only list changed files that are descendants of the current directory.

* `jj prev` and `jj next` now work when the working copy revision is a merge.

* `jj squash` now accepts a `--use-destination-message/-u` option that uses the
  description of the destination for the new squashed revision and discards the
  descriptions of the source revisions.

* You can check whether Watchman fsmonitor is enabled or installed with the new
  `jj debug watchman status` command.

* `jj rebase` now accepts revsets resolving to multiple revisions with the
  `--revisions`/`-r` option.

* `jj rebase -r` now accepts `--insert-after` and `--insert-before` options to
  customize the location of the rebased revisions.

### Fixed bugs

* Revsets now support `\`-escapes in string literal.

* The builtin diff editor now allows empty files to be selected during
  `jj split`.

* Fixed a bug with `jj split` introduced in 0.16.0 that caused it to incorrectly
  rebase the children of the revision being split if they had other parents
  (i.e. if the child was a merge).

* The `snapshot.max-new-file-size` option can now handle raw integer literals,
  interpreted as a number of bytes, where previously it could only handle string
  literals. This means that `snapshot.max-new-file-size="1"` and
  `snapshot.max-new-file-size=1` are now equivalent.

* `jj squash <path>` is now a no-op if the path argument didn't match any paths
  (it used to create new commits with bumped timestamp).
  [#3334](https://github.com/jj-vcs/jj/issues/3334)

### Contributors

Thanks to the people who made this release happen!

* Anton Älgmyr (@algmyr)
* Anton Bulakh (@necauqua)
* Austin Seipp (@thoughtpolice)
* Benjamin Tan (@bnjmnt4n)
* Cretezy (@Cretezy)
* Daniel Ploch (@torquestomp)
* Evan Mesterhazy (@emesterhazy)
* Ilya Grigoriev (@ilyagr)
* Martin von Zweigbergk (@martinvonz)
* Noah Mayr (@noahmayr)
* Jeremy O'Brien (@neutralinsomniac)
* Jonathan Lorimer (@JonathanLorimer)
* Philip Metzger (@PhilipMetzger)
* Poliorcetics (@poliorcetics)
* Rowan Walsh (@rowan-walsh)
* Scott Olson (@solson)
* Théo Daron (@tdaron)
* Yuya Nishihara (@yuja)

## [0.16.0] - 2024-04-03

### Deprecations

* `jj move` was deprecated in favor of `jj squash`.

### Breaking changes

* The `git_head` template keyword now returns an optional value instead of a
  list of 0 or 1 element.

* The `jj sparse set --edit`/`--reset` flags were split up into `jj sparse
  edit`/`reset` subcommands respectively.

* The `jj sparse` subcommands now parse and print patterns as workspace-relative
  paths.

* The `jj log` command no longer uses the default revset when a path is
  specified.

### New features

* Config now supports rgb hex colors (in the form `#rrggbb`) wherever existing
  color names are supported.

* `ui.default-command` now accepts multiple string arguments, for more complex
  default `jj` commands.

* Graph node symbols are now configurable via templates
  * `templates.log_node`
  * `templates.op_log_node`

* `jj log` now includes synthetic nodes in the graph where some revisions were
  elided.

* `jj squash` now accepts `--from` and `--into` (also aliased as `--to`) if `-r`
  is not specified. It can now be used for all use cases where `jj move` could
  previously be used. The `--from` argument accepts a revset that resolves to
  more than one revision.

* Commit templates now support `immutable` keyword.

* New template function `coalesce(content, ..)` is added.

* Timestamps are now shown in local timezone and without milliseconds and
  timezone offset by default.

* `jj git push` now prints messages from the remote.

* `jj branch list` now supports a `--conflicted/-c` option to show only
  conflicted branches.

* `jj duplicate` and `jj abandon` can now take more than a single `-r` argument,
  for consistency with other commands.

* `jj branch list` now allows combining `-r REVISIONS`/`NAMES` and `-a` options.

* `--all` is now named `--all-remotes` for `jj branch list`

* There is a new global `--quiet` flag to silence commands' non-primary output.

* `jj split` now supports a `--siblings/-s` option that splits the target
  revision into siblings with the same parents and children.

* New function `working_copies()` for revsets to show the working copy commits
  of all workspaces.

### Fixed bugs

None.

### Contributors

Thanks to the people who made this release happen!

* Aleksey Kuznetsov (@zummenix)
* Anton Älgmyr (@algmyr)
* Austin Seipp (@thoughtpolice)
* Benjamin Tan (@bnjmnt4n)
* Chris Krycho (@chriskrycho)
* Christoph Koehler (@ckoehler)
* Daniel Ploch (@torquestomp)
* Evan Mesterhazy (@emesterhazy)
* Ilya Grigoriev (@ilyagr)
* Khionu Sybiern (@khionu)
* Martin von Zweigbergk (@martinvonz)
* Matthew Davidson (@KingMob)
* mrstanwell (@mrstanwell)
* Noah Mayr (@noahmayr)
* Patric Stout (@TrueBrain)
* Poliorcetics (@poliorcetics)
* Simon Wollwage (@Kintaro)
* Steve Klabnik (@steveklabnik)
* Tom Ward (@tomafro)
* TrashCan (@TrashCan69420)
* Yuya Nishihara (@yuja)

## [0.15.1] - 2024-03-06

No code changes (fixing Rust `Cargo.toml` stuff).

## [0.15.0] - 2024-03-06

### Breaking changes

* The minimum supported Rust version (MSRV) is now 1.76.0.

* The on-disk index format changed. New index files will be created
  automatically, but it can fail if the repository is colocated and predates
  Git GC issues [#815](https://github.com/jj-vcs/jj/issues/815). If
  reindexing failed, you'll need to clean up corrupted operation history by
  `jj op abandon ..<bad operation ID>`.

* Dropped support for the "legacy" graph-drawing style. Use "ascii" for a very
  similar result.

* The default log output no longer lists all tagged heads. Set `revsets.log =
  "@ | ancestors(immutable_heads().., 2) | heads(immutable_heads())"` to restore
  the old behavior.

* Dropped support for the deprecated `:` revset operator. Use `::` instead.

* `jj rebase --skip-empty` no longer abandons commits that were already empty
  before the rebase.

### New features

* Partial support for commit signing. Currently you can configure jj to "keep"
  commit signatures by making new ones for rewritten commits, and to sign new
  commits when they are created.

  This comes with out-of-the-box support for the following backends:
  * GnuPG
  * SSH

  Signature verification and an explicit sign command will hopefully come soon.

* Templates now support logical operators: `||`, `&&`, `!`

* Templates now support the `self` keyword, which is the current commit in `jj
  log`/`obslog` templates.

* `jj show` now accepts `-T`/`--template` option to render its output using
  template

* `jj config list` now accepts `-T`/`--template` option.

* `jj git fetch` now accepts `-b` as a shorthand for `--branch`, making it more
  consistent with other commands that accept a branch

* In the templating language, Timestamps now have a `.local()` method for
  converting to the local timezone.

* `jj next/prev` now infer `--edit` when you're already editing a non-head
  commit (a commit with children).

* A new built-in pager named `:builtin` is available on all platforms,
  implemented with [minus](https://github.com/arijit79/minus/)

* Set config `ui.log-synthetic-elided-nodes = true` to make `jj log` include
  synthetic nodes in the graph where some revisions were elided
  ([#1252](https://github.com/jj-vcs/jj/issues/1252),
  [#2971](https://github.com/jj-vcs/jj/issues/2971)). This may become the
  default depending on feedback.

* When creating a new workspace, the sparse patterns are now copied over from
  the current workspace.

* `jj git init --colocate` can now import an existing Git repository. This is
  equivalent to `jj git init --git-repo=.`.

* `jj git fetch` now automatically prints new remote branches and tags by
  default.

* `--verbose/-v` is now `--debug` (no short option since it's not intended to be
  used often)

* `jj move --from/--to` can now be abbreviated to `jj move -f/-t`

* `jj commit`/`diffedit`/`move`/`resolve`/`split`/`squash`/`unsquash` now accept
  `--tool=<NAME>` option to override the default.
  [#2575](https://github.com/jj-vcs/jj/issues/2575)

* Added completions for [Nushell](https://nushell.sh) to `jj util completion`

* `jj branch list` now supports a `--tracked/-t` option which can be used to
  show tracked branches only. Omits local Git-tracking branches by default.

* Commands producing diffs now accept a `--context` flag for the number of
  lines of context to show.

* `jj` commands with the `-T`/`--template` option now provide a hint containing
  defined template names when no argument is given, assisting the user in making
  a selection.

### Fixed bugs

* On Windows, symlinks in the repo are now supported when Developer Mode is
  enabled.
  When symlink support is unavailable, they will be materialized as regular
  files in the
  working copy (instead of resulting in a crash).
  [#2](https://github.com/jj-vcs/jj/issues/2)

* On Windows, the `:builtin` pager is now used by default, rather than being
  disabled entirely.

* Auto-rebase now preserves the shape of history even for merge commits where
  one parent is an ancestor of another.
  [#2600](https://github.com/jj-vcs/jj/issues/2600)

### Contributors

Thanks to the people who made this release happen!

* Aleksey Kuznetsov (@zummenix)
* Anton Bulakh (@necauqua)
* Anton Älgmyr (@algmyr)
* Austin Seipp (@thoughtpolice)
* Benjamin Brittain (@benbrittain)
* Benjamin Tan (@bnjmnt4n)
* Daehyeok Mun (@daehyeok)
* Daniel Ploch (@torquestomp)
* Evan Mesterhazy (@emesterhazy)
* gulbanana (@gulbanana)
* Ilya Grigoriev (@ilyagr)
* Jonathan Tan (@jonathantanmy)
* Julien Vincent (@julienvincent)
* jyn (@jyn514)
* Martin von Zweigbergk (@martinvonz)
* Paulo Coelho (@prscoelho)
* Philip Metzger (@PhilipMetzger)
* Poliorcetics (@poliorcetics)
* Stephen Jennings (@jennings)
* Vladimir (@0xdeafbeef)
* Yuya Nishihara (@yuja)

## [0.14.0] - 2024-02-07

### Deprecations

* `jj checkout` and `jj merge` are both deprecated; use `jj new` instead to
  replace both of these commands in all instances.

  **Rationale**: `jj checkout` and `jj merge` both implement identical
  functionality, which is a subset of `jj new`. `checkout` creates a new working
  copy commit on top of a single specified revision, i.e. with one parent.
  `merge` creates a new working copy commit on top of *at least* two specified
  revisions, i.e. with two or more parents.

  The only difference between these commands and `jj new`, which *also* creates
  a new working copy commit, is that `new` can create a working copy commit on
  top of any arbitrary number of revisions, so it can handle both the previous
  cases at once. The only actual difference between these three commands is the
  command syntax and their name. These names were chosen to be familiar to users
  of other version control systems, but we instead encourage all users to adopt
  `jj new` instead; it is more general and easier to remember than both of
  these.

  `jj checkout` and `jj merge` will no longer be shown as part of `jj help`, but
  will still function for now, emitting a warning about their deprecation.

  **Deadline**: `jj checkout` and `jj merge` will be deleted and are expected
  become a **hard error later in 2024**.

* `jj init --git` and `jj init --git-repo` are now deprecated and will be
  removed
  in the near future.

  Use `jj git init` instead.

### Breaking changes

* (Minor) Diff summaries (e.g. `jj diff -s`) now use `D` for "Deleted" instead
  of `R` for "Removed". @joyously pointed out that `R` could also mean
  "Renamed".

* `jj util completion` now takes the shell as a positional argument, not a flag.
  the previous behavior is deprecated, but supported for now. it will be removed
  in the future.

* `jj rebase` now preserves the shape of history even for merge commits where
  one parent is an ancestor of another. You can follow the `jj rebase` by
  `jj rebase -s <merge commit> -d <single parent>` if you want to linearize the
  history.

### New features

* `jj util completion` now supports powershell and elvish.

* Official binaries for macOS running on Apple Silicon (`aarch64-apple-darwin`)
  are now available, alongside the existing macOS x86 binaries.

* New `jj op abandon` command is added to clean up the operation history. Git
  refs and commit objects can be further compacted by `jj util gc`.

* `jj util gc` now removes unreachable operation, view, and Git objects.

* `jj branch rename` will now warn if the renamed branch has a remote branch,
  since
  those will have to be manually renamed outside of `jj`.

* `jj git push` gained a `--tracked` option, to push all the tracked branches.

* There's now a virtual root operation, similar to the [virtual root
  commit](docs/glossary.md#root-commit). It appears at the end of `jj op log`.

* `jj config list` gained a `--include-overridden` option to allow
  printing overridden config values.

* `jj config list` now accepts `--user` or `--repo` option to specify
  config origin.

* New `jj config path` command to print the config file path without launching
  an editor.

* `jj tag list` command prints imported git tags.

* `jj next` and `jj prev` now prompt in the event of the next/previous commit
  being ambiguous, instead of failing outright.

* `jj resolve` now displays the file being resolved.

* `jj workspace root` was aliased to `jj root`, for ease of discoverability

* `jj diff` no longer shows the contents of binary files.

* `jj git` now has an `init` command that initializes a git backed repo.

* New template function `surround(prefix, suffix, content)` is added.

### Fixed bugs

* Fixed snapshots of symlinks in `gitignore`-d directory.
  [#2878](https://github.com/jj-vcs/jj/issues/2878)

* Fixed data loss in dirty working copy when checked-out branch is rebased or
  abandoned by Git.
  [#2876](https://github.com/jj-vcs/jj/issues/2876)

### Contributors

Thanks to the people who made this release happen!

* Austin Seipp (@thoughtpolice)
* Benjamin Brittain (@benbrittain)
* Chris Krycho (@chriskrycho)
* Daehyeok Mun (@daehyeok)
* Daniel Ploch (@torquestomp)
* Essien Ita Essien (@essiene)
* Ikko Eltociear Ashimine (@eltociear)
* Ilya Grigoriev (@ilyagr)
* Jonathan Tan (@jonathantanmy)
* jyn (@jyn514)
* Martin von Zweigbergk (@martinvonz)
* Matt Stark (@matts1)
* Michael Pratt (prattmic)
* Philip Metzger (@PhilipMetzger)
* Stephen Jennings (@jennings)
* Valentin Gatien-Baron (@v-gb)
* vwkd (@vwkd)
* Yuya Nishihara (@yuja)

## [0.13.0] - 2024-01-03

### Breaking changes

* `jj git fetch` no longer imports new remote branches as local branches. Set
  `git.auto-local-branch = true` to restore the old behavior.

### New features

* Information about new and resolved conflicts is now printed by every command.

* `jj branch` has gained a new `rename` subcommand that allows changing a branch
  name atomically. `jj branch help rename` for details.

### Fixed bugs

* Command aliases can now be loaded from repository config relative to the
  current working directory.
  [#2414](https://github.com/jj-vcs/jj/issues/2414)

### Contributors

Thanks to the people who made this release happen!

* Austin Seipp (@thoughtpolice)
* Essien Ita Essien (@essiene)
* Gabriel Scherer (@gasche)
* Ilya Grigoriev (@ilyagr)
* Martin von Zweigbergk (@martinvonz)
* Philip Metzger (@PhilipMetzger)
* Waleed Khan (@arxanas)
* Yuya Nishihara (@yuja)

## [0.12.0] - 2023-12-05

### Breaking changes

* The `remote_branches()` revset no longer includes branches exported to the Git
  repository (so called Git-tracking branches.)

* `jj branch set` no longer creates a new branch. Use `jj branch create`
  instead.

* `jj init --git` in an existing Git repository now errors and exits rather than
  creating a second Git store.

### New features

* `jj workspace add` can now take _multiple_ `--revision` arguments, which will
  create a new workspace with its working-copy commit on top of all the parents,
  as if you had run `jj new r1 r2 r3 ...`.

* You can now set `git.abandon-unreachable-commits = false` to disable the
  usual behavior where commits that became unreachable in the Git repo are
  abandoned ([#2504](https://github.com/jj-vcs/jj/pull/2504)).

* `jj new` gained a `--no-edit` option to prevent editing the newly created
  commit. For example, `jj new a b --no-edit -m Merge` creates a merge commit
  without affecting the working copy.

* `jj rebase` now takes the flag `--skip-empty`, which doesn't copy over commits
  that would become empty after a rebase.

* There is a new `jj util gc` command for cleaning up the repository storage.
  For now, it simply runs `git gc` on the backing Git repo (when using the Git
  backend).

### Fixed bugs

* Fixed another file conflict resolution issue where `jj status` would disagree
  with the actual file content.
  [#2654](https://github.com/jj-vcs/jj/issues/2654)

### Contributors

Thanks to the people who made this release happen!

* Antoine Cezar (@AntoineCezar)
* Anton Bulakh (@necauqua)
* Austin Seipp (@thoughtpolice)
* Benjamin Saunders (@Ralith)
* Carlos Precioso (@cprecioso)
* Chris Krycho (@chriskrycho)
* Ilya Grigoriev (@ilyagr)
* Jason R. Coombs (@jaraco)
* Jesse Somerville (@jessesomerville)
* Łukasz Kurowski (@crackcomm)
* Martin von Zweigbergk (@martinvonz)
* mlcui (@mlcui-corp)
* Philip Metzger (@PhilipMetzger)
* Waleed Khan (@arxanas)
* Yuya Nishihara (@yuja)

## [0.11.0] - 2023-11-01

### Breaking changes

* Conflicts are now stored in a different way. Commits written by a new `jj`
  binary will not be read correctly by older `jj` binaries. The new model
  solves some performance problems with the old model. For example, `jj log`
  should be noticeably faster on large repos. You may need to create a new
  clone to see the full speedup.

* The `remote_branches()` revset now includes branches exported to the Git
  repository (so called Git-tracking branches.) *This change will be reverted
  in 0.12.0.*

* Status messages are now printed to stderr.

* `jj config set` now interprets the value as TOML also if it's a valid TOML
  array or table. For example, `jj config set --user 'aliases.n' '["new"]'`

* Remote branches now have tracking or non-tracking flags. The
  `git.auto-local-branch` setting is applied only to newly fetched remote
  branches. Existing remote branches are migrated as follows:

  * If local branch exists, the corresponding remote branches are considered
    tracking branches.
  * Otherwise, the remote branches are non-tracking branches.

  If the deduced tracking flags are wrong, use `jj branch track`/`untrack`
  commands to fix them up.

  See [automatic local branch creation](docs/config.md#automatic-local-bookmark-creation)
  for details.

* Non-tracking remote branches aren't listed by default. Use `jj branch list
  --all` to show all local and remote branches.

* It's not allowed to push branches if non-tracking remote branches of the same
  name exist.

* Pushing deleted/moved branches no longer abandons the local commits referenced
  by the remote branches.

* `jj git fetch --branch` now requires `glob:` prefix to expand `*` in branch
  name.

### New features

* `jj`'s stable release can now be installed
  with [`cargo binstall jj-cli`](https://github.com/cargo-bins/cargo-binstall).

* `jj workspace add` now takes a `--revision` argument.

* `jj workspace forget` can now forget multiple workspaces at once.

* `branches()`/`remote_branches()`/`author()`/`committer()`/`description()`
  revsets now support glob matching.

* `jj branch delete`/`forget`/`list`, and `jj git push --branch` now support
  [string pattern syntax](docs/revsets.md#string-patterns). The `--glob` option
  is deprecated in favor of `glob:` pattern.

* The `branches`/`tags`/`git_refs`/`git_head` template keywords now return a
  list of `RefName`s. They were previously pre-formatted strings.

* The new template keywords `local_branches`/`remote_branches` are added to show
  only local/remote branches.

* `jj workspace add` now preserves all parents of the old working-copy commit
  instead of just the first one.

* `jj rebase -r` gained the ability to rebase a revision `A` onto a descendant
  of `A`.

### Fixed bugs

* Updating the working copy to a commit where a file that's currently ignored
  in the working copy no longer leads to a crash
  ([#976](https://github.com/jj-vcs/jj/issues/976)).

* Conflicts in executable files can now be resolved just like conflicts in
  non-executable files ([#1279](https://github.com/jj-vcs/jj/issues/1279)).

* `jj new --insert-before` and `--insert-after` now respect immutable revisions
  ([#2468](https://github.com/jj-vcs/jj/pull/2468)).

### Contributors

Thanks to the people who made this release happen!

* Antoine Cezar (@AntoineCezar)
* Austin Seipp (@thoughtpolice)
* Benjamin Saunders (@Ralith)
* Gabriel Scherer (@gasche)
* Ilya Grigoriev (@ilyagr)
* Infra (@1011X)
* Isabella Basso (@isinyaaa)
* Martin von Zweigbergk (@martinvonz)
* Tal Pressman (@talpr)
* Waleed Khan (@arxanas)
* Yuya Nishihara (@yuja)

## [0.10.0] - 2023-10-04

### Breaking changes

* A default revset-alias function `trunk()` now exists. If you previously
  defined
  your own `trunk()` alias it will continue to overwrite the built-in one.
  Check [revsets.toml](docs/revsets.toml)
  and [revsets.md](docs/revsets.md)
  to understand how the function can be adapted.

### New features

* The `ancestors()` revset function now takes an optional `depth` argument
  to limit the depth of the ancestor set. For example, use `jj log -r
  'ancestors(@, 5)` to view the last 5 commits.

* Support for the Watchman filesystem monitor is now bundled by default. Set
  `core.fsmonitor = "watchman"` in your repo to enable.

* You can now configure the set of immutable commits via
  `revset-aliases.immutable_heads()`. For example, set it to
  `"remote_branches() | tags()"` to prevent rewriting those those. Their
  ancestors are implicitly also immutable.

* `jj op log` now supports `--no-graph`.

* Templates now support an additional escape: `\0`. This will output a literal
  null byte. This may be useful for e.g.
  `jj log -T 'description ++ "\0"' --no-graph` to output descriptions only, but
  be able to tell where the boundaries are

* jj now bundles a TUI tool to use as the default diff and merge editors. (The
  previous default was `meld`.)

* `jj split` supports the `--interactive` flag. (This is already the default if
  no paths are provided.)

* `jj commit` accepts an optional list of paths indicating a subset of files to
  include in the first commit

* `jj commit` accepts the `--interactive` flag.

### Fixed bugs

### Contributors

Thanks to the people who made this release happen!

* Austin Seipp (@thoughtpolice)
* Emily Kyle Fox (@emilykfox)
* glencbz (@glencbz)
* Hong Shin (@honglooker)
* Ilya Grigoriev (@ilyagr)
* James Sully (@sullyj3)
* Martin von Zweigbergk (@martinvonz)
* Philip Metzger (@PhilipMetzger)
* Ruben Slabbert (@rslabbert)
* Vamsi Avula (@avamsi)
* Waleed Khan (@arxanas)
* Willian Mori (@wmrmrx))
* Yuya Nishihara (@yuja)
* Zachary Dremann (@Dr-Emann)

## [0.9.0] - 2023-09-06

### Breaking changes

* The minimum supported Rust version (MSRV) is now 1.71.0.

* The storage format of branches, tags, and git refs has changed. Newly-stored
  repository data will no longer be loadable by older binaries.

* The `:` revset operator is deprecated. Use `::` instead. We plan to delete the
  `:` form in jj 0.15+.

* The `--allow-large-revsets` flag for `jj rebase` and `jj new` was replaced by
  a `all:` before the revset. For example, use `jj rebase -d 'all:foo-'`
  instead of `jj rebase --allow-large-revsets -d 'foo-'`.

* The `--allow-large-revsets` flag for `jj rebase` and `jj new` can no longer be
  used for allowing duplicate destinations. Include the potential duplicates
  in a single expression instead (e.g. `jj new 'all:x|y'`).

* The `push.branch-prefix` option was renamed to `git.push-branch-prefix`.

* The default editor on Windows is now `Notepad` instead of `pico`.

* `jj` will fail attempts to snapshot new files larger than 1MiB by default.
  This behavior
  can be customized with the `snapshot.max-new-file-size` config option.

* Author and committer signatures now use empty strings to represent unset
  names and email addresses. The `author`/`committer` template keywords and
  methods also return empty strings.
  Older binaries may not warn user when attempting to `git push` commits
  with such signatures.

* In revsets, the working-copy or remote symbols (such as `@`, `workspace_id@`,
  and `branch@remote`) can no longer be quoted as a unit. If a workspace or
  branch name contains whitespace, quote the name like `"branch name"@remote`.
  Also, these symbols will not be resolved as revset aliases or function
  parameters. For example, `author(foo@)` is now an error, and the revset alias
  `'revset-aliases.foo@' = '@'` will be failed to parse.

* The `root` revset symbol has been converted to function `root()`.

* The `..x` revset is now evaluated to `root()..x`, which means the root commit
  is no longer included.

* `jj git push` will now push all branches in the range `remote_branches()..@`
  instead of only branches pointing to `@` or `@-`.

* It's no longer allowed to create a Git remote named "git". Use `jj git remote
  rename` to rename the existing remote.
  [#1690](https://github.com/jj-vcs/jj/issues/1690)

* Revset expression like `origin/main` will no longer resolve to a
  remote-tracking branch. Use `main@origin` instead.

### New features

* Default template for `jj log` now does not show irrelevant information
  (timestamp, empty, message placeholder etc.) about the root commit.

* Commit templates now support the `root` keyword, which is `true` for the root
  commit and `false` for every other commit.

* `jj init --git-repo` now works with bare repositories.

* `jj config edit --user` and `jj config set --user` will now pick a default
  config location if no existing file is found, potentially creating parent
  directories.

* `jj log` output is now topologically grouped.
  [#242](https://github.com/jj-vcs/jj/issues/242)

* `jj git clone` now supports the `--colocate` flag to create the git repo
  in the same directory as the jj repo.

* `jj restore` gained a new option `--changes-in` to restore files
  from a merge revision's parents. This undoes the changes that `jj diff -r`
  would show.

* `jj diff`/`log` now supports `--tool <name>` option to generate diffs by
  external program. For configuration, see [the documentation](docs/config.md).
  [#1886](https://github.com/jj-vcs/jj/issues/1886)

* A new experimental diff editor `meld-3` is introduced that sets up Meld to
  allow you to see both sides of the original diff while editing. This can be
  used with `jj split`, `jj move -i`, etc.

* `jj log`/`obslog`/`op log` now supports `--limit N` option to show the first
  `N` entries.

* Added the `ui.paginate` option to enable/disable pager usage in commands

* `jj checkout`/`jj describe`/`jj commit`/`jj new`/`jj squash` can take repeated
  `-m/--message` arguments. Each passed message will be combined into paragraphs
  (separated by a blank line)

* It is now possible to set a default description using the new
  `ui.default-description` option, to use when describing changes with an empty
  description.

* `jj split` will now leave the description empty on the second part if the
  description was empty on the input commit.

* `branches()`/`remote_branches()`/`author()`/`committer()`/`description()`
  revsets now support exact matching. For example, `branch(exact:main)`
  selects the branch named "main", but not "maint". `description(exact:"")`
  selects commits whose description is empty.

* Revsets gained a new function `mine()` that
  aliases `author(exact:"your_email")`.

* Added support for `::` and `..` revset operators with both left and right
  operands omitted. These expressions are equivalent to `all()` and `~root()`
  respectively.

* `jj log` timestamp format now accepts `.utc()` to convert a timestamp to UTC.

* templates now support additional string
  methods `.starts_with(x)`, `.ends_with(x)`
  `.remove_prefix(x)`, `.remove_suffix(x)`, and `.substr(start, end)`.

* `jj next` and `jj prev` are added, these allow you to traverse the history
  in a linear style. For people coming from Sapling and `git-branchles`
  see [#2126](https://github.com/jj-vcs/jj/issues/2126) for
  further pending improvements.

* `jj diff --stat` has been implemented. It shows a histogram of the changes,
  same as `git diff --stat`.
  Fixes [#2066](https://github.com/jj-vcs/jj/issues/2066)

* `jj git fetch --all-remotes` has been implemented. It fetches all remotes
  instead of just the default remote

### Fixed bugs

* Fix issues related to .gitignore handling of untracked directories
  [#2051](https://github.com/jj-vcs/jj/issues/2051).

* `jj config set --user` and `jj config edit --user` can now be used outside of
  any repository.

* SSH authentication could hang when ssh-agent couldn't be reached
  [#1970](https://github.com/jj-vcs/jj/issues/1970)

* SSH authentication can now use ed25519 and ed25519-sk keys. They still need
  to be password-less.

* Git repository managed by the repo tool can now be detected as a "colocated"
  repository.
  [#2011](https://github.com/jj-vcs/jj/issues/2011)

### Contributors

Thanks to the people who made this release happen!

* Alexander Potashev (@aspotashev)
* Anton Bulakh (@necauqua)
* Austin Seipp (@thoughtpolice)
* Benjamin Brittain (@benbrittain)
* Benjamin Saunders (@Ralith)
* Christophe Poucet (@poucet)
* Emily Kyle Fox (@emilykfox)
* Glen Choo (@chooglen)
* Ilya Grigoriev (@ilyagr)
* Kevin Liao (@kevincliao)
* Linus Arver (@listx)
* Martin Clausen (@maacl)
* Martin von Zweigbergk (@martinvonz)
* Matt Freitas-Stavola (@mbStavola)
* Oscar Bonilla (@ob)
* Philip Metzger (@PhilipMetzger)
* Piotr Kufel (@qfel)
* Preston Van Loon (@prestonvanloon)
* Tal Pressman (@talpr)
* Vamsi Avula (@avamsi)
* Vincent Breitmoser (@Valodim)
* Vladimir (@0xdeafbeef)
* Waleed Khan (@arxanas)
* Yuya Nishihara (@yuja)
* Zachary Dremann (@Dr-Emann)

## [0.8.0] - 2023-07-09

### Breaking changes

* The `jujutsu` and `jujutsu-lib` crates were renamed to `jj-cli` and `jj-lib`,
  respectively.

* The `ui.oplog-relative-timestamps` option has been removed. Use the
  `format_time_range()` template alias instead. For details, see
  [the documentation](docs/config.md).

* Implicit concatenation of template expressions has been disabled. Use
  `++` operator, `concat()`, or `separate()` function instead.
  Example: `description ++ "\n"`

* `jj git push` will consider pushing the parent commit only when the
  current commit has no content and no description, such as right after
  a `jj squash`.

* The minimum supported Rust version (MSRV) is now 1.64.0.

* The `heads()` revset function was split up into two functions. `heads()`
  without arguments is now called `visible_heads()`. `heads()` with one argument
  is unchanged.

* The `ui.default-revset` config was renamed to `revsets.log`.

* The `jj sparse` command was split up into `jj sparse list` and
  `jj sparse set`.

* `jj hide` (alias for `jj abandon`) is no longer available. Use `jj abandon`
  instead.

* `jj debug completion`, `jj debug mangen` and `jj debug config-schema` have
  been moved from `jj debug` to `jj util`.

* `jj` will no longer parse `br` as a git_ref `refs/heads/br` when a branch `br`
  does not exist but the git_ref does (this is rare). Use `br@git` instead.

* `jj git fetch` will no longer import unrelated branches from the underlying
  Git repo.

### New features

* `jj git push --deleted` will remove all locally deleted branches from the
  remote.

* `jj restore` without `--from` works correctly even if `@` is a merge
  commit.

* `jj rebase` now accepts multiple `-s` and `-b` arguments. Revsets with
  multiple commits are allowed with `--allow-large-revsets`.

* `jj git fetch` now supports a `--branch` argument to fetch some of the
  branches only.

* `jj config get` command allows retrieving config values for use in scripting.

* `jj config set` command allows simple config edits like
  `jj config set --repo user.email "somebody@example.com"`

* Added `ui.log-word-wrap` option to wrap `jj log`/`obslog`/`op log` content
  based on terminal width. [#1043](https://github.com/jj-vcs/jj/issues/1043)

* Nodes in the (text-based) graphical log output now use a `◉` symbol instead
  of the letter `o`. The ASCII-based graph styles still use `o`.

* Commands that accept a diff format (`jj diff`, `jj interdiff`, `jj show`,
  `jj log`, and `jj obslog`) now accept `--types` to show only the type of file
  before and after.

* `jj describe` now supports `--reset-author` for resetting a commit's author
  to the configured user. `jj describe` also gained a `--no-edit` option to
  avoid opening the editor.

* Added `latest(x[, n])` revset function to select the latest `n` commits.

* Added `conflict()` revset function to select commits with conflicts.

* `jj squash` AKA `jj amend` now accepts a `--message` option to set the
  description of the squashed commit on the command-line.

* The progress display on `jj git clone/fetch` now includes the downloaded size.

* The formatter now supports a "default" color that can override another color
  defined by a parent style.

* `jj obslog` and `jj log` now show abandoned commits as hidden.

* `jj git fetch` and `jj git push` will now use the single defined remote even
  if it is not named "origin".

* `jj git push` now accepts `--branch` and `--change` arguments together.

* `jj git push` now accepts a `-r/--revisions` flag to specify revisions to
  push. All branches pointing to any of the specified revisions will be pushed.
  The flag can be used together with `--branch` and `--change`.

* `jj` with no subcommand now defaults to `jj log` instead of showing help. This
  command can be overridden by setting `ui.default-command`.

* Description tempfiles created via `jj describe` now have the file extension
  `.jjdescription` to help external tooling detect a unique filetype.

* The shortest unique change ID prefixes and commit ID prefixes in `jj log` are
  now shorter within the default log revset. You can override the default by
  setting the `revsets.short-prefixes` config to a different revset.

* The last seen state of branches in the underlying git repo is now presented by
  `jj branch list`/`jj log` as a remote called `git` (e.g. `main@git`). They can
  also be referenced in revsets. Such branches exist in colocated repos or if
  you use `jj git export`.

* The new `jj chmod` command allows setting or removing the executable bit on
  paths. Unlike the POSIX `chmod`, it works on Windows, on conflicted files, and
  on arbitrary revisions. Bits other than the executable bit are not planned to
  be supported.

* `jj sparse set` now accepts an `--edit` flag which brings up the `$EDITOR` to
  edit sparse patterns.

* `jj branch list` can now be filtered by revset.

* Initial support for the Watchman filesystem monitor. Set
  `core.fsmonitor = "watchman"` in your repo to enable.

### Fixed bugs

* Modify/delete conflicts now include context lines
  [#1244](https://github.com/jj-vcs/jj/issues/1244).

* It is now possible to modify either side of a modify/delete conflict (any
  change used to be considered a resolution).

* Fixed a bug that could get partially resolved conflicts to be interpreted
  incorrectly.

* `jj git fetch`: when re-adding a remote repository that had been previously
  removed, in some situations the remote branches were not recreated.

* `jj git remote rename`: the git remote references were not rewritten with
  the new name. If a new remote with the old name and containing the same
  branches was added, the remote branches may not be recreated in some cases.

* `jj workspace update-stale` now snapshots the working-copy changes before
  updating to the new working-copy commit.

* It is no longer allowed to create branches at the root commit.

* `git checkout` (without using `jj`) in colocated repo no longer abandons
  the previously checked-out anonymous branch.
  [#1042](https://github.com/jj-vcs/jj/issues/1042).

* `jj git fetch` in a colocated repo now abandons branches deleted on the
  remote, just like in a non-colocated repo.
  [#864](https://github.com/jj-vcs/jj/issues/864)

* `jj git fetch` can now fetch forgotten branches even if they didn't move on
  the remote.
  [#1714](https://github.com/jj-vcs/jj/pull/1714)
  [#1771](https://github.com/jj-vcs/jj/pull/1771)

* It is now possible to `jj branch forget` deleted branches.
  [#1537](https://github.com/jj-vcs/jj/issues/1537)

* Fixed race condition when assigning change id to Git commit. If you've
  already had unreachable change ids, run `jj debug reindex`.
  [#924](https://github.com/jj-vcs/jj/issues/924)

* Fixed false divergence on racy working-copy snapshots.
  [#697](https://github.com/jj-vcs/jj/issues/697),
  [#1608](https://github.com/jj-vcs/jj/issues/1608)

* In colocated repos, a bug causing conflicts when undoing branch moves (#922)
  has been fixed. Some surprising behaviors related to undoing `jj git push` or
  `jj git fetch` remain.

### Contributors

Thanks to the people who made this release happen!

* Aaron Bull Schaefer (@elasticdog)
* Anton Bulakh (@necauqua)
* Austin Seipp (@thoughtpolice)
* Benjamin Saunders (@Ralith)
* B Wilson (@xelxebar)
* Christophe Poucet (@poucet)
* David Barnett (@dbarnett)
* Glen Choo (@chooglen)
* Grégoire Geis (@71)
* Ilya Grigoriev (@ilyagr)
* Isabella Basso (@isinyaaa)
* Kevin Liao (@kevincliao)
* Martin von Zweigbergk (@martinvonz)
* mlcui (@mlcui-corp)
* Samuel Tardieu (@samueltardieu)
* Tal Pressman (@talpr)
* Vamsi Avula (@avamsi)
* Waleed Khan (@arxanas)
* Yuya Nishihara (@yuja)

## [0.7.0] - 2023-02-16

### Breaking changes

* The minimum supported Rust version (MSRV) is now 1.61.0.

* The `jj touchup` command was renamed to `jj diffedit`.

* The `-i` option to `jj restore` was removed in favor of new `--from`/`--to`
  options to `jj diffedit`.

* To report the situation when a change id corresponds to multiple visible
  commits, `jj log` now prints the change id in red and puts `??` after it.
  Previously, it printed the word "divergent".

* `jj log` prefixes commit descriptions with "(empty)" when they contain no
  change compared to their parents.

* The `author`/`committer` templates now display both name and email. Use
  `author.name()`/`committer.name()` to extract the name.

* Storage of the "HEAD@git" reference changed and can now have conflicts.
  Operations written by a new `jj` binary will have a "HEAD@git" reference that
  is not visible to older binaries.

* The `description` template keyword is now empty if no description set.
  Use `if(description, description, "(no description set)\n")` to get back
  the previous behavior.

* The `template.log.graph` and `template.commit_summary` config keys were
  renamed to `templates.log` and `templates.commit_summary` respectively.

* If a custom `templates.log` template is set, working-copy commit will
  no longer be highlighted automatically. Wrap your template with
  `label(if(current_working_copy, "working_copy"), ...)` to label the
  working-copy entry.

* The `ui.relative-timestamps` option has been removed. Use the
  `format_timestamp()` template alias instead. For details on showing relative
  timestamps in `jj log` and `jj show`, see [the documentation](docs/config.md).

* `jj op log` now shows relative timestamps by default. To disable, set
  `ui.oplog-relative-timestamps` to `false`.

* The global `--no-commit-working-copy` is now called `--ignore-working-copy`.

* The `diff.format` config option is now called `ui.diff.format`. The old name
  is still supported for now.

* `merge-tools.<name>.edit-args` now requires `$left`/`$right` parameters.
  The default is `edit-args = ["$left", "$right"]`.

* The builtin `jj update` and `jj up` aliases for `jj checkout` have been
  deleted.

* Change IDs are now rendered using letters from the end of the alphabet (from
  'z' through 'k') instead of the usual hex digits ('0' through '9' and 'a'
  through 'f'). This is to clarify the distinction between change IDs and commit
  IDs, and to allow more efficient lookup of unique prefixes. This change
  doesn't affect the storage format; existing repositories will remain usable.

### New features

* The default log format now uses the committer timestamp instead of the author
  timestamp.

* `jj log --summary --patch` now shows both summary and diff outputs.

* `jj git push` now accepts multiple `--branch`/`--change` arguments

* `jj config list` command prints values from config and `config edit` opens
  the config in an editor.

* `jj debug config-schema` command prints out JSON schema for the jj TOML config
  file format.

* `jj resolve --list` can now describe the complexity of conflicts.

* `jj resolve` now notifies the user of remaining conflicts, if any, on success.
  This can be prevented by the new `--quiet` option.

* Per-repository configuration is now read from `.jj/repo/config.toml`.

* Background colors, bold text, and underlining are now supported. You can set
  e.g. `colors.error = { bg = "red", bold = true, underline = true }` in your
  `~/.jjconfig.toml`.

* The `empty` condition in templates is true when the commit makes no change to
  the three compared to its parents.

* `branches([needle])` revset function now takes `needle` as an optional
  argument and matches just the branches whose name contains `needle`.

* `remote_branches([branch_needle[, remote_needle]])` now takes `branch_needle`
  and `remote_needle` as optional arguments and matches just the branches whose
  name contains `branch_needle` and remote contains `remote_needle`.

* `jj git fetch` accepts repeated `--remote` arguments.

* Default remotes can be configured for the `jj git fetch` and `jj git push`
  operations ("origin" by default) using the `git.fetch` and `git.push`
  configuration entries. `git.fetch` can be a list if multiple remotes must
  be fetched from.

* `jj duplicate` can now duplicate multiple changes in one go. This preserves
  any parent-child relationships between them. For example, the entire tree of
  descendants of `abc` can be duplicated with `jj duplicate abc:`.

* `jj log` now highlights the shortest unique prefix of every commit and change
  id and shows the rest in gray. To customize the length and style, use the
  `format_short_id()` template alias. For details, see
  [the documentation](docs/config.md).

* `jj print` was renamed to `jj cat`. `jj print` remains as an alias.

* In content that goes to the terminal, the ANSI escape byte (0x1b) is replaced
  by a "␛" character. That prevents them from interfering with the ANSI escapes
  jj itself writes.

* `jj workspace root` prints the root path of the current workspace.

* The `[alias]` config section was renamed to `[aliases]`. The old name is
  still accepted for backwards compatibility for some time.

* Commands that draw an ASCII graph (`jj log`, `jj op log`, `jj obslog`) now
  have different styles available by setting e.g. `ui.graph.style = "curved"`.

* `jj split` accepts creating empty commits when given a path. `jj split .`
  inserts an empty commit between the target commit and its children if any,
  and `jj split any-non-existent-path` inserts an empty commit between the
  target commit and its parents.

* Command arguments to `ui.diff-editor`/`ui.merge-editor` can now be specified
  inline without referring to `[merge-tools]` table.

* `jj rebase` now accepts a new `--allow-large-revsets` argument that allows the
  revset in the `-d` argument to expand to several revisions. For example,
  `jj rebase -s B -d B- -d C` now works even if `B` is a merge commit.

* `jj new` now also accepts a `--allow-large-revsets` argument that behaves
  similarly to `jj rebase --allow-large-revsets`.

* `jj new --insert-before` inserts the new commit between the target commit and
  its parents.

* `jj new --insert-after` inserts the new commit between the target commit and
  its children.

* `author`/`committer` templates now support `.username()`, which leaves out the
  domain information of `.email()`.

* It is now possible to change the author format of `jj log` with the
  `format_short_signature()` template alias. For details, see
  [the documentation](docs/config.md).

* Added support for template aliases. New symbols and functions can be
  configured by `template-aliases.<name> = <expression>`. Be aware that
  the template syntax isn't documented yet and is likely to change.

* The `ui.diff-instructions` config setting can be set to `false` to inhibit the
  creation of the `JJ-INSTRUCTIONS` file as part of diff editing.

### Fixed bugs

* When sharing the working copy with a Git repo, we used to forget to export
  branches to Git when only the working copy had changed. That's now fixed.

* Commit description set by `-m`/`--message` is now terminated with a newline
  character, just like descriptions set by editor are.

* The `-R`/`--repository` path must be a valid workspace directory. Its
  ancestor directories are no longer searched.

* Fixed a crash when trying to access a commit that's never been imported into
  the jj repo from a Git repo. They will now be considered as non-existent if
  referenced explicitly instead of crashing.

* Fixed handling of escaped characters in .gitignore (only keep trailing spaces
  if escaped properly).

* `jj undo` now works after `jj duplicate`.

* `jj duplicate` followed by `jj rebase` of a tree containing both the original
  and duplicate commit no longer crashes. The fix should also resolve any
  remaining
  instances of https://github.com/jj-vcs/jj/issues/27.

* Fix the output of `jj debug completion --help` by reversing fish and zsh text.

* Fixed edge case in `jj git fetch` when a pruned branch is a prefix of another
  branch.

### Contributors

Thanks to the people who made this release happen!

* Aleksandr Mikhailov (@AM5800)
* Augie Fackler (@durin42)
* Benjamin Saunders (@Ralith)
* Daniel Ploch (@torquestomp)
* Danny Hooper (@hooper)
* David Barnett (@dbarnett)
* Glen Choo (@chooglen)
* Herby Gillot (@herbygillot)
* Ilya Grigoriev (@ilyagr)
* Luke Granger-Brown (@lukegb)
* Martin von Zweigbergk (@martinvonz)
* Michael Forster (@MForster)
* Philip Metzger (@PhilipMetzger)
* Ruben Slabbert (@rslabbert)
* Samuel Tardieu (@samueltardieu)
* Tal Pressman (@talpr)
* Vamsi Avula (@avamsi)
* Waleed Khan (@arxanas)
* Yuya Nishihara (@yuja)

## [0.6.1] - 2022-12-05

No changes, only changed to a released version of the `thrift` crate dependency.

## [0.6.0] - 2022-12-05

### Breaking changes

* Dropped candidates set argument from `description(needle)`, `author(needle)`,
  `committer(needle)`, `merges()` revsets. Use `x & description(needle)`
  instead.

* Adjusted precedence of revset union/intersection/difference operators.
  `x | y & z` is now equivalent to `x | (y & z)`.

* Support for open commits has been dropped. The `ui.enable-open-commits` config
  that was added in 0.5.0 is no longer respected. The `jj open/close` commands
  have been deleted.

* `jj commit` is now a separate command from `jj close` (which no longer
  exists). The behavior has changed slightly. It now always asks for a
  description, even if there already was a description set. It now also only
  works on the working-copy commit (there's no `-r` argument).

* If a workspace's working-copy commit has been updated from another workspace,
  most commands in that workspace will now fail. Use the new
  `jj workspace update-stale` command to update the workspace to the new
  working-copy commit. (The old behavior was to automatically update the
  workspace.)

### New features

* Commands with long output are paginated.
  [#9](https://github.com/jj-vcs/jj/issues/9)

* The new `jj git remote rename` command allows git remotes to be renamed
  in-place.

* The new `jj resolve` command allows resolving simple conflicts with
  an external 3-way-merge tool.

* `jj git push` will search `@-` for branches to push if `@` has none.

* The new revset function `file(pattern..)` finds commits modifying the
  paths specified by the `pattern..`.

* The new revset function `empty()` finds commits modifying no files.

* Added support for revset aliases. New symbols and functions can be configured
  by `revset-aliases.<name> = <expression>`.

* It is now possible to specify configuration options on the command line
  with the new `--config-toml` global option.

* `jj git` subcommands will prompt for credentials when required for HTTPS
  remotes rather than failing.
  [#469](https://github.com/jj-vcs/jj/issues/469)

* Branches that have a different target on some remote than they do locally are
  now indicated by an asterisk suffix (e.g. `main*`) in `jj log`.
  [#254](https://github.com/jj-vcs/jj/issues/254)

* The commit ID was moved from first on the line in `jj log` output to close to
  the end. The goal is to encourage users to use the change ID instead, since
  that is generally more convenient, and it reduces the risk of creating
  divergent commits.

* The username and hostname that appear in the operation log are now
  configurable via config options `operation.username` and `operation.hostname`.

* `jj git` subcommands now support credential helpers.

* `jj log` will warn if it appears that the provided path was meant to be a
  revset.

* The new global flag `-v/--verbose` will turn on debug logging to give
  some additional insight into what is happening behind the scenes.
  Note: This is not comprehensively supported by all operations yet.

* `jj log`, `jj show`, and `jj obslog` now all support showing relative
  timestamps by setting `ui.relative-timestamps = true` in the config file.

### Fixed bugs

* A bug in the export of branches to Git caused spurious conflicted branches.
  This typically occurred when running in a working copy colocated with Git
  (created by running `jj init --git-dir=.`).
  [#463](https://github.com/jj-vcs/jj/issues/463)

* When exporting branches to Git, we used to fail if some branches could not be
  exported (e.g. because Git doesn't allow a branch called `main` and another
  branch called `main/sub`). We now print a warning about these branches
  instead.
  [#493](https://github.com/jj-vcs/jj/issues/493)

* If you had modified branches in jj and also modified branches in conflicting
  ways in Git, `jj git export` used to overwrite the changes you made in Git.
  We now print a warning about these branches instead.

* `jj edit root` now fails gracefully.

* `jj git import` used to abandon a commit if Git branches and tags referring
  to it were removed. We now keep it if a detached HEAD refers to it.

* `jj git import` no longer crashes when all Git refs are removed.

* Git submodules are now ignored completely. Earlier, files present in the
  submodule directory in the working copy would become added (tracked), and
  later removed if you checked out another commit. You can now use `git` to
  populate the submodule directory and `jj` will leave it alone.

* Git's GC could remove commits that were referenced from jj in some cases. We
  are now better at adding Git refs to prevent that.
  [#815](https://github.com/jj-vcs/jj/issues/815)

* When the working-copy commit was a merge, `jj status` would list only the
  first parent, and the diff summary would be against that parent. The output
  now lists all parents and the diff summary is against the auto-merged parents.

### Contributors

Thanks to the people who made this release happen!

* Martin von Zweigbergk (@martinvonz)
* Benjamin Saunders (@Ralith)
* Yuya Nishihara (@yuja)
* Glen Choo (@chooglen)
* Ilya Grigoriev (@ilyagr)
* Ruben Slabbert (@rslabbert)
* Waleed Khan (@arxanas)
* Sean E. Russell (@xxxserxxx)
* Pranay Sashank (@pranaysashank)
* Luke Granger-Brown (@lukegb)

## [0.5.1] - 2022-10-17

No changes (just trying to get automated GitHub release to work).

## [0.5.0] - 2022-10-17

### Breaking changes

* Open commits are now disabled by default. That means that `jj checkout` will
  always create a new change on top of the specified commit and will let you
  edit that in the working copy. Set `ui.enable-open-commits = true` to restore
  the old behavior and let us know that you did so we know how many people
  prefer the workflow with open commits.

* `jj [op] undo` and `jj op restore` used to take the operation to undo or
  restore to as an argument to `-o/--operation`. It is now a positional
  argument instead (i.e. `jj undo -o abc123` is now written `jj undo abc123`).

* An alias that is not configured as a string list (e.g. `my-status = "status"`
  instead of `my-status = ["status"]`) is now an error instead of a warning.

* `jj log` now defaults to showing only commits that are not on any remote
  branches (plus their closest commit on the remote branch for context). This
  set of commits can be overridden by setting `ui.default-revset`. Use
  `jj log -r 'all()'` for the old behavior. Read more about revsets
  [here](https://github.com/jj-vcs/jj/blob/main/docs/revsets.md).
  [#250](https://github.com/jj-vcs/jj/issues/250)

* `jj new` now always checks out the new commit (used to be only if the parent
  was `@`).

* `jj merge` now checks out the new commit. The command now behaves exactly
  like `jj new`, except that it requires at least two arguments.

* When the working-copy commit is abandoned by `jj abandon` and the parent
  commit is open, a new working-copy commit will be created on top (the open
  parent commit used to get checked out).

* `jj branch` now uses subcommands like `jj branch create` and
  `jj branch forget` instead of options like `jj branch --forget`.
  [#330](https://github.com/jj-vcs/jj/issues/330)

* The [`$NO_COLOR` environment variable](https://no-color.org/) no longer
  overrides the `ui.color` configuration if explicitly set.

* `jj edit` has been renamed to `jj touchup`, and `jj edit` is now a new command
  with different behavior. The new `jj edit` lets you edit a commit in the
  working copy, even if the specified commit is closed.

* `jj git push` no longer aborts if you attempt to push an open commit (but it
  now aborts if a commit does not have a description).

* `jj git push` now pushes only branches pointing to the `@` by default. Use
  `--all` to push all branches.

* The `checkouts` template keyword is now called `working_copies`, and
  `current_checkout` is called `current_working_copy`.

### New features

* The new `jj interdiff` command compares the changes in commits, ignoring
  changes from intervening commits.

* `jj rebase` now accepts a `--branch/-b <revision>` argument, which can be used
  instead of `-r` or `-s` to specify which commits to rebase. It will rebase the
  whole branch, relative to the destination. The default mode has changed from
  `-r @` to `-b @`.

* The new `jj print` command prints the contents of a file in a revision.

* The new `jj git remotes list` command lists the configured remotes and their
  URLs.
  [#243](https://github.com/jj-vcs/jj/issues/243)

* `jj move` and `jj squash` now lets you limit the set of changes to move by
  specifying paths on the command line (in addition to the `--interactive`
  mode). For example, use `jj move --to @-- foo` to move the changes to file
  (or directory) `foo` in the working copy to the grandparent commit.

* When `jj move/squash/unsquash` abandons the source commit because it became
  empty and both the source and the destination commits have non-empty
  descriptions, it now asks for a combined description. If either description
  was empty, it uses the other without asking.

* `jj split` now lets you specify on the CLI which paths to include in the first
  commit. The interactive diff-editing is not started when you do that.

* Sparse checkouts are now supported. In fact, all working copies are now
  "sparse", only to different degrees. Use the `jj sparse` command to manage
  the paths included in the sparse checkout.

* Configuration is now also read from `~/.jjconfig.toml`.

* The `$JJ_CONFIG` environment variable can now point to a directory. If it
  does, all files in the directory will be read, in alphabetical order.

* The `$VISUAL` environment is now respected and overrides `$EDITOR`. The new
  `ui.editor` config has higher priority than both of them. There is also a new
  `$JJ_EDITOR` environment variable, which has even higher priority than the
  config.

* You can now use `-` and `+` in revset symbols. You used to have to quote
  branch names like `my-feature` in nested quotes (outer layer for your shell)
  like `jj co '"my-feature"'`. The quoting is no longer needed.

* The new revset function `connected(x)` is the same as `x:x`.

* The new revset function `roots(x)` finds commits in the set that are not
  descendants of other commits in the set.

* ssh-agent is now detected even if `$SSH_AGENT_PID` is not set (as long as
  `$SSH_AUTH_SOCK` is set). This should help at least macOS users where
  ssh-agent is launched by default and only `$SSH_AUTH_SOCK` is set.

* When importing from a git, any commits that are no longer referenced on the
  git side will now be abandoned on the jj side as well. That means that
  `jj git fetch` will now abandon unreferenced commits and rebase any local
  changes you had on top.

* `jj git push` gained a `--change <revision>` argument. When that's used, it
  will create a branch named after the revision's change ID, so you don't have
  to create a branch yourself. By default, the branch name will start with
  `push-`, but this can be overridden by the `push.branch-prefix` config
  setting.

* `jj git push` now aborts if you attempt to push a commit without a
  description or with the placeholder "(no name/email configured)" values for
  author/committer.

* Diff editor command arguments can now be specified by config file.
  Example:

      [merge-tools.kdiff3]
      program = "kdiff3"
      edit-args = ["--merge", "--cs", "CreateBakFiles=0"]

* `jj branch` can accept any number of branches to update, rather than just one.

* Aliases can now call other aliases.

* `jj log` now accepts a `--reversed` option, which will show older commits
  first.

* `jj log` now accepts file paths.

* `jj obslog` now accepts `-p`/`--patch` option, which will show the diff
  compared to the previous version of the change.

* The "(no name/email configured)" placeholder value for name/email will now be
  replaced if once you modify a commit after having configured your name/email.

* Color setting can now be overridden by `--color=always|never|auto` option.

* `jj checkout` now lets you specify a description with `--message/-m`.

* `jj new` can now be used for creating merge commits. If you pass more than
  one argument to it, the new commit will have all of them as parents.

### Fixed bugs

* When rebasing a conflict where one side modified a file and the other side
  deleted it, we no longer automatically resolve it in favor of the modified
  content (this was a regression from commit c0ae4b16e8c4).

* Errors are now printed to stderr (they used to be printed to stdout).

* Updating the working copy to a commit where a file's executable bit changed
  but the contents was the same used to lead to a crash. That has now been
  fixed.

* If one side of a merge modified a directory and the other side deleted it, it
  used to be considered a conflict. The same was true if both sides added a
  directory with different files in. They are now merged as if the missing
  directory had been empty.

* When using `jj move` to move part of a commit into an ancestor, any branches
  pointing to the source commit used to be left on a hidden intermediate commit.
  They are now correctly updated.

* `jj untrack` now requires at least one path (allowing no arguments was a UX
  bug).

* `jj rebase` now requires at least one destination (allowing no arguments was a
  UX bug).

* `jj restore --to <rev>` now restores from the working copy (it used to restore
  from the working copy's parent).

* You now get a proper error message instead of a crash when `$EDITOR` doesn't
  exist or exits with an error.

* Global arguments, such as `--at-op=<operation>`, can now be passed before
  an alias.

* Fixed relative path to the current directory in output to be `.` instead of
  empty string.

* When adding a new workspace, the parent of the current workspace's current
  checkout will be checked out. That was always the intent, but the root commit
  was accidentally checked out instead.

* When checking out a commit, the previous commit is no longer abandoned if it
  has a non-empty description.

* All commands now consistently snapshot the working copy (it was missing from
  e.g. `jj undo` and `jj merge` before).

## [0.4.0] - 2022-04-02

### Breaking changes

* Dropped support for config in `~/.jjconfig`. Your configuration is now read
  from `<config dir>/jj/config.toml`, where `<config dir>` is
  `${XDG_CONFIG_HOME}` or `~/.config/` on Linux,
  `~/Library/Application Support/` on macOS, and `~\AppData\Roaming\` on
  Windows.

### New features

* You can now set an environment variable called `$JJ_CONFIG` to a path to a
  config file. That will then be read instead of your regular config file. This
  is mostly intended for testing and scripts.

* The [standard `$NO_COLOR` environment variable](https://no-color.org/) is now
  respected.

* `jj new` now lets you specify a description with `--message/-m`.

* When you check out a commit, the old commit no longer automatically gets
  abandoned if it's empty and has descendants, it only gets abandoned if it's
  empty and does not have descendants.

* When undoing an earlier operation, any new commits on top of commits from the
  undone operation will be rebased away. For example, let's say you rebase
  commit A so it becomes a new commit A', and then you create commit B on top of
  A'. If you now undo the rebase operation, commit B will be rebased to be on
  top of A instead. The same logic is used if the repo was modified by
  concurrent operations (so if one operation added B on top of A, and one
  operation rebased A as A', then B would be automatically rebased on top of
  A'). See #111 for more examples.
  [#111](https://github.com/jj-vcs/jj/issues/111)

* `jj log` now accepts `-p`/`--patch` option.

### Fixed bugs

* Fixed crash on `jj init --git-repo=.` (it almost always crashed).

* When sharing the working copy with a Git repo, the automatic importing and
  exporting (sometimes?) didn't happen on Windows.

## [0.3.3] - 2022-03-16

No changes, only trying to get the automated build to work.

## [0.3.2] - 2022-03-16

No changes, only trying to get the automated build to work.

## [0.3.1] - 2022-03-13

### Fixed bugs

- Fixed crash when `core.excludesFile` pointed to nonexistent file, and made
  leading `~/` in that config expand to `$HOME/`
  [#131](https://github.com/jj-vcs/jj/issues/131)

## [0.3.0] - 2022-03-12

Last release before this changelog started.


[unreleased]: https://github.com/jj-vcs/jj/compare/v0.34.0...HEAD
[0.34.0]: https://github.com/jj-vcs/jj/compare/v0.33.0...v0.34.0
[0.33.0]: https://github.com/jj-vcs/jj/compare/v0.32.0...v0.33.0
[0.32.0]: https://github.com/jj-vcs/jj/compare/v0.31.0...v0.32.0
[0.31.0]: https://github.com/jj-vcs/jj/compare/v0.30.0...v0.31.0
[0.30.0]: https://github.com/jj-vcs/jj/compare/v0.29.0...v0.30.0
[0.29.0]: https://github.com/jj-vcs/jj/compare/v0.28.2...v0.29.0
[0.28.2]: https://github.com/jj-vcs/jj/compare/v0.28.1...v0.28.2
[0.28.1]: https://github.com/jj-vcs/jj/compare/v0.28.0...v0.28.1
[0.28.0]: https://github.com/jj-vcs/jj/compare/v0.27.0...v0.28.0
[0.27.0]: https://github.com/jj-vcs/jj/compare/v0.26.0...v0.27.0
[0.26.0]: https://github.com/jj-vcs/jj/compare/v0.25.0...v0.26.0
[0.25.0]: https://github.com/jj-vcs/jj/compare/v0.24.0...v0.25.0
[0.24.0]: https://github.com/jj-vcs/jj/compare/v0.23.0...v0.24.0
[0.23.0]: https://github.com/jj-vcs/jj/compare/v0.22.0...v0.23.0
[0.22.0]: https://github.com/jj-vcs/jj/compare/v0.21.0...v0.22.0
[0.21.0]: https://github.com/jj-vcs/jj/compare/v0.20.0...v0.21.0
[0.20.0]: https://github.com/jj-vcs/jj/compare/v0.19.0...v0.20.0
[0.19.0]: https://github.com/jj-vcs/jj/compare/v0.18.0...v0.19.0
[0.18.0]: https://github.com/jj-vcs/jj/compare/v0.17.1...v0.18.0
[0.17.1]: https://github.com/jj-vcs/jj/compare/v0.17.0...v0.17.1
[0.17.0]: https://github.com/jj-vcs/jj/compare/v0.16.0...v0.17.0
[0.16.0]: https://github.com/jj-vcs/jj/compare/v0.15.1...v0.16.0
[0.15.1]: https://github.com/jj-vcs/jj/compare/v0.15.0...v0.15.1
[0.15.0]: https://github.com/jj-vcs/jj/compare/v0.14.0...v0.15.0
[0.14.0]: https://github.com/jj-vcs/jj/compare/v0.13.0...v0.14.0
[0.13.0]: https://github.com/jj-vcs/jj/compare/v0.12.0...v0.13.0
[0.12.0]: https://github.com/jj-vcs/jj/compare/v0.11.0...v0.12.0
[0.11.0]: https://github.com/jj-vcs/jj/compare/v0.10.0...v0.11.0
[0.10.0]: https://github.com/jj-vcs/jj/compare/v0.9.0...v0.10.0
[0.9.0]: https://github.com/jj-vcs/jj/compare/v0.8.0...v0.9.0
[0.8.0]: https://github.com/jj-vcs/jj/compare/v0.7.0...v0.8.0
[0.7.0]: https://github.com/jj-vcs/jj/compare/v0.6.1...v0.7.0
[0.6.1]: https://github.com/jj-vcs/jj/compare/v0.6.0...v0.6.1
[0.6.0]: https://github.com/jj-vcs/jj/compare/v0.5.1...v0.6.0
[0.5.1]: https://github.com/jj-vcs/jj/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/jj-vcs/jj/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/jj-vcs/jj/compare/v0.3.3...v0.4.0
[0.3.3]: https://github.com/jj-vcs/jj/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/jj-vcs/jj/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/jj-vcs/jj/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/jj-vcs/jj/releases/tag/v0.3.0
