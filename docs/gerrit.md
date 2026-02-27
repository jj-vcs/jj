# Using Jujutsu with Gerrit Code Review

JJ and Gerrit share the same mental model, which makes Gerrit feel like a
natural collaboration tool for JJ. JJ tracks a "change identity" across
rewrites, and Gerrit’s `Change-Id` tracks the same logical change across patch
sets. JJ and Gerrit's `Change-Id`s aren’t natively compatible yet, but they’re
philosophically aligned. `jj gerrit upload` bridges the gap today by adding a
Gerrit-style `Change-Id` while JJ keeps its own notion of change identity on the
client. In practice, that means small, clean commits that evolve over
time, exactly how Gerrit wants you to work.

This guide assumes a basic understanding of Git, Gerrit, and Jujutsu.

## Set up a Gerrit remote

Jujutsu communicates with Gerrit by pushing commits to a Git remote. If you're
starting from an existing Git repository with Gerrit remotes already configured,
you can use `jj git init --colocate` to start using JJ in that repo. Otherwise,
set up your Gerrit remote.

```shell
# Option 1: Start JJ in an existing Git repo with Gerrit remotes
$ jj git init --colocate

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

`jj gerrit upload` takes one or more revsets, and uploads the stack of commits
ending in them to Gerrit.
* If a change has no `Change-Id`, jj will create a new change in gerrit and
  create a bookmark `gerrit-$CHANGE_ID` attached to the change.
* If a change has an existing `Change-Id`, jj will create a new patchset for
  the existing change on gerrit.

This means that:
* You can reassociate a gerrit CL with a different change by simply running
  `jj bookmark move`. This may be useful after running `jj split`, for example.
* A change can gain multiple `Change-Id`s if you squash an uploaded
  change. If this happens, jj will refuse to upload, and you will need to run
  `jj bookmark forget` to remove one of them.

> Note: `Change-Id`s can also be set directly via the `Change-ID` and `Link` git
> footers. This is not recommended, however, as multiple commits can easily end
> up with the same `Change-Id` after a command such as `jj split`

### Upload a single change

```shell
# Upload @ if it has a description, otherwise uploads @-
$ jj gerrit upload

# Or explicitly specify a revision to upload.
$ jj gerrit upload -r @-
```

## Selecting revisions (revsets)

`jj gerrit upload` accepts one or more `-r/--revisions` arguments. Each argument
may expand to multiple commits. Common patterns:

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

- Please run `jj config set --user gerrit.default-remote-branch <branch name>` to set your
  default branch across all repos
- Please run `jj config set --repo gerrit.default-remote-branch <branch name>` to set your
  default branch for this specific repo.
- Use `--remote-branch <branch name>` to override this for one specific occasion.

The remote used to push is determined as follows:

- If you have more than one origin, or the origin isn't called gerrit, run
  `jj config set --repo gerrit.default_remote <gerrit remote name>` to set-up a
  default remote.
- To upload to a specific remote as a one-off thing, use `--remote <remote name>`

## Updating changes after review

To address review feedback, update your revisions, then run `jj gerrit
upload` again with the same revsets. Gerrit will add new patch sets to the
existing changes instead of creating new ones.

Examples:

```shell
# Edit an earlier commit in the stack
$ jj edit xcv  # position on the stack to edit
 --- Apply needed edits ---
$ jj gerrit upload -r xcv
```
