# CLI options for specifying revisions

Jujutsu has several CLI options for selecting revisions. They are used
consistently, but it can be difficult to remember when each one is used.

This document explains the difference between each option.

## Summary

These flags are used to specify the sources of the operation:

| Long flag                       | Short flag | Description                                                                    |
| ------------------------------- | ---------- | ------------------------------------------------------------------------------ |
| `--revision` (or `--revisions`) | `-r`       | The default, especially for commands that don't need to specify a destination. |
| `--source`                      | `-s`       | The specified revision and all its descendants.                                |
| `--from`                        | `-f`       | The _contents_ of a revision.                                                  |
| `--branch`                      | `-b`       | A whole branch, relative to the destination.                                   |

These flags are used when commands need both a "source" revision and a
"destination" revision:

| Long flag         | Short flag | Description                                                  |
| ----------------- | ---------- | ------------------------------------------------------------ |
| `--onto`          | `-o`       | Create children of the specified revisions.                  |
| `--insert-after`  | `-A`       | Insert _between_ the specified revisions and their children. |
| `--insert-before` | `-B`       | Insert _between_ the specified revisions and their parents.  |
| `--to`, `--into`  | `-t`       | Which revision to place the selected _contents_.             |

## Manipulating revisions

Most commands accept a revset with `-r`. This selects the revisions in the
revset, and no more. Examples: `jj log -r REV` displays revisions in `REV`, `jj
split -r REV` splits revision `REV` into multiple revisions.

`--source` (`-s`) is used with commands that manipulate revisions _and their
descendants_. `-s REV` is essentially identical to `-r REV::`.

Examples of `-r` and `-s`:

- `jj log -r xyz` displays revision `xyz`.

- `jj fix -s xyz` runs fix tools on files in `xyz` and all of its descendants.
  This command _must_ operate on all of a revision's descendants, so it accepts
  `-s` and not `-r` to communicate this fact.

### Specifying destinations

Commands that move revisions around also need to specify the destinations.

- `--onto REV` (`-o REV`) places revisions as children of `REV`.
- `--insert-after REV` (`-A REV`) inserts revisions as children of `REV` and parents of `REV+`.
- `--insert-before REV` (`-B REV`) inserts revisions as the children of `REV-` and parents of `REV`.

Examples:

- `jj rebase -r REV -o main` rebases revisions in `REV` as children of `main`.
- `jj rebase -r REV -B yyy` inserts revisions `REV` between `yyy` and its parents.
- `jj rebase -r REV -A main -B yyy` inserts revisions `REV` between `main` and `yyy`.
- `jj revert -r xyz -o main` creates a revision that reverts `xyz` then rebases it on top of `main`.

## Manipulating diffs and file contents

Commands that view or manipulate the _contents_ of revisions use `--from` and
`--to` (or `--into`).

Examples:

- `jj diff --from F --to T` compares the files at revision `F` to the files at
  revision `T`.

- `jj restore --from F --to T` copies file contents from `F` to `T`.

- `jj squash --from F --into T` moves the file changes from `F` to `T`.

!!! info

    Commands that accept `--into` also accept `--to`. You can always use `--to`
    if you're not sure which to use.

    They both exist because "into" makes some commands read more clearly in
    English. For example, `jj squash --from X --into Y`.

### Special cases that use `-r`

Some commands manipulate revision contents but allow for `-r`. This means
"compared with its parent". For example, `jj diff -r R` means "compare revision
`R` to its parent `R-`".

### Special cases that don't use any option

Most commands accept revisions as options and paths as positional parameters.
For example, the command to display the diff of a specific file in a specific
revision is:

```command
$ jj diff -r REV file.txt
```

However, some commands cannot accept paths, so they allow omitting the `-r`
flag. For example, the canonical command would be `jj new -r xyz`, but this
command is so common that Jujutsu allows `jj new xyz`.

The commands that allow omitting the `-r` are:

- `jj abandon`
- `jj describe`
- `jj duplicate`
- `jj metaedit`
- `jj new`
- `jj parallelize`
- `jj show`

## Other special cases

`jj git push --change REV` (`-c REV`) means (a) create a new bookmark with a
generated name, and (b) immediately push it to the remote.

`jj restore --changes-in REV` (`-c REV`) means, "remove any changes to the given
files in `REV`". This doesn't use `-r` because `jj restore -r REV` might seem
like it would restore files _from_ `REV` into the working copy.

`jj rebase --branch REV` (`-b REV`) rebases a topological branch of revisions
with respect to some base. This is a convenience for a very common operation.
These commands are equivalent:

- `jj rebase -o main -b @`
- `jj rebase -o main -r (main..@)::`
- `jj rebase -o main -s roots(main..@)`
- `jj rebase -o main` (this is so common that `-b @` is the default "source" of
  a rebase if unspecified)
