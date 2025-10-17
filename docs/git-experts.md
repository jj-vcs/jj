# Jujutsu for Git experts

People who are proficient with Git often ask what benefit there is to using
Jujutsu. This page explains the practical advantages for Git experts, with
examples showing how common workflows become easier, safer, or faster with
Jujutsu.

## Git can be used side-by-side in the same repository

Jujutsu and Git repositories exist in the same directory, so you can use `jj`
and `git` side-by-side (this is called [colocation]). If you find a situation
that's easier with Git, run the `git` command and return to `jj` when you're
done (and please [make a feature request] if there isn't one yet!).

Colocation makes migration easier because you can adopt Jujutsu for the
workflows it improves without losing access to the Git commands and tools you
already know.

[colocation]: glossary.md#colocated-repos
[make a feature request]: https://github.com/jj-vcs/jj/issues

## The Git index/staging area

Jujutsu does not have an index/staging area as Git does. Because rewriting
commits is quick and easy, it's natural to use commits as a replacement for the
index.

Instead of separate commands just for interacting with the index (`git add`,
`git rm --cached`), the commands `jj split` and `jj squash` can be used to move
work-in-progress as easily as moving finished work.

```sh
# Split the working copy commit into two sequential commits, putting file1 and
# file2 In the first commit
jj split file1 file2

# or interactively select which changes to split
jj split

# Move the changes in file3 into the parent commit
jj squash file3
# or, interactively:
jj squash -i
```

## Automatic and safer history editing

If you frequently amend, reorder, or squash commits, Jujutsu can often perform
the same operations in fewer commands.

Suppose you want to amend an older commit. With Git you might do this in three
steps:

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
