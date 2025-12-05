# Handling divergent changes

## What are divergent changes?

A [divergent change] occurs when multiple [visible commits] have the same change
ID.

These changes are displayed with a `??` after their change ID:

```shell
$ jj log
@  mzvwutvl?? test.user@example.com 2001-02-03 08:05:12 29d07a2d
│  a divergent change
```

Normally, when commits are rewritten, the original version (the "predecessor")
becomes hidden and the new commit (the "successor") is visible. Thus, only one
commit with a given change ID is visible at a time.

But, a hidden commit can become visible again. This can happen if:

- A visible descendant is added locally. For example, `jj new REV` will make
  `REV` visible even if it was hidden before.

- A visible descendant is fetched from a remote. If the hidden commit was pushed
  to a remote, others may base new commits off of them. When their new commits are
  fetched, their visibility makes the hidden commit visible again.

- It is made the working copy. `jj edit REV` will make `REV` and all its
  ancestors visible if it wasn't already.

- Some other operations make hidden commits visible. For example, adding a
  bookmark to a hidden commit makes it visible with the assumption that you are
  now working with that commit again.

Divergent changes also occur if two different users or processes amend the same
change, creating two visible successors. This can happen when:

- Another author modifies commits in a branch that you have also modified
  locally.

- You perform operations on the same change from different workspaces of the
  same repository.

- Two programs modify the repository at the same time. For example, you run
  `jj describe` and, while writing your commit description, an IDE integration
  fetches and rebases the branch you're working on.

[divergent change]: ../glossary.md#divergent-change
[visible commits]: ../glossary.md#visible-commits

## How do I resolve divergent changes?

When you encounter divergent changes, you have several strategies to choose
from. The best approach depends on whether you want to keep the content from one
commit, both commits, or merge them together.

Note that revsets must refer to the divergent commit using its commit ID since
the change ID is ambiguous.

### Strategy 1: Abandon one of the commits

If one of the divergent commits is clearly obsolete or incorrect, simply abandon
it:

```shell
# Abandon the unwanted commit using its commit ID
jj abandon <unwanted-commit-id>

# You can abandon several at once with:
# jj abandon abc def 123
# jj abandon abc::
```

This is the simplest solution when you know which version to keep.

### Strategy 2: Generate a new change ID

If you want to keep both versions as separate changes with different change IDs,
you can generate a new change ID for one of the commits:

```shell
jj metaedit --update-change-id <commit-id>
```

This preserves both versions of the content while resolving the divergence.

### Strategy 3: Squash the commits together

When you want to combine the content from both divergent commits:

```shell
# Squash one commit into the other
jj squash --from <source-commit-id> --into <target-commit-id>
```

This combines the changes from both commits into a single commit. The source
commit will be abandoned.

### Strategy 4: Ignore the divergence

Divergence isn't an error. If the divergence doesn't cause immediate problems,
you can leave it as-is. If both commits are part of immutable history, this may
be your only option.

However, it can be inconvenient since you cannot refer to divergent changes
unambiguously using their change ID.
