# Machine-readable completion output

Initial Version, 22.04.2026

**Summary:** This document proposes an editor-facing completion command for
Jujutsu. The goal is to expose structured completion candidates without forcing
callers to reverse-engineer shell-specific completion text.

## Context

Jujutsu already has rich completion logic in `cli/src/complete.rs`. Internally,
completion candidates can carry more than a plain string value:

- a candidate value,
- a help string,
- a de-duplication id,
- a grouping tag,
- a display order,
- a hidden/visible flag.

However, the current public dynamic-completion interface is shell oriented. In
practice, shells consume a lossy text protocol such as Fish's
`candidate<TAB>help`, which is good for shell UIs but not ideal for editor
frontends.

Editor integrations like Emacs or IDE plugins want machine-readable completion
results so they can:

- render annotations in their own UI,
- preserve ordering and hidden-state information,
- avoid guessing semantic kinds from human-oriented text,
- stop maintaining fallback completion sources solely to recover structure that
  `jj` already knows internally.

## Goals

- Expose structured completion results for editor and tool integrations.
- Reuse the same clap-based completion engine as shell completion.
- Preserve alias expansion and default-command behavior on the left side of the
  completion cursor.
- Keep shell completion behavior unchanged.

## Non-goals

- Replacing shell completion scripts.
- Freezing a large semantic schema for every completion source on day one.
- Encoding shell-specific quoting or cursor-rewrite behavior in the new output.

## Design

Add a new command:

```shell
jj util complete --index <N> -- jj <args...>
```

The arguments after `--` are the full command line being completed, including
the leading `jj` binary name. `--index` points at the argument to complete. To
complete after whitespace, callers append an empty final argument and point the
index at that empty argument.

Example:

```shell
jj util complete --index 3 -- jj diff --from ""
```

The command returns JSON containing one object per completion candidate. The
initial schema mirrors clap's existing `CompletionCandidate` model closely:

- `value: string`
- `help: string | null`
- `id: string | null`
- `tag: string | null`
- `display_order: integer | null`
- `hidden: bool`

This schema is intentionally conservative. It exposes the structure Jujutsu
already computes today, while leaving room for future source-specific metadata
if editor integrations need it later.

## Why a new command instead of extending `COMPLETE=fish`

The shell-completion environment protocol is optimized for shells, not editor
integrations. Overloading it with JSON would either:

- create shell-specific divergence, or
- force callers to emulate shell behavior even when they do not want shell text.

A dedicated command is simpler to document, easier to test, and more explicit
for non-shell consumers.

## Future work

Possible follow-ups include:

- additional output formats such as JSON Lines,
- richer source-specific metadata for revsets, paths, bookmarks, or tags,
- a stable versioned schema if external integrations start depending on it.
