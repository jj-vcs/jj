# Explicit submodule gitlink updates

Author: 0WD0

## Summary

This design proposes a jj-native workflow for updating Git submodule gitlinks
without recursively snapshotting the submodule working copy.

The core idea is intentionally simple:

```text
superproject tree entry at <submodule path>
    = TreeValue::GitSubmodule(<submodule commit id>)
```

The selected submodule revision is ordinary superproject content. It is stored in
the superproject tree as `TreeValue::GitSubmodule(CommitId)`, which the Git
backend already reads from and writes to Git tree entries of mode `160000`.

What changes is how that tree entry is updated. The superproject working-copy
snapshot must not inspect the submodule's current checkout and must not update
the gitlink implicitly. Instead, users update the gitlink with an explicit jj
command, such as:

```bash
jj submodule bind modules/jgit -r e5232d0c
```

That command resolves the submodule revision and rewrites the selected
superproject change so the path `modules/jgit` has a new
`TreeValue::GitSubmodule(e5232d0c...)` value.

This keeps submodule work isolated from superproject snapshotting: editing,
rebasing, or checking out commits inside the submodule does not implicitly dirty
the superproject. At the same time, the selected submodule commit remains part of
the normal jj tree/commit model, so diffing, rebasing, splitting, undo,
operation-log restore, Git export, `jj git push`, and `jj gerrit upload` can use
existing content semantics.

This document complements [git-submodules.md](./git-submodules.md) and
[git-submodule-storage.md](./git-submodule-storage.md). Those documents describe
storage and broad Git submodule support. This document focuses on how a mutable
jj superproject change records *which* submodule revision it depends on.

## State of the feature

As of today, jj represents Git submodule entries in trees as
`TreeValue::GitSubmodule(CommitId)`. The Git backend can read and write these
entries as Git tree entries of mode `160000`. However, the local working-copy
snapshot logic deliberately ignores existing submodule directories.

In `lib/src/local_working_copy.rs`, tracked submodule paths are skipped rather
than recursively scanned. Checkout/update logic also avoids treating an existing
submodule directory as a normal tracked file. This avoids destroying or
recursively managing the submodule's working copy, but it also means that
changing the submodule checkout does not update the superproject commit's
`GitSubmodule` tree entry.

A tempting fix would be to make the working-copy snapshot read the submodule's
current `HEAD` and automatically update the superproject tree entry. This design
rejects that default because it would make ordinary work inside the submodule
implicitly mutate the superproject's current change.

## Prior work

### Git submodules

Git stores submodule dependencies as tree entries with mode `160000`, plus
metadata in `.gitmodules`. A submodule has its own repository, index, working
copy, refs, and remote configuration. Updating a submodule checkout can make the
superproject appear dirty because the checked-out commit no longer matches the
superproject's gitlink.

This implicit coupling is one of the main sources of Git submodule UX problems:
users often have to reason about whether a command is operating on the
superproject, the submodule, or both.

This design keeps Git's interoperable storage format but rejects Git's implicit
working-copy-to-gitlink update behavior as jj's default.

### Existing jj submodule design documents

[git-submodules.md](./git-submodules.md) proposes phased Git submodule support:
readonly submodules, then snapshotting new changes, then merge/rebase/conflict
support.

[git-submodule-storage.md](./git-submodule-storage.md) proposes storing
submodules as full jj repositories, with commands coordinating the relationship
through the superproject. It explicitly notes that the submodule repository can
be internally valid while the combined superproject/submodule state is invalid,
and that submodule interactions should go through superproject coordination.

This document is compatible with that direction. The submodule repository
provides commits, refs, and working-copy state. The superproject records the
selected dependency target as an ordinary gitlink tree entry.

### Discussion in issue #2919

