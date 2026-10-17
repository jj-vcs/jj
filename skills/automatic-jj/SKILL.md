---
name: automatic-jj
description: Use when editing files or finishing tasks in directories that may be managed by Jujutsu (jj), when jj should replace .bak backups, when an existing Git repo may need jj colocation, or when the user wants opt-in per-change jj checkpoints.
---

# automatic-jj

## Overview

This skill makes Jujutsu (`jj`) the default safety layer for file edits inside a healthy jj workspace.

Inside a healthy jj workspace, do not create `.bak` files for normal edits. Use jj history and jj recovery instead. Only fall back to `.bak` when jj is unavailable, inactive, broken, or initialization was declined.

Default finalization behavior is one jj task-finalization checkpoint when the task is complete. Per-change checkpointing is available only when the user explicitly turns it on.

## When to Use

Use when:
- editing or creating files in a directory that may already use jj
- working in an existing Git repo that may need `jj git init --colocate`
- replacing `.bak`-style safety copies with jj-backed history
- defining when to finalize work in jj
- the user explicitly wants per-change jj checkpoints

Do not use when:
- the task does not modify files
- the directory is outside the scope of the current task
- the environment has no `jj` binary and the user does not want to initialize it

## Core Rules

1. In a healthy jj workspace, jj is the backup and rollback layer.
2. In a healthy jj workspace, do not create `.bak` files for normal edits.
3. If jj is unavailable, inactive, broken, or initialization is declined, fall back to the normal `.bak` rule for existing-file edits.
4. Do not silently switch strategies mid-task. If jj stops working after edits begin, pause and ask before changing from jj-backed safety to `.bak` fallback.
5. Default checkpointing is one jj finalization checkpoint when the task is complete.
6. Per-change checkpointing is opt-in only and must be explicitly enabled by the user.
7. If the current environment requires approval before mutating commands, ask before running `jj git init`, `jj commit`, or any other mutating jj command.

## Health Check

Treat jj as healthy only when all of the following are true:
- the `jj` command exists
- the current directory is inside a jj workspace
- read-only jj inspection commands succeed cleanly

Minimal read-only checks:
- `jj root`
- `jj status`

If these checks fail, do not assume jj is safe to rely on.

## Decision Flow

### State A: healthy jj workspace already active

Use jj as the history layer.

- edit normally
- do not create `.bak` files
- use the default completion checkpoint at task end unless the user enabled per-change mode

### State B: Git repo exists, but jj is not active here

Ask whether to initialize jj with colocation.

Preferred prompt:

> This directory is a Git repo but not an active jj workspace. Do you want me to initialize jj here with `jj git init --colocate` so jj can replace `.bak` backups and manage task checkpoints?

Notes:
- prefer `jj git init --colocate` for an existing Git repo
- if jj reports a Git-worktree-specific refusal, stop and ask whether to initialize from the main repo or use a workspace-specific flow instead

### State C: neither jj nor Git is active here

Ask whether to initialize jj.

Preferred prompt:

> This directory is not an active jj workspace. Do you want me to initialize it with `jj git init` before I edit files, so jj can replace `.bak` backups and manage task checkpoints?

### State D: jj unavailable, broken, or initialization declined

Use the normal backup rule for existing-file edits.

- create `.bak` backups only in this fallback state
- continue without jj-specific checkpoint rules

## Backup Policy

Inside a healthy jj workspace:
- no `.bak` files for normal file edits
- jj history is the primary recovery path

Outside a healthy jj workspace:
- use the environment's normal backup policy
- if that policy is `.bak`, create the `.bak` only then

If a repo starts dirty, be careful with finalization commands. Do not blindly sweep unrelated changes into the same jj checkpoint without the user's approval.

## Checkpoint Policy

### Default mode

Default meaning of “auto-commit” in this skill:

- create one jj task-finalization checkpoint when the requested task is complete
- do not treat incidental jj working-copy snapshot behavior as sufficient finalization
- use a clear task-level description/message when finalizing

This is the default mode.

### Opt-in per-change mode

Only enable this when the user explicitly asks.

Trigger command:
- `automatic-jj: commit-every-change`

Meaning:
- after each completed mutating edit batch or tool-driven file-change batch, create a jj checkpoint
- this is not “every keystroke”
- this mode is task-scoped and turns off again after the task finishes unless the user explicitly re-enables it later

Disable trigger:
- `automatic-jj: normal-mode`

Meaning:
- return to the default single finalization checkpoint at task completion

## Completion Guidance

Before a final jj checkpoint:
- finish the requested edit scope
- run the required verification for the environment or task
- make sure the checkpoint message describes the task outcome, not just the file name

If the repo was already dirty before work began, ask before creating a final checkpoint that would mix unrelated changes.

## Quick Reference

| Situation | Action |
|---|---|
| Healthy jj workspace | Use jj, no `.bak` |
| Git repo without jj | Ask about `jj git init --colocate` |
| No Git, no jj | Ask about `jj git init` |
| jj unavailable or broken | Fall back to `.bak` |
| Task complete | One jj finalization checkpoint |
| User wants checkpoint after every change | Enable `automatic-jj: commit-every-change` |
| User wants to leave per-change mode | Use `automatic-jj: normal-mode` |

## Common Mistakes

- Treating incidental jj working-copy snapshots as the same thing as task finalization
- Offering `jj git init` and `jj git init --colocate` as if they were the same choice in an existing Git repo
- Creating `.bak` files inside a healthy jj workspace
- Silently switching from jj-backed safety to `.bak` fallback mid-task
- Enabling per-change checkpointing without an explicit user request

## Non-Goals

This skill does not:
- install background watchers or hooks
- force jj initialization without the user's approval
- define one universal commit-message template for every environment
- rename machine slugs or agent identifiers automatically
