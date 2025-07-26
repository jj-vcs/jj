# Handling divergent changes

## What are divergent changes?

A [divergent change][glossary_divergent_change] occurs when multiple visible
commits have the same change ID.

Normally, when commits are rewritten, the original version (the "predecessor")
becomes hidden and the new commit (the "successor") is visible. Thus, only one
commit with a given change ID is visible at a time.

But, a hidden commit can become visible again. This can happen if:

- A visible descendant is added. For example, `jj new REV` will make `REV`
  visible even if it was hidden before.

- It is made the working copy. `jj edit REV` will make `REV` visible if it
  wasn't already.

Divergent changes also occur if two different users or processes amend the same
change, creating two visible successors. This can happen when:

- Multiple people edit the same change simultaneously in different repositories.

- You perform operations on the same change from different workspaces of the
  same repository.

[glossary_divergent_change]: ../glossary.md#divergent-change

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
# First, identify the divergent commits
jj log -r 'divergent()'

# Abandon the unwanted commit using its commit ID
jj abandon <unwanted-commit-id>
```

This is the simplest solution when you know which version to keep.

### Strategy 2: Duplicate and abandon

If you want to keep both versions as separate changes with different change IDs,
you can duplicate one of the commits to generate a new change ID, then abandon
the original:

```shell
# Duplicate one of the commits to create a new change ID
jj duplicate <commit-id>

# Abandon the original commit
jj abandon <commit-id>
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
