# Using Jujutsu with Gerrit Code Review

JJ and Gerrit share the same mental model, which makes Gerrit feel like a
natural collaboration tool for JJ. JJ tracks a "change identity" across
rewrites, and Gerrit's `Change-Id` tracks the same logical change across patch
sets. JJ and Gerrit's `Change-Id`s aren't natively compatible yet, but they're
philosophically aligned. `jj gerrit upload` bridges the gap today by adding a
Gerrit-style `Change-Id` trailer while JJ keeps its own notion of change
identity on the client. In practice, that means small, clean commits that evolve
over time, exactly how Gerrit wants you to work.

This guide assumes a basic understanding of Git, Gerrit, and Jujutsu.

## Set up a Gerrit remote

Jujutsu communicates with Gerrit by pushing commits to a Git remote. If you're
starting from an existing Git repository with Gerrit remotes already configured,
you can use `jj git init` to start using JJ in that repo. Otherwise, set up your
Gerrit remote:

```shell
# Option 1: Start JJ in an existing Git repo with Gerrit remotes
$ jj git init

# Option 2: Add a Gerrit remote to a JJ repo
$ jj git remote add gerrit https://review.gerrithub.io/yourname/yourproject

# Option 3: Clone the repo via jj
$ jj git clone https://review.gerrithub.io/your/project
```

If you used option 2, you can configure default values in your repository config
by appending the following lines to your config file, like so (to do this for
a specific repo, run `jj config edit --repo`):

```toml
[gerrit]
default-remote = "gerrit"       # name of the Git remote to push to
default-remote-branch = "main"  # target branch in Gerrit
```

## Basic workflow

`jj gerrit upload` takes one or more revsets and uploads the stack of commits
and their ancestors to Gerrit. Each JJ change will map to a single Gerrit change
by generating a `Change-Id` based on the JJ change ID (or using the existing
`Change-Id` trailer if already present). This should be what you want most of
the time, but if you want to associate a JJ change with a specific change
already uploaded to Gerrit, you can copy the `Change-Id` trailer from Gerrit to
the bottom of the commit description in JJ.

> Note: Gerrit identifies and updates changes by the `Change-Id` trailer. When
> you re-upload a commit with the same `Change-Id`, Gerrit creates a new patch
> set.

### Upload a single change

```shell
# Uploads `@` if it has a description, otherwise uploads `@-`.
$ jj gerrit upload

# Or explicitly specify a revision to upload.
$ jj gerrit upload -r @-
```

## Selecting revisions (revsets)

`jj gerrit upload` accepts one or more `-r`/`--revisions` arguments. Each
argument may expand to multiple commits. Common patterns:

- `-r @-`: the commit previous to the one you're currently working on
- `-r A..B`: commits that are ancestors of B but not of A

See the [revsets](revsets.md) guide for more information.

### Preview without pushing

Use `--dry-run` to see which commits would be modified and pushed, and where,
without changing anything or contacting the remote.

```shell
$ jj gerrit upload -r '@-' --remote-branch main --dry-run
```

## Target branch and remote selection

There are a few way of specifying the target remote for your projects:

- Run `jj config set --user gerrit.default-remote <remote name>` to set your
  default remote across all repos.
- Run `jj config set --repo gerrit.default-remote <remote name>` to set your
  default remote for this specific repo.
- Use `--remote <remote name>` to use a specific remote in one invocation of
  `jj gerrit upload`.
- Otherwise, a remote named `gerrit` will be used. If that doesn't exist, the
  command will fail, and you will need to use one of the above methods to
  specify the remote to upload to.

Additionally, you can specify the target remote branch:

- Run `jj config set --user gerrit.default-remote-branch <branch name>` to set
  your default branch across all repos.
- Run `jj config set --repo gerrit.default-remote-branch <branch name>` to set
  your default branch for this specific repo.
- Use `--remote-branch <branch name>` to use a specific branch in one invocation
  of `jj gerrit upload`.

## Updating changes after review

To address review feedback, update your revisions, then run `jj gerrit
upload` again with the same revsets. Gerrit will add new patch sets to the
existing changes instead of creating new ones.

Examples:

```shell
# Edit an earlier commit in the stack
$ jj edit xyz  # position on the stack to edit
 --- Apply needed edits ---
$ jj gerrit upload -r xyz
```

## `Change-Id` management

If you do not provide an explicit `Change-Id` trailer in your commits,
`jj gerrit upload` will generate a transient one for you based on your JJ
change ID. This means that as long as the JJ change ID remains the same (and
you don't add an explicit `Change-Id` trailer), it will upload as a new patch
set on the existing change.

Keep this association in mind when splitting or squashing changes. For example,
when splitting a change, the portion that you want associated with the
original Gerrit change should remain in the original JJ change (the first half
of the split). Similarly, when squashing new changes, you typically want to
squash into the change that was previously uploaded to Gerrit.

If your JJ changes no longer align with the desired mapping to Gerrit changes,
you can manually copy a Gerrit `Change-Id` trailer into your JJ change
description to directly assign a JJ change to an exist Gerrit change.

As an alternative to `jj gerrit upload`'s automatic `Change-Id` mapping, you
can configure JJ to automatically add `Change-Id` trailer to all change
descriptions:

```toml
[templates]
commit_trailers = '''
if(
  !trailers.contains_key("Change-Id"),
  format_gerrit_change_id_trailer(self)
)
'''
```

In this case, the Gerrit change mapping is defined entirely by the `Change-Id`
trailer. When splitting or squashing changes, be sure to keep the `Change-Id`
trailer associated with the desired changes. Be sure not to duplicate the same
`Change-Id` across different changes. Gerrit will reject pushes that contain
duplicate `Change-Id`s, but if the uploads are done separately, you may
unintentionally overwrite an existing change.

Note that when `jj gerrit upload` automatically adds a `Change-Id` trailer
before pushing to Gerrit, this addition to the commit description will only be
reflected on the uploaded commit, not your local commit. This means the server
will have a different commit ID than you do. As a result, you may encounter
divergence when you fetch a merged change into your local repo. To address this,
you can abandon your local change or rebase it on top of trunk with
`jj rebase --skip-emptied ...`, which will resolve the divergence. Alternatively,
add the above config so that the `Change-Id` trailer is automatically added to
your local commits before `jj gerrit upload` does.

## Alternative `Link` trailer

Since version 3.3.1 Gerrit supports an alternative to the `Change-Id` trailer,
using a `Link` trailer in the format of `<review-url>/id/I<change-id>`. It is
only documented in the [commit-msg hook documentation]. Jujutsu's
`jj gerrit upload` will do the same if you set
`jj config set --repo gerrit.review-url <review-url>`.

[commit-msg hook documentation]: https://gerrit-documentation.storage.googleapis.com/Documentation/3.3.1/cmd-hook-commit-msg.html
