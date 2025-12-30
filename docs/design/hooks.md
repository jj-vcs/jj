# Native JJ Hooks

Authors: [Matt Stark](mailto:msta@google.com)

## Overview

In `jj`, we have no support to trigger things outside of jj to happen when
events in jj happen. The most obvious and simple use case for this is the
pre-upload hook. Consider the following things a user might want to do:
* I want to sign my commits.
* I want to always run `jj fix`.
* I want to validate that my commit message matches the style required by the
  remote.
* I want to validate that my commit passes some subset of tests before
  uploading.

All of these are highly reasonable things to be expected of a user to do before
running code, but it would be unreasonable to require the user to manually do
this every time they upload their code.

## What is a hook?

A hook is an (**event, callback**) pair. When an event happens, something is
triggered. This bears great similarities to pub-sub systems, but differ in a few
key areas:

*   Timeliness
    *   pub-sub provides no guarantees about when callbacks are completed
*   Failures
    *   Because callbacks are not guaranteed to be completed, it is impossible
        to check whether a callback succeeded.
*   Daemon
    *   Pub-sub **may**, depending on the implementation, require a daemon and
        monitor things in the background.

The main requirement of a hook which makes a pub-sub system unsuitable, however,
is that a hook generally involves:
1) Run the first part of a command (eg. take a snapshot)
2) Run the hook
3) Run the second part of the command (eg. upload the code)

## Goals

*   Support running a command before uploading in jj
    *   Investigate potential alternatives to hooks
*   Attempt to find other use cases where triggering external code from jj may
    be useful.
*   Implement this solution:
    *   In a secure manner (running arbitrary code in jj commands is bad)
    *   Introduce a general purpose framework to do this
    *   Implement this framework in an on-demand basis based on usefulness.

## Useful hooks

