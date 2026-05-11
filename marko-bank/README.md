# Jujutsu Marko Bank

This bank was pulled from source with `/Users/mm/Documents/dev/TGramm/marko/target/debug/marko`.

## Files

| File | Source | Use |
|---|---|---|
| `jj-reusable-bank.txt` | `.marks.toml` group `jj-reusable-bank` | Curated reusable modules across CLI, repo, transaction, revset, working copy, and Git refs |
| `jj-entry.txt` | group `jj-entry` | Binary and CLI bootstrap path |
| `jj-command-surface.txt` | group `jj-command-surface` | Command registry, dispatch, and helper context |
| `jj-working-copy-flow.txt` | group `jj-working-copy-flow` | Snapshot, stale working-copy handling, checkout, and locked working-copy persistence |
| `jj-repo-transaction-flow.txt` | group `jj-repo-transaction-flow` | Repo loader, mutable repo, commit builders, and operation write/commit |
| `jj-query-flow.txt` | group `jj-query-flow` | Revset parse, symbol resolution, visibility resolution, and optimization |
| `jj-git-flow.txt` | group `jj-git-flow` | Git import/export and Git backend adapter |
| `source-function-index.txt` | `rg` source index | Function/type/impl index for the tagged source surfaces |

## Reuse Notes

- `CliRunner`, `CommandHelper`, and `WorkspaceCommandHelper` are the reusable CLI-shell pattern.
- `RepoLoader`, `MutableRepo`, and `Transaction::write` are the reusable repo mutation/persistence pattern.
- `revset::parse` and `revset::optimize` are the reusable DSL parse/transform pattern.
- `LocalWorkingCopy` and `LockedLocalWorkingCopy` are the reusable filesystem state mutation pattern.
- `git::import_refs` and `git::export_refs` are the reusable external reference synchronization pattern.

Regenerate a bank file from the repository root with:

```sh
/Users/mm/Documents/dev/TGramm/marko/target/debug/marko pull --group jj-reusable-bank > marko-bank/jj-reusable-bank.txt
```
