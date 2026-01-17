# Colocated Workspaces

Author: [sjawhar](https://github.com/sjawhar) (sami@metr.org)

## Summary

This document specifies the corner cases in colocated workspace detection and
command behaviors. It addresses the documentation TODOs from issue #8052
("Colocated workspaces: Support Git worktrees and multiple colocated jj
workspaces").

Colocated workspaces allow jj and Git to share the same working directory,
automatically synchronizing refs on every command. With the introduction of
multiple jj workspaces backed by a Git repo that uses Git worktrees, several
edge cases need clear specification.

## Goals and non-goals

**Goals:**
- Document all corner cases in workspace colocation detection
- Specify expected behavior for each detection scenario
- Clarify the difference between jj workspaces and Git worktrees
- Document command behaviors in colocated scenarios
- Document known architectural limitations

**Non-goals:**
- Implement per-workspace `git_head` tracking (future work)
- Full investigation of Git worktree locks, orphans, and submodules

## Overview

### What is a Colocated Workspace?

A colocated workspace is where jj and Git share the same working directory. The
`.git` directory (or file, for worktrees) exists alongside the `.jj` directory,
and both tools operate on the same files. jj automatically imports Git refs and
exports jj bookmarks on every command.

Colocation is detected by comparing the Git repository that jj uses internally
with any Git repository at the workspace root. If they share the same
`common_dir` (the shared `.git` directory for worktrees), the workspace is
considered colocated.

### jj Workspaces vs Git Worktrees

While jj workspaces and Git worktrees both provide multiple working directories
backed by a single repository, they have fundamental design differences:

| Aspect | jj Workspaces | Git Worktrees |
|--------|---------------|---------------|
| Branch checkout | Multiple workspaces can edit the same change | Each worktree must be on a different branch |
| What `git status` shows | May not reflect jj's state until export | Shows worktree's actual state |
| Switching context | Use `jj new` / `jj edit` | Use `git checkout` / `git switch` |
| Creation | `jj workspace add` | `git worktree add` |
| Locking | No built-in locking | Supports `git worktree lock` |
| Repair | N/A | `git worktree repair` |

In a colocated setup with multiple workspaces, both systems coexist: each jj
workspace corresponds to a Git worktree, sharing the same underlying Git
repository.

## Detailed Design

### Detection Algorithm

The `MaybeColocatedGitRepo` struct in `lib/src/git_backend.rs` handles
colocation detection. The algorithm works as follows:

1. Open the Git repository that jj's store points to
2. If a workspace root is provided, attempt to open `<workspace_root>/.git`
3. If both succeed, compare their `common_dir()` paths (canonicalized)
4. If the paths match, the workspace is colocated

### Workspace Detection Corner Cases

The following table specifies all known detection scenarios and their expected
behaviors:

#### Currently Handled

| Scenario | Detection Method | Expected Behavior | Test Coverage |
|----------|------------------|-------------------|---------------|
| `.git` directory | `gix::open()` + common_dir match | Colocated if same repo | `test_git_colocated()` |
| `.git` file (worktree) | `gix::open()` + common_dir match | Colocated if points to jj's git | `test_colocated_workspace_in_bare_repo()` |
| `.git` symlink | `symlink_metadata()` check | Warn if points elsewhere | `test_colocated_workspace_git_symlink_to_wrong_repo()` |
| Broken worktree | `gix::open()` fails | Warn + suggest `git worktree repair` | `test_colocated_workspace_invalid_gitdir()` |
| Wrong gitdir | common_dir mismatch | Warn "not managed by jj" | `test_colocated_workspace_wrong_gitdir()` |
| Bare repo backing | N/A | Works, each worktree independent | `test_colocated_workspace_in_bare_repo()` |
| Canonicalization failure | `canonicalize()` returns Err | Assume not colocated (debug log) | Implicit |
| Moved original repo | Worktree links break | Warn broken worktree, suggest repair | `test_colocated_workspace_moved_original_on_disk()` |

#### Warning Scenarios

When jj detects a `.git` presence but isn't colocated, it warns the user. The
`warn_about_unexpected_git_in_workspace` function in `cli/src/git_util.rs`
handles these cases:

| `.git` Type | Warning Message | Hint |
|-------------|-----------------|------|
| Symlink to different repo | "Workspace has a .git symlink ... that isn't pointing to jj's git repo" | `rm .git` |
| Directory not managed by jj | "Workspace has a .git directory that is not managed by jj" | `rm -rf .git` |
| Git worktree to different repo | "Workspace is also a Git worktree that is not managed by jj" | `git worktree remove .` from parent repo |
| Broken worktree | "Workspace is a broken Git worktree" | `git worktree repair` |

#### Known Gaps (needing investigation)

| Scenario | Current Behavior | Notes | Priority |
|----------|------------------|-------|----------|
| TOCTOU race condition | Detection is non-atomic | Concurrent `.git` changes during detection may cause incorrect state; detection is best-effort | Medium |
| Network filesystems | Canonicalization may be slow | Detection runs on every command; `canonicalize()` performs N stat calls where N = path depth | Medium |
| Symlink chains | Only immediate target checked | May need to follow full chain | Low |
| Cross-filesystem symlinks | May fail with relative paths | Detect and warn? | Low |
| Submodules in worktrees | Unknown | Needs investigation | Low |
| Locked worktrees | Unknown | Git supports `git worktree lock` | Low |
| Orphan worktrees (unborn branch) | Unknown | Worktree on unborn branch | Low |

### Command Behavior Corner Cases

#### `jj workspace add`

| Scenario | Current Behavior | Notes |
|----------|------------------|-------|
| In colocated repo | Creates jj workspace only | Use `--colocate` to also create a Git worktree |
| In colocated repo with `--colocate` | Creates jj workspace + Git worktree | New workspace is also colocated |
| In non-colocated repo | Creates jj workspace | As expected |
| Path with slashes | Allowed | Slashes are namespace separators in workspace names |

#### `jj git init --colocate`

| Scenario | Current Behavior | Notes |
|----------|------------------|-------|
| Inside Git worktree | Error: "Cannot create colocated jj repo inside Git worktree" | Prevents nested worktree confusion |
| In directory with unrelated `.git` | Warning shown, proceeds non-colocated | User should remove unrelated `.git` first |
| With `--git-repo` pointing to worktree | Imports from worktree | Works as expected |

#### `jj git colocation enable/disable`

| Scenario | Current Behavior | Notes |
|----------|------------------|-------|
| In main workspace | Works | Toggles colocation for the repo |
| In secondary workspace | Error: "not the main workspace" | Only main workspace can change colocation |
| With existing `.git` directory | Error on enable | User must remove existing `.git` first |

#### `jj workspace update-stale`

| Scenario | Current Behavior | Notes |
|----------|------------------|-------|
| Colocated workspace moved | May fail to detect | Should detect via common_dir |
| Git worktree pruned externally | Broken state | Should warn, offer recovery |

#### `jj workspace forget`

| Scenario | Current Behavior | Notes |
|----------|------------------|-------|
| Colocated workspace | Removes jj workspace + Git worktree | Automatic cleanup |
| Colocated workspace with uncommitted changes | Warns and aborts | Use `--force` to override |
| Non-colocated workspace | Removes jj workspace only | Directory left on disk for manual cleanup |
| Missing Git worktree | Removes jj workspace, warns about missing worktree | Graceful handling |

### Known Architectural Limitations

#### Single git_head in View

**Location:** `lib/src/op_store.rs`, `View::git_head` field

**Issue:** The `View` struct stores a single `git_head`, but when multiple
colocated workspaces exist (each with its own Git worktree), each worktree has
an independent HEAD.

**Impact:**
- `git_head()` template shows the wrong value in non-default workspaces
- The `git_head` in the view reflects whichever workspace last exported

**Workaround:** The implementation now writes to each worktree's HEAD file
independently during export, but the view's `git_head` field doesn't track
per-workspace state. If you need accurate HEAD information in a secondary
workspace, use `git -C <path> rev-parse HEAD` instead of the `git_head()`
template.

**Future Fix:** Store per-workspace git_head in view, keyed by workspace name.

#### Export Only Checks Main HEAD

**Issue:** During git export, only the main worktree's HEAD is checked for
changes made outside jj.

**Impact:** Changes made via `git` commands in secondary colocated workspaces
may not be detected during import.

**Future Fix:** Check all worktrees' HEAD files during import/export.

## Issues addressed

- [#8052](https://github.com/jj-vcs/jj/issues/8052) - Colocated workspaces:
  Support Git worktrees and multiple colocated jj workspaces (documentation
  items)

## Related Work

- Git worktrees: https://git-scm.com/docs/git-worktree
- jj workspaces: See `jj workspace --help` and the [working copy documentation](../working-copy.md#workspaces)

## Future Possibilities

### Per-workspace git_head Tracking

Store `git_head` per workspace in the view, allowing accurate HEAD tracking
across multiple colocated workspaces.

### Additional Corner Cases (Need Investigation)

As noted by @jyn514 in issue #8052, the following Git worktree features may
have interactions that need investigation:

- **Locked worktrees:** Git allows locking worktrees to prevent pruning
- **Orphan worktrees:** Worktrees on unborn branches
- **Submodules:** Behavior of submodules within worktrees
- **Worktree repair:** Integration with `git worktree repair` for broken states