Issue [#2919](https://github.com/jj-vcs/jj/issues/2919) discusses native
submodule support and the broader problem with read-write submodules. One point
raised there is that submodules are difficult because they are both references to
other repositories and editable working trees. If edits in the submodule are
implicitly reflected in the superproject, the abstraction leaks.

This proposal avoids that leak by separating:

- work performed in the submodule repository; from
- the superproject's explicit decision to depend on a specific submodule
  revision.

## Goals and non-goals

### Goals

- Avoid recursive snapshotting of submodule contents.
- Avoid automatically dirtying the superproject when the submodule checkout
  changes.
- Make updating a submodule dependency an explicit operation.
- Store the selected submodule commit in the superproject tree as
  `TreeValue::GitSubmodule(CommitId)`.
- Reuse jj's existing tree, commit, transaction, operation-log, diff, and Git
  export semantics.
- Preserve compatibility with Git servers and Gerrit by producing ordinary Git
  commits with `160000` gitlink entries.
- Provide UI that clearly distinguishes the bound gitlink target from the
  submodule working-copy checkout.
- Leave room for jj-native submodule storage, non-Git subrepos, and nested
  submodules in the future.

### Non-goals

- Automatically tracking every change made inside a submodule from the
  superproject.
- Making the superproject working-copy snapshot recurse into submodule working
  copies.
- Treating the Git index as the source of truth for jj submodule state.
- Introducing a separate operation-versioned binding store for selected
  submodule revisions.
- Replacing package managers or build-system dependency resolution.
- Designing all merge/rebase conflict resolution behavior for submodules in this
  document.
- Solving all Git compatibility issues for `.gitmodules` and Git's submodule
  commands in the first iteration.
- Guaranteeing that an old gitlink can be materialized if the selected submodule
  commit has been deleted or garbage-collected. In that case jj should report a
  missing submodule commit.

## Overview

A superproject commit records a submodule dependency directly in its tree:

```text
modules/jgit -> TreeValue::GitSubmodule(e5232d0c4515f00ee5a6191526adbcb2c69a6b09)
```

When exported to Git, this becomes the ordinary Git submodule gitlink:

```text
160000 commit e5232d0c4515f00ee5a6191526adbcb2c69a6b09 modules/jgit
```

If the submodule working copy later checks out another commit, the superproject
tree does not change. The user can inspect the mismatch and explicitly update
the gitlink when desired:

```bash
jj submodule bind modules/jgit -r @
```

Conceptually, this command does the same kind of superproject tree rewrite as
editing a tracked file in the superproject, except the new value is a submodule
commit id rather than file contents.

## Detailed design

### Terminology

- **Superproject**: the repository containing the submodule path.
- **Submodule repository**: the repository checked out at the submodule path.
- **Submodule path**: the path in the superproject where the submodule is
  mounted, such as `modules/jgit`.
- **Gitlink**: the Git tree entry of mode `160000` that stores the selected
  submodule commit id.
- **Gitlink target**: the submodule commit id stored in the superproject tree.
- **Working-copy checkout**: the commit currently checked out in the submodule
  repository's working copy. This is not the source of truth for the
  superproject.
- **Selector**: optional user-facing input such as a submodule revset, bookmark,
  or change id that resolves to a concrete gitlink target when an explicit
  command runs.

### Source of truth

The source of truth for the selected submodule revision is the superproject tree
entry:

```text
TreeValue::GitSubmodule(target_commit_id)
```

The source of truth is not:

- the submodule working-copy checkout;
- the submodule's current `@`;
- the Git index;
- a live Git ref file;
- a mutable bookmark target looked up at export time;
- a submodule change id looked up at export time.

Selectors are useful while running commands, but the command records the resolved
commit id in the tree. For example:

```text
jj submodule bind modules/jgit -r main
```

If `main` resolves to `A`, the superproject tree stores `A`. If `main` later
moves to `B`, the existing superproject revision still records `A` until the
user explicitly binds again.

This distinction avoids implicit updates:

| Action | Effect on superproject gitlink |
| --- | --- |
| Edit files inside the submodule | No effect |
| Rebase the submodule's current `@` | No effect |
| Checkout another submodule commit | No effect |
| Move a submodule bookmark | No effect |
| Run `jj submodule bind/update` | Rewrites the superproject tree entry |
| Export/push/upload the superproject | Uses the stored tree entry |

### Why the gitlink lives in the tree

The selected submodule commit is exported as a Git tree entry. Keeping the same
value in jj's tree has several advantages:

- Historical commits are stable: exporting the same jj commit produces the same
  gitlink target as long as the submodule commit object is available.
- Binding-only updates change the superproject tree id, so normal jj content
  semantics see the update.
- `jj diff`, `jj status`, `jj log`, split/squash/rebase, operation log, undo, and
  `--at-op` do not need a parallel metadata model for selected submodule
  revisions.
- `jj git export`, `jj git push`, and `jj gerrit upload` can use the existing Git
  backend path for writing `TreeValue::GitSubmodule` as mode `160000`.
- Imported Git commits already contain the necessary value in the tree.

The important UX rule is that working-copy snapshotting must not update this
entry implicitly. The tree is the right storage location; automatic snapshotting
of nested repository state is the behavior to avoid.

### User interface

A minimal command surface could be:

```bash
jj submodule bind <path> -r <submodule-rev>
```

This command would:

1. resolve `<path>` to a known submodule repository;
2. resolve `<submodule-rev>` inside that repository to a concrete commit id;
3. find the target superproject revision, defaulting to the current working-copy
   commit;
4. rewrite that superproject revision's tree so `<path>` is
   `TreeValue::GitSubmodule(target_commit_id)`;
5. record an ordinary jj operation describing the gitlink update.

Examples:

```bash
# In the superproject, bind to the submodule working-copy commit:
jj submodule bind modules/jgit -r @

# Bind to a named submodule revision:
jj submodule bind modules/jgit -r main

# Bind a different superproject change explicitly:
jj submodule bind modules/jgit --change qlnuwmow -r e5232d0c

# Show submodule gitlinks and working-copy checkouts:
jj submodule status
```

Possible `jj submodule status` output:

```text
modules/jgit
  gitlink target: e5232d0c Add CommitBuilder support for extra headers
  working copy:   7aa3c9d Work in progress inside JGit
  differs:        run `jj submodule bind modules/jgit -r @` to update gitlink
```

This output deliberately distinguishes the superproject's recorded gitlink from
the submodule working-copy revision.

Possible names for the update command include:

- `jj submodule bind <path> -r <rev>`
- `jj submodule set <path> -r <rev>`
- `jj submodule update-gitlink <path> -r <rev>`

This document uses `bind` as a placeholder, but the command name is open to
bikeshedding.

### Updating the superproject tree

`jj submodule bind` should behave like an explicit content edit to the selected
superproject revision.

For a non-merge working-copy commit, the command conceptually does:

```text
tree = current_superproject_tree
tree[modules/jgit] = TreeValue::GitSubmodule(target_commit_id)
rewrite current commit with tree
```

Implementation details should use the normal mutable-repo transaction and commit
rewrite machinery rather than editing backend objects in place. The result is a
new jj commit id with the same change id, just like amending file contents.

The command should validate:

- `<path>` is a configured or otherwise recognized submodule path;
- the target revision resolves to a concrete commit in the submodule repository;
- the path is not simultaneously a regular file or directory in the target
  superproject tree, unless the command is explicitly replacing it with a
  submodule;
- `.gitmodules` metadata is present or can be updated/created when Git
  compatibility requires it;
- the target submodule commit object is available locally, or the command records
  a clearly marked missing target only if such a workflow is intentionally
  supported.

For a merge commit or conflicted tree, updating a submodule path should either
resolve the path-level conflict to the selected gitlink target or fail with a
message explaining how to resolve the conflict explicitly.

### Working-copy behavior

The superproject working-copy snapshot should continue to avoid recursive
snapshotting of submodule contents. A submodule checkout changing from one commit
to another should not automatically update the superproject tree.

Specifically:

- `jj status` in the superproject should not report a tracked content change just
  because the submodule working copy moved.
- `jj status` may report that the submodule working copy differs from the stored
  gitlink target, but such reporting should be informational unless the user
  explicitly asks to update the gitlink.
- `jj commit` in the superproject should not automatically change submodule
  gitlinks unless requested.
- Direct `jj commit` in the submodule is allowed only to the extent permitted by
  the broader submodule storage design. If supported, it operates on the
  submodule repository only and does not update the superproject gitlink.
- Commands that mutate the relationship between the superproject and submodule
  should be superproject-coordinated commands.

This preserves a clear boundary between repositories while keeping the selected
submodule revision in normal superproject content.

### Checkout behavior

When checking out a tree that contains `TreeValue::GitSubmodule(target)`, jj
should avoid overwriting an existing submodule working copy.

A minimal behavior is:

- create the submodule directory if it does not exist;
- record the path as a Git submodule in the working-copy state;
- do not recursively populate or reset the submodule working copy by default;
- if the submodule working copy exists at a different commit, report the mismatch
  through `jj submodule status` rather than treating it as a superproject file
  modification.

Future commands can add explicit population and checkout behavior, such as:

```bash
jj submodule sync modules/jgit
jj submodule checkout modules/jgit
jj submodule foreach ...
```

Those commands should be explicit because they operate on another repository's
working copy.

### Missing targets and deleted submodule changes

If a superproject tree contains `TreeValue::GitSubmodule(target)` and the target
commit is missing from the submodule repository, jj should report a deterministic
missing submodule target error when an operation needs the target object.

Example:

```text
error: submodule target is missing
  path: modules/jgit
  target: e5232d0c4515f00ee5a6191526adbcb2c69a6b09
  submodule repo: jgit
hint: fetch the submodule repository or bind this submodule to another commit
```

jj should not silently reinterpret the gitlink through a bookmark or change id.
If an old submodule bookmark used to point to `A` and later points to `B`, a
superproject tree that records `A` still means `A`.

For Git export, a missing target object may or may not block writing the
superproject Git commit depending on backend constraints. Even if the superproject
Git tree can technically contain a gitlink to an object not present in the
superproject object database, jj should warn or fail early enough that users do
not accidentally publish a gitlink reviewers cannot fetch.

### Import from Git

When importing Git commits that contain submodule gitlinks, jj should preserve
the Git tree's `TreeValue::GitSubmodule(target)` entry. No separate binding
metadata is required for the selected revision.

A conservative import strategy:

- Preserve the Git tree's `TreeValue::GitSubmodule` entry in the imported jj
  commit.
- If the submodule repository is known and contains the target commit, status and
  UI commands can display commit metadata for the target.
- If the submodule repository is missing or the target commit is unavailable,
  preserve the raw gitlink and report the missing target only when the user
  interacts with that submodule or runs a command that requires the target.

This allows jj to round-trip existing Git submodule commits without requiring all
submodule repositories to be present immediately.

### Git export, push, and Gerrit upload

No special materialization layer is needed for the selected submodule revision.
The selected target is already in the jj tree as `TreeValue::GitSubmodule`, and
the Git backend already writes that value as a Git tree entry of mode `160000`.

This applies to:

- `jj git export` and Git ref export paths;
- `jj git push`;
- `jj gerrit upload`;
- any future command that writes jj commits to Git.

A binding-only update is a normal tree update, so it changes the jj commit id and
Git commit id. Gerrit upload does not need a special tree overlay for submodules;
it only needs to preserve the already-recorded tree entry when performing its
existing transient description/parent rewrites.

### Diff and status presentation

Diffs between two superproject trees should treat gitlink target changes as
submodule pointer changes, not as recursive file diffs.

Example diff presentation:

```text
Modified git submodule modules/jgit:
  e5232d0c Add CommitBuilder support for extra headers
  7aa3c9d Update CommitBuilder API
```

`jj status` can show two different pieces of information:

1. ordinary superproject changes, including gitlink target changes already
   recorded in the current commit; and
2. informational submodule checkout mismatches, where the submodule working copy
   is at a different commit than the recorded gitlink target.

For example:

```text
Working copy changes:
M modules/jgit (gitlink target changed)

Submodule working copies:
modules/jgit: working copy 7aa3c9d differs from gitlink e5232d0c
```

The second section should not imply that `jj commit` will automatically record a
new gitlink target.

### Rebase, split, squash, duplicate, and divergent changes

Because the selected submodule revision lives in the superproject tree, normal jj
rewrite operations can treat it like other path-level content.

Initial behavior:

- Rebase: preserve gitlink changes as part of the rebased tree diff.
- Amend/describe: preserve gitlink targets unless the command explicitly updates
  the tree.
- Explicit `jj submodule bind/update`: rewrite the selected superproject change
  with a new gitlink target.
- Duplicate with a new change id: copy the tree, including gitlink entries, just
  as duplicate copies file contents.
- Split: if the gitlink path is selected into one side of the split, the gitlink
  change moves with that side. If selection is ambiguous, prompt or leave it with
  the selected path-level changes.
- Squash: if source and destination modify the same submodule gitlink to
  incompatible targets, surface a normal path-level conflict for that gitlink.
- Divergent visible commits with the same change id: each divergent commit has
  its own tree and therefore its own gitlink target. No unversioned side table is
  needed to distinguish them.

This document does not fully specify content-aware submodule merge behavior. It
only requires that gitlink target changes be represented as first-class
path-level tree changes.

### Conflicts

Two superproject changes may update the same submodule path to different target
commits. A merge or rebase can therefore produce a gitlink conflict even if the
superproject's regular file tree is otherwise clean.

A future conflict representation could include:

```text
Submodule conflict at modules/jgit:
  side A: e5232d0c Add CommitBuilder support for extra headers
  side B: 7aa3c9d Update CommitBuilder API
```

Possible resolution commands:

```bash
jj submodule bind modules/jgit -r e5232d0c
jj submodule bind modules/jgit -r 7aa3c9d
jj submodule bind modules/jgit -r <merge-of-both>
```

A later design can specify content-aware merge behavior for submodule histories,
such as creating a merge commit inside the submodule and then updating the
superproject gitlink to that merge commit.

### Operation log and transactions

No separate binding state is required for the selected submodule revision. A
submodule gitlink update is a normal superproject tree rewrite, so it naturally
participates in jj operations.

This means:

- `jj undo` restores the previous gitlink target because it restores the previous
  commit/tree state;
- `jj op restore` restores the gitlink state visible at the restored operation;
- `--at-op` reads the tree from that operation;
- concurrent operation merging can use existing commit divergence and path-level
  conflict machinery;
- exported Git commits are reproducible from the jj commit tree as long as the
  submodule target commit is available.

Because submodules are full jj repos with their own operation logs, commands that
also mutate the submodule repository may still create separate submodule
operations. A later improvement could add cross-repository operation correlation
so `jj op log` can show that one user command updated both a submodule and the
superproject gitlink.

### Remote synchronization

Git/Gerrit compatibility uses the materialized gitlink in the exported
superproject commit as the portable source of truth.

The first implementation can keep submodule fetching and pushing explicit. A
superproject push or Gerrit upload should at least detect or warn when the
selected submodule target is not known to be available from an appropriate
submodule remote.

Future synchronization options:

- Fetch missing submodule target commits based on gitlinks.
- Push selected submodule commits before pushing the superproject commit that
  references them.
- Add policy controls that require gitlink targets to be reachable from a
  configured submodule remote.
- Store additional jj-native metadata for submodule repository identity, URL,
  branch policy, or preferred update selector.

## Alternatives considered

### Automatically snapshot submodule HEAD into the superproject

This is the most direct way to make `jj status` and `jj gerrit upload` notice a
submodule pointer update. However, it recreates Git submodule's read-write UX
problem: ordinary work inside the submodule implicitly modifies the
superproject.

This is rejected as the default behavior.

### Read the Git index during `jj gerrit upload`

`jj gerrit upload` could inspect the superproject Git index and use staged
submodule gitlinks when creating transient Git commits.

This is rejected because it makes Gerrit upload depend on Git staging state that
is not represented in jj. It would also make `jj gerrit upload` behave
differently from other Git export paths unless the same special case were added
everywhere.

### Use a mutable bookmark as the source of truth

A pure bookmark-backed design would store:

```text
(superproject change id, path) -> submodule bookmark
```

and resolve the bookmark target at materialization time. This fits jj's mutable
ref model, but it is not stable enough for historical dependencies. Bookmark
movement, bookmark deletion, hidden commits, and divergent commits sharing a
change id can all make an old superproject revision export differently later.

This design allows bookmarks or revsets as command inputs, but the resolved
submodule commit id is stored in the superproject tree.

### Store selected child commits in a separate operation-versioned binding store

A separate binding store could record selected submodule commit ids outside the
superproject tree and make that store participate in jj's operation log.

This can be made correct, but it adds a parallel content model. Git export would
need a materialization layer that overlays binding metadata onto trees, normal jj
commands would need to become binding-aware, and `jj git export` would need to
track the relationship between jj commit ids and transient Git commit ids.

Since Git submodule selection is already represented as a tree entry, this design
prefers using the existing tree model and making updates explicit.

### Use files such as `.gitmodules` or `.jjmodules` as the source of truth

A file-based manifest is easy to inspect and can be committed. However, it makes
submodule dependency updates ordinary file edits, which do not naturally behave
like Git gitlinks and would still need conversion during Git export.

A manifest remains useful for static metadata such as path, URL, branch policy,
and update policy, but the selected revision for a Git-compatible submodule
should be represented as `TreeValue::GitSubmodule(target)`.

## Issues addressed

- [#2919](https://github.com/jj-vcs/jj/issues/2919): Native submodules,
  submodule-specific config.
- [#494](https://github.com/jj-vcs/jj/issues/494): Git submodule support.

## Related work

### Git submodules

Git submodules are the compatibility target for Git export but not the desired
implicit mutable UX model.

### Git subtree and git subrepo

Subtree-style approaches avoid separate working copies but mix subproject
contents into the superproject history. That can be useful, but it is a different
model from a full submodule repository with independent refs and history.

### Josh

Josh provides repository projection and subtree-like workflows with better UX
than raw Git subtree. It is relevant prior art for thinking about explicit
repository relationships and transformations rather than raw Git submodule
mechanics.

### Package managers and build-system dependencies

Package managers often use explicit dependency declarations and lockfiles. This
proposal is not a package manager, but it borrows the idea that dependency
selection should be explicit rather than an accidental consequence of a nested
working-copy checkout.

## Future possibilities

- Content-aware submodule merge/rebase that can create submodule merge commits
  and update gitlinks automatically when appropriate.
- Policy controls, such as requiring that a change which updates a submodule
  gitlink does not modify unrelated superproject paths.
- Non-Git subrepos using explicit tree entries or a different materialization
  backend.
- Nested submodule support with explicit recursion limits.
- Cross-repository operation correlation in `jj op log`.
- UI for showing whether a submodule working copy differs from the recorded
  gitlink target without treating it as a superproject diff.
- Remote synchronization policies for ensuring gitlink targets are fetchable by
  reviewers and collaborators.
- A migration path from existing Git submodule commits to explicit jj submodule
  update commands.

## Open questions

- What should the explicit update command be named: `bind`, `set`, `update`, or
  something more Git-specific such as `update-gitlink`?
- How should `jj submodule bind` discover and validate configured submodule
  paths, especially before `.gitmodules` support is complete?
- Should the command update `.gitmodules` for new submodules, or should adding a
  submodule be a separate command?
- How should status present informational working-copy mismatches without making
  users think the mismatch will be committed automatically?
- How should merge conflicts between different gitlink targets be represented in
  existing tree conflict machinery?
- Should `jj git push` and `jj gerrit upload` warn or fail if a gitlink target is
  not known to be reachable from the configured submodule remote?
- How much submodule checkout/population behavior should be implemented before
  improving `.gitmodules` compatibility?
