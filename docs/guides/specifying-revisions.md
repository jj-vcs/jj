# Specifying revisions

Jujutsu has several CLI options for selecting revisions. They are used
consistently, but it can be difficult to remember when each one is used.

This document explains the difference between each option.

## Summary

These flags are used to specify the sources of the operation:

| Long flag                       | Short flag | Description                                                                    |
| ------------------------------- | ---------- | ------------------------------------------------------------------------------ |
| `--revision` (or `--revisions`) | `-r`       | The default, especially for commands that don't need to specify a destination. |
| `--source`                      | `-s`       | The specified revision and all its descendants.                                |
| `--from`                        | `-f`       | The _contents_ of a commit                                                     |
| `--branch`                      | `-b`       | A whole branch, relative to the destination.                                   |

These flags are used when commands need both a "source" revision and a
"destination" revision:

| Long flag         | Short flag | Description                                                          |
| ----------------- | ---------- | -------------------------------------------------------------------- |
| `--destination`   | `-d`       | Commits become descendants of the destination.                       |
| `--insert-after`  | `-A`       | Insert commits _between_ the specified revisions and their children. |
| `--insert-before` | `-B`       | Insert commits _between_ the specified revisions and their parents.  |
| `--to`, `--into`  | `-t`       | Which commit to place the selected _contents_.                       |

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

Commands that move commits around also need to specify the destinations.

- `--destination REV` (`-d REV`) places commits as children of `REV`.
- `--insert-after REV` (`-A REV`) inserts commits as children of `REV` and parents of `REV+`.
- `--insert-before REV` (`-B REV`) inserts commits as the children of `REV-` and parents of `REV`.

Examples:

- `jj rebase -r REV -d main` rebases revisions in `REV` as children of `main`.
- `jj rebase -r REV -B B` inserts revisions `REV` between `B` and its parents.
- `jj rebase -r REV -A main -B B` inserts revisions `REV` between `main` and `B`.
- `jj revert -r xyz -d main` creates the commit that reverts `xyz` then rebases it on top of `main`.

## Manipulating diffs and snapshots

Commands that view or manipulate the _contents_ of commits use `--from` and `--to`.

- `--from` (`-f`) specifies the revision that provides the contents (the "from"
  snapshot).

- `--to` or `--into` (`-t`) specifies which revisions the contents will be moved
  or copied to.

Examples:

- `jj diff --from F --to T` compares the files at revision `F` to the files at
  revision `T`.

- `jj restore --from F --to T` copies file contents from `F` to `T`.

- `jj squash --from F --into T` moves the file changes from `F` to `T`.

!!! info

    `--into` and `--to` are synonyms. Commands that accept one also accept the
    other. They both exist because it makes commands read more clearly in
    English.

### Special cases that use `-r`

Some commands manipulate revision contents but allow for `-r`. This means
"compared with its parent". For example, `jj diff -r R` means "compare revision
`R` to its parent `R-`".

## Other special cases

`jj git push --change REV` (`-c REV`) means (a) create a new bookmark with a
generated name, and (b) immediately push it to the remote.

`jj restore --changes-in REV` (`-c REV`) means, "remove any changes to the given
files in `REV`". This doesn't use `-r` because `jj restore -r REV` might seem
like it would restore files _from_ `REV` into the working copy.

`jj rebase --branch REV` (`-b REV`) rebases a topological branch of revisions
with respect to some base. This is a convenience for a very common operation.
These commands are equivalent:

- `jj rebase -d main -b @`
- `jj rebase -d main -r (main..@)::`
- `jj rebase -d main -s roots(main..@)`
- `jj rebase -d main` (this is so common that `-b @` is the default "source" of
  a rebase if unspecified)
