# automatic-jj

This folder is the canonical source for the `automatic-jj` rollout.

It contains two install surfaces:

- `SKILL.md` — version 2, the full reusable skill folder
- `instruction-snippet.md` — version 1, the short instruction-file snippet that references `@automatic-jj`

## Intended rollout order

1. Create and review the canonical source here first.
2. Install the version-2 skill folder into target skill directories.
3. Add the version-1 snippet to target instruction files with a clearly delimited managed block.

## Version 1: instruction-file install

Use `instruction-snippet.md` as the managed text block for instruction files such as `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `SOUL.md`, and similar files.

The snippet is intentionally short. It delegates detailed behavior to `@automatic-jj` instead of duplicating the full policy in every instruction file.

## Version 2: skill-folder install

Install the entire `automatic-jj/` folder into any target skill root that supports `SKILL.md` discovery.

For community-facing installation flows, this version is the one that maps naturally to `npx skills add` style distribution.

## Design constraints captured here

- no `.bak` files inside a healthy jj workspace
- `.bak` fallback only when jj is unavailable, broken, or declined
- default jj finalization at task completion
- explicit opt-in per-change checkpointing via `automatic-jj: commit-every-change`
- explicit return to default mode via `automatic-jj: normal-mode`
