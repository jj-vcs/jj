# Jujutsu Marko Map

Source read:
- `README.md`
- `docs/technical/architecture.md`
- `docs/core_tenets.md`
- `docs/glossary.md`
- `docs/working-copy.md`
- `docs/operation-log.md`
- `docs/revsets.md`
- Marko private project docs at `/Users/mm/Documents/dev/TGramm/marko/{README.md,SYNTAX.md,docs/USAGE.md,docs/INTERNALS.md}`

## Project Boundary

Jujutsu is a Rust workspace whose public binary is `jj`.

In boundary:
- `cli/`: terminal UI, command model, configuration loading, parsing global arguments, command dispatch, output formatting, shell completion, command implementations.
- `lib/`: storage-independent version-control model and algorithms. This is the core API boundary intended to be usable by non-CLI frontends.
- `docs/` and `web/docs/`: user and technical documentation. `docs/technical/architecture.md` defines the primary source architecture contract.
- `demos/`, `cli/tests/`, `lib/tests/`, examples, generated proto support: validation, examples, generated support, and integration surfaces.

Out of boundary:
- User home config, terminal I/O policy, editor process behavior, Git remote hosting behavior, and any external backend not present in this checkout.

Primary architectural constraint from the docs: the CLI owns user interaction; `jj-lib` should avoid terminal I/O, user-home config reads, and UI policy.

## Levels And Layers

| Level | Layer | Source surface | Responsibility | Marko group |
|---|---|---|---|---|
| L0 | Product/docs | `README.md`, `docs/*.md` | User concepts, glossary, workflows, and architecture intent | mapping only |
| L1 | Binary entry | `cli/src/main.rs` | Construct and run `CliRunner` | `jj-entry` |
| L2 | CLI platform | `cli/src/cli_util.rs` | config/env loading, argument expansion, workspace/repo loading, snapshot/update orchestration | `jj-entry`, `jj-command-surface`, `jj-working-copy-flow` |
| L3 | Command registry | `cli/src/commands/mod.rs`, `cli/src/commands/*.rs` | Clap command enum, dispatch table, command-specific behavior | `jj-command-surface` |
| L4 | Core domain API | `lib/src/repo.rs`, `lib/src/transaction.rs`, `lib/src/commit*.rs`, `lib/src/rewrite.rs` | repo views, mutable repos, transactions, commit creation/rewrites | `jj-repo-transaction-flow` |
| L5 | Query languages | `lib/src/revset*.rs`, `lib/src/fileset*.rs`, `cli/src/template*.rs` | parse/lower/resolve/evaluate user expressions | `jj-query-flow` |
| L6 | State surfaces | `lib/src/working_copy.rs`, `lib/src/local_working_copy.rs`, `lib/src/workspace.rs` | working-copy state, snapshot, checkout, stale detection | `jj-working-copy-flow` |
| L7 | Storage/backends | `lib/src/backend.rs`, `lib/src/git_backend.rs`, `lib/src/op_store.rs`, `lib/src/op_heads_store.rs`, `lib/src/index.rs`, `lib/src/store.rs` | commit/object storage, operation store, op-head store, index store | `jj-git-flow` |
| L8 | Git interop | `lib/src/git.rs`, `cli/src/git_util.rs`, `cli/src/commands/git/*` | import/export refs, remotes, colocated Git behavior, Git backend adapter | `jj-git-flow` |

Granularity used for source Marko tags:
- Project level: this map.
- Layer level: `.marks.toml` groups.
- Surface level: source `@flag` and `@wrap` tags on entry structs/functions.
- Function level: individual `@wrap` anchors on key functions that carry flow transitions.

## Main Flow: CLI Command Execution

1. `jj-cli-entry-main`: `cli/src/main.rs` creates `CliRunner`.
2. `jj-cli-runner-init`: initializes tracing, cleanup guard, default command app, config layers, store factories, working-copy factories, revset/template extensions, and default command dispatch.
3. `jj-cli-run`: creates `Ui`, loads environment config, blocks on `run_internal`, handles command result, finalizes pager.
4. `jj-cli-run-internal`: canonicalizes cwd, loads user/repo/workspace config, expands aliases, parses arguments, resolves workspace loader, builds `CommandHelper`, then invokes dispatch.
5. `jj-cli-command-dispatch`: converts Clap matches to `Command` and calls the command implementation.

Function-level source anchors:
- `cli/src/main.rs`: `fn main`
- `cli/src/cli_util.rs`: `CliRunner::init`, `CliRunner::run_internal`, `CliRunner::run`
- `cli/src/commands/mod.rs`: `default_app`, `run_command`

## Main Flow: Workspace Snapshot And Update

1. Commands that need a workspace call `CommandHelper::workspace_helper_with_stats`.
2. `workspace_helper_no_snapshot` loads `Workspace`, resolves the operation, loads `ReadonlyRepo`, builds workspace environment.
3. `WorkspaceCommandHelper::maybe_snapshot_impl` coordinates Git import lock, stale-op reload, Git HEAD import, working-copy snapshot, then Git ref import.
4. `handle_stale_working_copy` compares the locked working copy operation/tree with the repo view and returns stale/fresh/reloaded outcomes.
5. `update_working_copy` delegates checkout to `Workspace::check_out`.
6. `LockedLocalWorkingCopy::{snapshot,check_out,finish}` persist tree and checkout state.

