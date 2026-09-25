# jj Hooks

Author: [Sascha Andres](mailto:sascha.andres@livingit.de)

## Summary

Add a hook mechanism to jj: user-provided executables that jj runs at defined
points in a command's lifecycle. This proposal covers the mechanism itself plus
two initial hook kinds, `git-pre-push` and `git-post-push`, run around
`jj git push`'s use of the gitoxide-based push path (`git::push_refs` in
`lib/src/git.rs`).

Hooks can be stored globally, under `$config_dir/jj/hooks/` (the same
`root_config_dir()` used for the user config file, see `cli/src/config.rs`), or
per repository, under `<workspace root>/.jj-hooks/`, a plain directory next to
`.jj` rather than inside it, so it is an ordinary tracked path and gets
versioned like any other file in the repo.

Repository-scoped hooks are executable code that arrives with a clone, which is
exactly the "zip file problem" `docs/design/secure-config.md` already flags as
a concern for future hook designs. This document treats the trust model as a
first-class part of the design, not an add-on: repository-scoped hooks are
inert until a user explicitly enables them for that repository, and that
approval is revoked automatically the moment the hook's content changes,
including a change that arrives via `jj git fetch`.

## Goals and non-goals

Goals:

* A general hook-invocation mechanism in `jj-lib`, extensible to more hook
  kinds later, starting with `git-pre-push` and `git-post-push`.
* Global hooks (`$config_dir/jj/hooks/`) that behave like ordinary trusted user
  configuration: no extra gate, since they cannot arrive via a clone.
* Repository-scoped hooks (`.jj-hooks/`) that require an explicit, per-clone
  opt-in before jj will execute them, and that lose that approval whenever
  their content changes.
* A `jj-native` invocation contract (environment variables plus a structured
  JSON payload on stdin) describing the bookmarks/tags and commit ids being
  pushed, rather than reusing git's raw-ref pre-push contract.
* A `--no-hooks` flag, available on any command that would otherwise run one
  or more hooks, that skips all hook execution for that single invocation.

Non-goals for this iteration:

* Hook kinds other than `git-pre-push`/`git-post-push` (e.g. pre-commit style
  hooks around `jj describe`/`jj new`).
* Hooking `jj gerrit upload` (`cli/src/commands/gerrit/upload.rs`), which has
  its own push path.
* Sandboxing hook execution. Hooks run with the same privileges as jj itself,
  same as git hooks and merge/diff tools already do.
* Byte-for-byte compatibility with git's `pre-push` hook interface.

## Prior work

* Git's own `pre-push` hook: installed under `.git/hooks/`, which is
  deliberately excluded from what `git clone` populates, precisely so a
  cloned repo cannot ship hooks that run unattended.
* Tools like husky and pre-commit invert that default: hooks live in the
  versioned tree, and a separate install step (`pre-commit install`, husky's
  `prepare` script) is what actually wires them into the local checkout. That
  install step is the opt-in this design borrows.
* direnv's trust model (`direnv allow`) revokes trust automatically when the
  trusted file's content changes, which is the precedent for the re-trust
  behavior described below.

## Overview

### Locations and precedence

* Global: `$config_dir/jj/hooks/<hook-name>` (`root_config_dir()` joined with
  `hooks`; on Linux this is `~/.config/jj/hooks/`).
* Repository-scoped: `<workspace root>/.jj-hooks/<hook-name>`.

For a given hook name, if a repository-scoped hook file exists and repository
hooks are enabled and currently trusted for this repository (see below), it
runs and the global hook of the same name does not. Otherwise, the global
hook runs if present. If neither exists, nothing runs. This is a full
override, not a merge; a repository that wants the global hook's behavior
needs to invoke it explicitly.

### Exit code semantics

Every hook is a plain executable (script with a shebang, or binary). Exit code
`0` means success. Any nonzero exit code is treated as failure; `1` is the
conventional code a hook author should use, but jj does not special-case it.