Through much discussion on
[generalized hook support](https://github.com/jj-vcs/jj/issues/3577), we have
realized that most "events" in Git are not so clear cut in jj.

For example, Git's `pre-commit` hook is supposed to be ran before `git commit`
creates a new commit. Semantically, it means "run this before committing code".
However, in jj, you can:
* `jj commit`, which is definitely semantically a commit
* `jj new` / `jj new @`, which is maybe semantically a commit
* `jj new <other>`, which could be a checkout or a stash (with the squash
  workflow), or perhaps a commit (with the edit workflow)
* `jj gerrit upload` / `jj git push`, which with the edit workflow are kind of
  a "commit" operation
* `jj split` is potentially semantically a commit.

So based on this, we will completely ignore Git's hooks, as most don't make
sense with jj, and instead implement jj-specific hooks on-demand based on
usefulness.

### Pre-upload

This hook is the most common, and the most useful hook. Chromium, for example,
has aliased `jj upload` to `tools/jj/upload.py`, which:

1.  Runs `jj fix`
2.  Runs `git cl presubmit` (which runs things such as a linter and some quick
    tests)
3.  Runs `jj gerrit upload`

This hook would be called by `jj gerrit upload`, other forges (eg.
`jj piper upload` for Google’s backend), and potentially `jj git push` (though
`jj git push` is a little more semantically questionable as it can be used for
other things as well). This could be mitigated, however, by identifying the
"upstream" / "primary" remote and only running the hooks before pushing to that.

### Sync hooks

Some repositories have an invariant that needs to be held by the version control
system. In most cases, however, they instead require the user to maintain that
invariant. As an example, chromium has a
[DEPS](https://source.chromium.org/chromium/chromium/src/+/main:DEPS?q=DEPS&ss=chromium%2Fchromium%2Fsrc)
file. It is assumed that whenever you change what you’re synced to, you are
required to run `gclient sync`, which:

1.  Syncs your Git submodules are at the correct version
2.  Runs a variety of hooks which do a variety of things
    ([source](https://source.chromium.org/chromium/chromium/src/+/main:DEPS;l=4053;drc=41151fb2f2e2ef4ad7fa4f11e095254cd1226f31)).
    As an example, it reads a “.gclient” file which says which OS you want to
    target, and then downloads the toolchain for that OS
3.  Ensures that any dependencies for the build (such as your build system) are
    at the correct version.

This ensures, for example, that you don’t need to download a submodule used only
for Windows when doing a checkout for Linux. Using a DEPS file is a relatively
common pattern at Google.

In an ideal world we would use a build system such as bazel that allows us to:
* Move the job of all of these hooks into the build system
* Check out these repos on-demand rather than every time

So there’s an argument to be made that this shouldn’t be in scope of the version
control system. Whether this should be in scope of the version control system is
probably based on if others in the industry do similar things.

We'll leave this out of scope of the initial solution. We may reconsider it
later, but better to assume this won't be implemented.

## Detailed Design

### Security

Security has been an ongoing issue with Git hooks, as they allow for execution
of arbitrary code. This is a known issue in jj, but rather than reimplementing
the wheel, jj's hooks will be configuration.

This means that a repo looking to provide hooks does not put the hooks in the
repo. They instead put it in a managed config file. The UX for the user is then:
* The user is told that a managed config file exists, and is asked to either:
  * Trust the managed config ("installing" the hooks and automatically
    installing any future hooks created for the repo)
  * Notify you whenever the repo's config changes (notify the user whenever
    their hooks are out of date)
  * Disable the managed config (don't install the hooks)

This ensures that we inherit the best-practices for config security. If we
update said best practices, we get it for hooks as well. It also unifies the
user experience to simplify the setup process.

### Pre-upload hooks

Pre-upload hooks fill a large variety of purposes.

The most common kinds of general-purpose hooks, which we will definitely need to
support, are:
* Validate something about the description 
  * Only needs the commit description, no checkout required
* Validate something about the code (eg. Run lints, or run a specific presubmit
  tool)
  * *May* need a checkout
* Wants to use `jj run` under the hood
  * Until `jj run` is implemented, the script probably needs to run
    `jj edit <commit>`, then once it's done `jj edit <original commit>`
  * Ideally wants a VFS
  * Due to performance reasons, some repos may want to only run on the heads
* Modify the description (eg. add a Git footer)
  * Doesn’t need a checkout
  * Can rewrite commits

To solve this, I propose we take a similar approach to our fix tools. Namely,
our configuration will look like:

```toml
# Fix and sign specifically do not support commands.
[hooks.pre-upload.fix]
enabled = true
order = 0 # Default (fix always runs first by default)
[hooks.pre-upload.sign]
enabled = true
order = 99999 # Default (sign always runs last by default)
[hooks.pre-upload.my-tool]
# This defaults to enabled, similar to fix.tools.*
command = ["python3", "$root/my_hook.py"]
order = 1 # Default
```

Hooks will be ran in ascending order, with multiple hooks of the same order
being an error.

A hook will need to meet a spec. Specifically, `jj` will pass some json to
`$COMMAND` via stdin:
```json
{
    // Probably not required, but no harm providing this.
    "operation": "<operation ID>"
    // commit ID of @
    // This is provided because many checks (eg. run a test) only work if the
    // commit under test is @ (or @- with an empty @).
    "working_copy": "<commit ID>",
    // All commits that we are going to upload.
    // In topological order.
    "to_upload": ["<commit ID>", "<commit ID>"],

    // @ | to_upload
    "commits": {
        "<commit ID>": {
            // The colored format used in jj log.
            // This is provided so that the pre-upload check can print nice
            // error messages showing which commit failed validation.
            "pretty": "...",
            // The commit ID is subject to change if rewriting commits.
            // So if you rewrite the first commit in the chain, you can then
            // interact with the new parent via the change ID of the parent
            // commit.
            // Similarly, tools that write to disk may modify @, so this can be
            // used when restoring the working copy.
            "change_id": "...",
            // Many tools will validate the commit description.
            "description": "...",
            // This is primarily useful for @. Some tools may require @ to be
            // empty.
            "empty": false,
            // May contain commit IDs not in "commits".
            // This is useful to understand the shape of the graph.
            // For example, I can check "is the commit being uploaded @-"
            "parents": ["<commit ID>", "<commit ID>"],
        }
    }
}
```

* The working directory of the command will always be the repo root
* stdin will be json data meeting the above specification.
* The stderr of the command will be passed through to the stderr of the main
  program
  * A nonzero exit code will result in the main program complaining that a
    hook failed.
* The command must not write to stdout, as we reserve it in case we want to
  use that mechanism to write info back to jj in a future version of hooks.

In the initial version, we will require that hooks are (with the exception of
the builtin hooks fix & sign), readonly.

In future versions, there are a few things we could do to allow more interesting
hooks that can write as well as read. It's unclear what the right approach would
be:
* We could allow the command to print a list of operations to stdout such as:
  * `[{"op": "describe", "commit_id": "<commit_id>", "description": "new description"}]`
* We could migrate it to Mahou (still a long way away), and allow it to run
  something like:
  * `new_transaction().describe("<commit_id>", "new description").finish()`
* We could, after running the hooks, check whether any commits had been
  rewritten
* This might be a little more expensive, but would be simpler for the user.

We may also consider in the future integration with a pub/sub API. Hooks could
run under a daemon created by and could be non-blocking.

### Alternatives considered

### Mahou (scripting language for jj)

Mahou does not directly solve the problem hooks solve. This is because even if
you could create an alias `jj upload`, for example, which ran a Mahou script
that had the same effect as running these hooks, then running `jj gerrit upload`
directly will bypass that entirely.

However, it does put a lot of the groundwork in place to solve the issue. In the
distant future, it is likely that hooks will be Mahou scripts, which should
significantly improve the UX, as it will allow direct access to jj objects and
functions from the hook.

One can imagine, for example, a `.config/jj/hooks.mahou` script which contains:
```py
def preupload_hooks(repo: Repo, commits: list[Commit]) -> list[Commit]:
  tx = repo.start_transaction()
  commits = tx.fix(commits) # returns list of rewritten commits
  for commit in commits:
    validate_commit_message(commit)
  for commit in commits:
    repo.jj_run(linter, commit)
  commits = tx.sign(commits) # returns list of rewritten commits
  tx.finish()
```

And your `config.toml` would just contain: `hooks = ".config/jj/hooks.mahou"`

It would simply load the Mahou script, look for the function `preupload_hooks`
with the signature `fn(Repo, list[Commit]) -> list[Commit]`, and call it. This
would give extremely precise control over how hooks are run, and allow the
performance impact to be minimized by giving that precise control to the Mahou
script.