Function-level source anchors:
- `jj-cli-workspace-helper`
- `jj-cli-snapshot-flow`
- `jj-cli-stale-working-copy-flow`
- `jj-cli-update-working-copy`
- `jj-lib-working-copy-start-mutation`
- `jj-lib-working-copy-snapshot`
- `jj-lib-working-copy-checkout`
- `jj-lib-working-copy-finish`

## Main Flow: Repo Mutation And Operation Commit

1. `jj-cli-start-transaction` starts a transaction from `ReadonlyRepo`, sets workspace name, and records shell args metadata.
2. `jj-lib-mutable-repo` owns mutable view/index state, commit predecessors, and rewrite parent mapping.
3. `jj-lib-new-commit` and `jj-lib-rewrite-commit` create attached builders for new and rewritten commits.
4. `jj-lib-transaction-write-operation` writes the view, operation object, mutable index, and returns an unpublished operation.
5. `jj-lib-transaction-commit` writes and publishes the operation to op heads.

Function-level source anchors:
- `lib/src/repo.rs`: `RepoLoader::init_from_file_system`, `MutableRepo::new`, `MutableRepo::new_commit`, `MutableRepo::rewrite_commit`
- `lib/src/transaction.rs`: `Transaction::new`, `Transaction::write`, `Transaction::commit`

## Main Flow: Revset Query

1. User text enters CLI command args and is passed to revset utility/evaluator code.
2. `jj-lib-revset-parse` parses Pest syntax into an expression node, expands aliases, and lowers to `UserRevsetExpression`.
3. `jj-lib-revset-resolve-symbols` binds names to commits/bookmarks/tags/refs through repo and symbol resolver.
4. `jj-lib-revset-resolve-visibility` inserts visibility bounds such as `all()` and `visible_heads()`.
5. `jj-lib-revset-optimize` rewrites the expression tree to reduce evaluation cost before index-backed evaluation.

Function-level source anchors:
- `lib/src/revset.rs`: `parse`, `optimize`, `resolve_symbols`, `resolve_visibility`

## Main Flow: Git Interop

1. CLI snapshot path may call Git import/export helpers when working copy is colocated with Git.
2. `jj-lib-git-import-head` imports Git `HEAD` into the mutable repo view.
3. `jj-lib-git-import-refs` imports Git refs and remote tags/bookmarks.
4. `jj-lib-git-export-refs` exports local view updates back to Git refs.
5. `jj-lib-git-backend` and `jj-lib-git-backend-trait-impl` adapt Git object storage to `Backend`.

Function-level source anchors:
- `lib/src/git.rs`: `import_head`, `import_refs`, `export_refs`
- `lib/src/git_backend.rs`: `GitBackend`, `impl Backend for GitBackend`

## Source Surfaces

| Surface | Tag | File | Role |
|---|---|---|---|
| CLI entry | `jj-cli-entry-main` | `cli/src/main.rs` | Binary entry and process exit code |
| CLI runner | `jj-cli-runner` | `cli/src/cli_util.rs` | Builder/runner object for the entire CLI |
| Command registry | `jj-cli-command-registry` | `cli/src/commands/mod.rs` | Clap command enum and style surface |
| Dispatch | `jj-cli-command-dispatch` | `cli/src/commands/mod.rs` | Routes parsed subcommand to implementation |
| Workspace helper | `jj-cli-workspace-helper` | `cli/src/cli_util.rs` | Loads workspace/repo and snapshots |
| Snapshot flow | `jj-cli-snapshot-flow` | `cli/src/cli_util.rs` | Git import, snapshot, ref import |
| Repo loader | `jj-lib-repo-loader` | `lib/src/repo.rs` | Loads a repo at a concrete operation |
| Mutable repo | `jj-lib-mutable-repo` | `lib/src/repo.rs` | Mutable view/index and rewrite state |
| Transaction write | `jj-lib-transaction-write-operation` | `lib/src/transaction.rs` | Persists view/op/index |
| Revset parse | `jj-lib-revset-parse` | `lib/src/revset.rs` | User expression to lowered AST |
| Local working copy | `jj-lib-local-working-copy` | `lib/src/local_working_copy.rs` | Disk-backed working-copy implementation |
| Git refs | `jj-lib-git-import-refs`, `jj-lib-git-export-refs` | `lib/src/git.rs` | Git ref import/export |

## Reusable Module Candidates

These were selected for the Marko bank because they are cohesive and reusable across new frontends, tooling, or analysis agents:

- `CliRunner`: reusable CLI bootstrap and extension hook pattern.
- `CommandHelper`: reusable command context boundary.
- `WorkspaceCommandHelper` snapshot helpers: reusable workspace/repo preparation boundary.
- `start_repo_transaction`: reusable operation metadata wrapper.
- `RepoLoader`: reusable repository loading boundary over store factories.
- `MutableRepo`: reusable in-memory mutation model.
- `Transaction::write`: reusable operation persistence pattern.
- `revset::parse` and `revset::optimize`: reusable DSL front-end and optimization passes.
- `LocalWorkingCopy` and locked working-copy methods: reusable stateful filesystem mutation pattern.
- `git::{import_refs,export_refs}`: reusable external-ref synchronization pattern.