* `git-pre-push` failing aborts the push: nothing is sent to any remote.
* `git-post-push` failing is reported as a warning; the push already happened
  and cannot be undone by a hook.

### Hook kinds

`git-pre-push` runs once per `jj git push` invocation, before the first call
to `git::push_refs`, covering every remote matched by that invocation
(`--remote` can be repeated). It does not run once per remote: a single
rejection cancels the push to all matched remotes, none of them get
partially pushed. This differs from git's own pre-push hook, which git
invokes once per `git push <remote>` process; jj's command can target several
remotes in one call, and per-remote partial success was ruled out as
confusing here.

`git-post-push` runs once after all matched remotes have been attempted,
only if every one of them fully succeeded
(`GitPushStats::all_ok()` for each). If any remote's push was rejected,
`git-post-push` does not run, since "pushed successfully" did not hold.

Neither hook runs for `--dry-run`, since nothing is actually pushed.

### Skipping hooks for one invocation (`--no-hooks`)

Any command that would run a hook accepts a `--no-hooks` flag. When passed,
every hook that would otherwise run for that invocation, global or
repository-scoped, is skipped; the command proceeds exactly as it would if no
hook file existed. This is per-invocation only:

* It does not touch the enablement flag or any recorded content hash from
  the trust model above. A repository-scoped hook that is enabled and
  trusted stays enabled and trusted; it is simply not invoked this once.
* It applies uniformly regardless of hook kind, so `jj git push --no-hooks`
  skips both `git-pre-push` and `git-post-push` for that push, not just one
  of them.
* It is not a trust decision and does not require the hook to be otherwise
  runnable: `--no-hooks` also suppresses the "skipped, untrusted" warning
  from the trust model, since the hook is being skipped deliberately, not
  because content verification failed.
* It has no effect on a command with no configured hook of any kind for
  that invocation; jj does not warn about a no-op `--no-hooks`.

The flag is defined once, alongside `HookKind`, and reused by every
hook-invocation call site (see "`jj-lib` primitives" below), so a future
hook kind gets `--no-hooks` support by construction rather than needing its
own opt-out flag.

### Invocation contract

Working directory: the workspace root.

Environment variables, kept deliberately minimal: `JJ_HOOK` (hook name),
`JJ_REPO_ROOT`. Everything else about the operation lives in the stdin JSON
payload rather than further env vars, so the env contract stays stable as
the payload evolves.

Stdin: a single JSON envelope shape shared by both hook kinds, with a
schema-version field, and a list of remotes, each with the bookmark/tag
updates being (or having been) pushed: kind (bookmark or tag), name, old
commit id (nullable, absent means the ref is being created), new commit id
(nullable, absent means deletion). The per-update outcome field (pushed,
rejected, remote-rejected; sourced from `GitPushStats`) is simply absent for
`git-pre-push` and present for `git-post-push`, rather than the two kinds
having distinct top-level schemas.

Stdout/stderr are passed through to the user's terminal, the same way
`GitSubprocessUi` already streams git's own output during push, so a
declining hook can explain itself.

### Trust model

Global hooks need no gate: they cannot be delivered by cloning a repository,
so they are exactly as trusted as the rest of the user's configuration.

Repository-scoped hooks need two layers, both required:

1. **Explicit per-repository enablement.** A repository-scoped hook never
   runs until the user runs `jj hook enable` in that specific checkout. This
   flag is stored in the repository's already-existing secure, out-of-tree
   config (`lib/src/secure_config.rs`, the same mechanism backing
   `jj config set --repo`), keyed by the repository's config-id and bound to
   its path. That store is generated locally by jj itself and is never
   populated from the cloned tree, so it cannot be pre-set by a malicious
   repository the way an in-tree file could be. `jj hook disable` turns it
   back off.

   TODO (explicit, must not be dropped from the implementation): this
   enablement is per repository and per clone. Cloning a repository that
   already has `.jj-hooks/` populated must never itself enable execution;
   the user has to run `jj hook enable` again in the new checkout.

