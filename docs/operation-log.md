# Operation log

## Introduction

Jujutsu records each operation that modifies the repo in the "operation log".
You can see the log with `jj op log`. Each operation object contains a snapshot
of how the repo looked at the end of the operation. We call this snapshot a
"view" object. The view contains information about where each bookmark, tag, and
Git ref (in Git-backed repos) pointed, as well as the set of heads in the repo,
and the current working-copy commit in each workspace. The operation object also
(in addition to the view) contains pointers to the operation(s) immediately
before it, as well as metadata about the operation, such as timestamps,
username, hostname, description.

The operation log allows you to undo operations one-by-one (`jj undo`) or even
revert a specific one which isn't the most recent operation (`jj op revert`). It
also lets you restore the entire repo to the way it looked at an earlier point
(`jj op restore`).

When referring to operations, you can use `@` to represent the current
operation.

The following operators are supported:

* `x-`: Parents of `x` (e.g. `@-`)
* `x+`: Children of `x`

## Divergent operations

One benefit of the operation log (and the reason for its creation) is that it
allows lock-free concurrency -- you can run concurrent `jj` commands without
corrupting the repo, even if you run the commands on different machines that
access the repo via a distributed file system (as long as the file system
guarantees that a write is only visible once previous writes are visible). When
you run a `jj` command, it will start by loading the repo at the latest
operation. It will not see any changes written by concurrent commands. If there
are conflicts, you will be informed of them by subsequent `jj st` and/or
`jj log` commands.

As an example, let's say you had started editing the description of a change and
then also update the contents of the change (maybe because you had forgotten the
editor). When you eventually close your editor, the command will succeed and
e.g. `jj log` will indicate that the change has diverged.

## Undoing operations when other workspaces write operations too

`jj undo` usually undoes the latest operation by restoring the repo to the
operation before it. That isn't possible, or not what you want, when the latest
operation was performed in another workspace, or is a merge of concurrent
operations. In those cases, `jj undo` undoes the latest operation performed in
the current workspace instead. Restoring the operation before it would also
undo the operations performed in other workspaces since, so `jj undo` reverts
it like `jj op revert` does. Running `jj undo` again continues with older
operations of the current workspace.

The latest operation of the current workspace is the first one that `jj op log`
lists, skipping merge operations and operations that have already been undone.
Operations that don't record a workspace, such as `add workspace` operations and
operations written by some tools, don't count as the current workspace's.

Reverting an operation merges the inverse of its changes with the changes made
since, so the result can have conflicts. For example, if another workspace
moved a bookmark after the operation you undo moved it, the bookmark becomes
conflicted, and if another workspace rewrote a commit after the operation you
undo rewrote it, the change becomes divergent. `jj undo` warns about these.

Pass `--allow-cross-workspace` to undo operations performed in other workspaces
as well. `jj undo` then undoes the latest operation even if it was performed in
another workspace, and after a merge of concurrent operations, it reverts the
latest operation of any workspace.

## Loading an old version of the repo

The top-level `--at-op`/`--at-operation` option allows you to load the repo at a
specific operation. This can be useful for understanding how your repo got into
the current state. It can be even more useful for understanding why someone
else's repo got into its current state.

When you use `--at-op`, the automatic snapshotting of the working copy will not
take place. When referring to a revision with the `@` symbol (as many commands
do by default), that will resolve to the working-copy commit recorded in the
operation's view (which is actually how it always works -- it's just the
snapshotting that's skipped with `--at-op`).

As a top-level option, `--at-op` can be passed to any command. However, you
will typically only want to run read-only commands. For example, `jj log`,
`jj st`, and `jj diff` all make sense. It's still possible to run e.g.
`jj --at-op=<some operation ID> describe`. That's equivalent to having started
`jj describe` back when the specified operation was the most recent operation
and then let it run until now (which can be done for that particular command by
not closing the editor). There's practically no good reason to do that other
than to simulate concurrent commands.
