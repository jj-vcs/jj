# Comparison with Git

## Introduction

This document attempts to describe how Jujutsu is different from Git. See
[the Git-compatibility doc](git-compatibility.md) for information about how
the `jj` command interoperates with Git repos. See
[the Git command table](git-command-table.md) for a table of similar commands.

## Overview

Here is a list of conceptual differences between Jujutsu and Git, along with
links to more details where applicable and available. There's a
[table](git-command-table.md) explaining how to achieve various use cases.

* **The working copy is automatically committed.** That results in a simpler and
  more consistent CLI because the working copy is now treated like any other
  commit. [Details](working-copy.md).
* **There's no index (staging area).** Because the working copy is automatically
  committed, an index-like concept doesn't make sense. The index is very similar
  to an intermediate commit between `HEAD` and the working copy, so workflows
  that depend on it can be modeled using proper commits instead. Jujutsu has
  excellent support for moving changes between commits. [Details](#the-index).
* **No need for branch names (but they are supported as
  [bookmarks](glossary.md#bookmark)).** Git lets you check out a commit without
  attaching a branch to it. It calls this state "detached HEAD". This is the
  normal state in Jujutsu (there's actually no way -- yet, at least -- to have
  an active branch/bookmark). However, Jujutsu keeps track of all visible heads
  (leaves) of the commit graph, so the commits won't get lost or
  garbage-collected.
* **No current branch.** Git lets you check out a branch, making it the 'current
  branch', and new commits will automatically update the branch. This is
  necessary in Git because Git might otherwise lose track of the new commits.

    Jujutsu does not have a corresponding concept of a 'current bookmark';
  instead, you update bookmarks manually. For example, if you start work on top
  of a commit with a bookmark, new commits are created on top of the bookmark,
  then you issue a later command to update the bookmark.
* **Conflicts can be committed.** No commands fail because of merge conflicts.
  The conflicts are instead recorded in commits and you can resolve them later.
  [Details](conflicts.md). This lets us rebase conflict and conflict
  resolutions, and thereby addressing most `git rerere` use cases.
* **Descendant commits are automatically rebased.** Whenever you rewrite a
  commit (e.g. by running `jj rebase`), all its descendants commits will
  automatically be rebased on top. Branches pointing to it will also get
  updated, and so will the working copy if it points to any of the rebased
  commits.
* **No "evil merges".** In Git, a merge commit that contains changes that are
  not in any parent are called [evil merges]. We believe the reason for this
  opinionated term is that Git has historically not been very good at working
  with such commits. For example, `git show` doesn't show them well (without
  `--remerge-diff` from 2022), and `git rebase` drops the changes (without
  `--rebase-merges` from 2018). Jujutsu defines the changes in a commit as
  being relative to the auto-merged parents in all contexts, so you can safely
  include change in merge commits.
* **Bookmarks/branches are identified by their names (across remotes).** For
  example, if you pull from a remote that has a `main` branch, you'll get a
  bookmark by that name in your local repo. If you then move it and push back to
  the remote, the `main` branch on the remote will be updated.
  [Details](bookmarks.md).
* **The operation log replaces reflogs.** The operation log is similar to
  reflogs, but is much more powerful. It keeps track of atomic updates to all
  refs at once (Jujutsu thus improves on Git's per-ref history much in the same
  way that Subversion improved on RCS's per-file history). The operation log
  powers e.g. the undo functionality. [Details](operation-log.md)
* **There's a single, virtual root commit.** Like Mercurial, Jujutsu has a
  virtual commit (with a hash consisting of only zeros) called the "root commit"
  (called the "null revision" in Mercurial). This commit is a common ancestor of
  all commits. That removes the awkward state Git calls the "unborn branch"
  state (which is the state a newly initialized Git repo is in), and related
  command-line flags (e.g. `git rebase --root`, `git checkout --orphan`).

## The index

Git's ["index"](https://git-scm.com/book/en/v2/Git-Tools-Reset-Demystified) has
multiple roles. One role is as a cache of file system information. Jujutsu has
something similar. Unfortunately, Git exposes the index to the user, which makes
the CLI unnecessarily complicated (learning what the different flavors of
`git reset` do, especially when combined with commits and/or paths, usually
takes a while). Jujutsu, like Mercurial, doesn't make that mistake.

As a Git power-user, you may think that you need the power of the index to
commit only part of the working copy. However, Jujutsu provides commands for
more directly achieving most use cases you're used to using Git's index for. For
example, to create a commit from part of the changes in the working copy, you
might be used to using `git add -p; git commit`. With Jujutsu, you'd instead
use `jj split` to split the working-copy commit into two commits. To add more
changes into the parent commit, which you might normally use
`git add -p; git commit --amend` for, you can instead use `jj squash -i` to
choose which changes to move into the parent commit, or `jj squash <file>` to
move a specific file.

[evil merges]: https://git-scm.com/docs/gitglossary#Documentation/gitglossary.txt-evilmerge