2. **Per-file content trust, re-checked on every run.** At the moment
   `jj hook enable` is run, jj records a content hash of every
   `.jj-hooks/*` file that currently exists, alongside the enabled flag, in
   the same secure per-repository config. Before running a repository-scoped
   hook, jj recomputes that file's hash and compares it to the recorded one.
   A mismatch, whether from a local edit, a `jj git fetch` that pulled in a
   commit touching `.jj-hooks/`, a rebase, or a checkout of a different
   bookmark, means the file is currently untrusted: jj skips it (falling
   back to the global hook of the same name, or running nothing) and prints
   a warning pointing at `jj hook enable` to re-approve. The overall
   "enabled" flag from layer 1 is not cleared by this; only that file's
   approval is stale until re-granted. A file that did not exist at
   enable-time is untrusted by construction until it is explicitly approved.

   This is the mechanism that satisfies "the trust step must be executed
   again if pulling from a remote changes the hooks": trust is tied to
   content, not to a one-time decision about the repository as a whole.

## Detailed Design

### `jj-lib` primitives

New module, e.g. `lib/src/hooks.rs`:

* A `HookKind` enum (`GitPrePush`, `GitPostPush`), deliberately small so
  adding a kind later is additive.
* Path resolution: global directory from `ConfigEnv::root_config_dir()`
  joined with `hooks`; repository directory as `<workspace root>/.jj-hooks`.
* A hook-invocation type carrying the resolved executable path, working
  directory, environment variables, and the stdin payload, plus a runner
  that spawns it via `std::process::Command`, following the same idiom as
  `lib/src/gpg_signing.rs` and `lib/src/ssh_signing.rs`, and reusing
  `lib/src/subprocess_util.rs::suppress_console_window` for Windows parity.
  The runner takes a `no_hooks: bool` alongside the hook kind and returns
  immediately (as if no hook file existed, and without touching the trust
  check) when it is set, so `--no-hooks` is enforced in one place rather
  than at each call site.
* An error type analogous to `GitPushError`/`ConfigGetError` (spawn failure,
  nonzero exit, hooks disabled/untrusted).
* The content-hash trust check itself (hash file, compare to a stored value)
  belongs here too, so it is independent of any particular hook kind.

### Trust storage

Extend the repository-scoped secure config with a `hooks` section: an
`enabled` boolean and a table of file name to content hash. Read and written
through the existing `SecureConfig` machinery, no new storage layer.

### CLI wiring

New command family under `cli/src/commands/hook/`, mirroring the
`cli/src/commands/config/` layout (one file per subcommand, a `mod.rs`
dispatching them, registered in the `Commands` enum in
`cli/src/commands/mod.rs`):

* `jj hook enable`: lists the `.jj-hooks/*` files it is about to trust,
  states plainly that jj will execute them automatically from now on in this
  checkout, and records the enabled flag and current hashes.
* `jj hook disable`: clears the enabled flag and the stored content hashes.
  Re-enabling always re-lists and re-approves current file contents, even if
  unchanged since disable; disable resets to a clean slate rather than
  leaving stale trust state around.
* `jj hook status` (or similar): shows, per hook name, whether a global
  and/or local file exists, whether local ones are currently trusted, and
  which one would actually run.

`cli/src/commands/git/push.rs::cmd_git_push`:

* A new `--no-hooks` flag on `GitPushArgs`, threaded into the hook runner
  described above (see "`jj-lib` primitives"). This is a per-subcommand
  flag, not a global one: it is added to each hook-bearing command's own
  `Args` struct, mirroring `--dry-run`. This is the first call site; later
  commands that grow hook support add the same flag name to their own args
  rather than a differently-named one, so the user-facing contract stays
  consistent.
* After computing `by_remote` and confirming there is something to push
  (the existing "Nothing changed" check), and only when not `--dry-run`, run
  `git-pre-push` once, covering all of `by_remote`. A nonzero exit returns a
  `CommandError` before the loop that calls `git::push_refs` for any remote.
  Skipped entirely if `--no-hooks` was passed.
