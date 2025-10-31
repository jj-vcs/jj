# Jujutsu for Git experts

People who are proficient with Git often ask what benefit there is to using
Jujutsu. This page explains the practical advantages for Git experts, with
examples showing how common workflows become easier, safer, or faster with
Jujutsu.

## Git can be used side-by-side in the same repository

Jujutsu repositories are colocated by default, so you can use `jj` and `git`
side-by-side. If you find a situation that's easier with Git, run the `git`
command and return to `jj` when you're done.

Colocation makes migration easier because you can adopt Jujutsu for the
workflows it improves without losing access to the Git commands and tools you
already know.

## Automatic and safer history editing

If you frequently amend, reorder, or squash commits, Jujutsu can often perform
the same operations in fewer commands.

Suppose you want to amend an older commit and squash it into earlier history.
With Git you might do this in three steps:

```sh
git add file1 file2
git commit --fixup abc
git rebase -i --autosquash
```

With Jujutsu, you simply squash the changes directly into the commit you want to
amend. All descendants are automatically rebased on top of the amended commit:

```sh
jj squash --into abc file1 file2
```

## Undo is more powerful than using the reflog

Git's reflog is powerful, but it's per-ref and can be awkward to use when
multiple refs and operations are involved.

Jujutsu's operation log records the state of the entire repository: Every change
is an operation you can inspect, and you can restore to any earlier state with
one command.

Common uses of the operation log:

- `jj undo` reverts the last operation in one step, without needing to figure
  out which ref to reset. You can repeat `jj undo` to continue stepping backwards
  in time.

- `jj op log -p` shows operations with diffs so you can inspect what happened.

- `--at-operation ID` lets you run commands as if the repository were in a
  previous state.

## The evolution log shows the history of a single change

The Git reflog shows how refs moved over time, but makes it difficult to see how
a particular commit evolved over time. Jujutsu's evolution log ("evolog") shows
exactly this: Each time a change is rewritten, the update is visible in the
evolog.

You can use the evolog to find a previous version, then `jj restore` to restore
the complete or partial contents to the current version.

## Conflict resolution can be deferred

Git forces you to resolve conflicts immediately while merging or rebasing is
in progress. Jujutsu lets you defer that work, which is useful when conflicts
are complicated or when you want to switch context to fix something else.

You can leave a commit in a conflicted state, continue other work, and return
later. This reduces the cost of context switching when resolving a large number
of conflicts.

Because Jujutsu records the inputs to conflicts, not just conflict markers, it
can sometimes automatically resolve conflicts after a rebase. When performing
several rebases in sequence, some conflicts may be introduced by one and then
later automatically resolved by another, without any manual effort to resolve
the conflicts.

## `jj absorb` makes it easier to update a patch stack

When amending several commits in a stack of changes, Git requires you to run
`git commit --fixup <ID>` at least once for each commit before running `git
rebase --autosquash`.

`jj absorb` is useful when you've made small fixes in the working copy and want
them incorporated into recent commits. It automatically moves each change in the
working copy into the previous commit where that line was changed.

It doesn't solve all cases: If multiple commits in the stack modified the same
line as was changed in the working copy, it will not move that change. But it
does help the trivial cases, leaving you to decide how to squash the remaining
changes.