* After that loop, if every remote's `GitPushStats::all_ok()` and not
  `--dry-run`, run `git-post-push`; a nonzero exit is reported through
  `ui.warning_default()`, not as a command error. Also skipped if
  `--no-hooks` was passed.

### Config schema and docs

* `docs/config-schema.json` and `cli/src/config-schema.json`: document the
  `hooks.*` keys used by the trust store, primarily so editors do not flag
  them as unknown if a user inspects the repo config directly; the intended
  entry point remains the `jj hook` subcommands, not manual editing.
* `docs/config.md`: new section on hook locations, precedence, and the two
  hook kinds.
* `docs/git-compatibility.md`: a short note that jj hooks are a distinct
  mechanism from git's `.git/hooks/`, different location, different
  contract, so users familiar with git hooks are not misled.
* CLI help text on the new subcommands, regenerated into
  `docs/cli-reference.md` via `cargo insta test --accept --workspace --
  test_generate` per the existing convention (`cli/tests/test_generate_md_cli_help.rs`).

### Tests

* `jj-lib` unit tests for path resolution, precedence, and the content-hash
  trust check.
* CLI integration tests extending `cli/tests/test_git_push.rs` (which
  already has a pattern for writing an executable test hook cross-platform,
  see `test_git_push_rejected_by_remote`): local hook absent falls back to
  global; local hook present but not enabled is skipped; enabled and exit 0
  proceeds and runs `git-post-push`; enabled and exit 1 aborts before any
  remote is touched; enabled hook whose content changes after enable-time
  (simulating a fetch that touched `.jj-hooks/`) is skipped with a warning,
  not silently run; `--no-hooks` with an enabled, trusted, exit-1 hook
  configured still succeeds and the hook binary is never invoked, and the
  enabled/trusted state is unchanged afterward; `--no-hooks` with no hook
  configured at all behaves identically to a plain `jj git push`.
* A new `cli/tests/test_hook_command.rs` for `jj hook enable/disable/status`.

## Alternatives considered

**Always run repository-scoped hooks, no enable step.** Matches the literal
request most directly but reproduces the exact scenario
`docs/design/secure-config.md` names as a reason to be careful with hooks.
Rejected.

**One-time trust, not re-checked on content change.** Simpler to implement,
but a compromised or malicious remote could land a change to
`.jj-hooks/git-pre-push` in an ordinary-looking commit and have it run the
next time the victim pushes, no new consent required. Rejected once the
requirement to re-trust on change was raised.

**Mirror git's literal pre-push contract** (remote name/URL as argv, `local
ref local-oid remote-ref remote-oid` lines on stdin). Would let people reuse
existing git hook scripts unmodified. Rejected for now because jj's push
model is bookmarks/tags with commit ids, not raw refs with OIDs, and forcing
that shape back to a jj-native structure said more to bikeshed than to solve;
worth revisiting as an optional compatibility layer once the jj-native
contract exists.

**Run `git-pre-push` per remote, allowing partial success.** Closer to git's
own semantics and to how `git::push_refs` is already called in a loop.
Rejected in favor of a single all-or-nothing run per invocation, to keep the
failure model simple to reason about.

## Related work

* `docs/design/secure-config.md`, which already anticipated this feature and
  is the reason the trust model here treats the zip-file scenario as a
  primary constraint rather than an afterthought.
* Git's `pre-push` hook (see "Prior work" above).

## Future possibilities

* Additional hook kinds (e.g. around `jj new`, `jj describe`, working-copy
  updates).
* Extending hook coverage to `jj gerrit upload`.
* An optional git-compatible invocation mode for `git-pre-push`, for users
  migrating existing git hook scripts.
* Diffing old versus new hook content in the `jj hook enable` re-approval
  flow, rather than only reporting that content changed.
