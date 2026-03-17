# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Jujutsu (jj) is a Git-compatible version control system written in Rust. The working copy is always a commit (no staging area), conflicts are first-class objects stored in commits, and an operation log enables undo. Git repositories serve as the storage backend.

## Build Commands

```bash
# Build
cargo build

# Test (uses cargo-nextest for parallel execution)
cargo nextest run --workspace

# Test with snapshot checking (CI mode — fails on snapshot mismatches)
cargo insta test --workspace --test-runner nextest

# Run a single test
cargo nextest run -p jj-cli test_abandon  # by test name substring
cargo nextest run -p jj-lib test_merge     # target a specific crate

# Update snapshots after intentional output changes
cargo insta test --workspace --test-runner nextest --accept

# Review snapshots interactively
cargo insta review

# Format (requires nightly toolchain)
cargo +nightly fmt
cargo +nightly fmt --check  # lint only

# Clippy
cargo clippy --workspace --all-targets

# Generate protobuf code (after changing .proto files)
cargo run -p gen-protos
```

**Rust version:** 1.89 (enforced in Cargo.toml). **Slow test timeout:** 10s (configured in `.config/nextest.toml`).

## Architecture

### Workspace Crates

| Crate | Path | Purpose |
|-------|------|---------|
| `jj-cli` | `cli/` | CLI binary — commands, templating, formatting, UI |
| `jj-lib` | `lib/` | Core library — backends, repo, working copy, revsets, operations |
| `gen-protos` | `lib/gen-protos/` | Protobuf code generation for internal storage |
| `jj-proc-macros` | `lib/proc-macros/` | Procedural macros |
| `testutils` | `lib/testutils/` | Test harness (`TestEnvironment`, `TestWorkDir`) |

### Layered Design

1. **Backend layer** (`lib/src/backend.rs`, `git_backend.rs`) — Pluggable storage trait. Git backend via gitoxide (`gix`). A `SecretBackend` exists for testing.
2. **Core domain** (`repo.rs`, `commit.rs`, `merge.rs`, `tree_merge.rs`, `operation.rs`) — Immutable repo snapshots, conflict-aware merged trees, operation log.
3. **Mutation layer** — All repo changes go through `Transaction`, which produces a new `Operation`.
4. **Working copy** (`local_working_copy.rs`) — Auto-snapshotted before operations, auto-updated when the working-copy commit changes.
5. **CLI layer** (`cli/src/commands/`) — 134 command modules. Uses `clap` derive for arg parsing, custom template engine for output formatting.

### Key Abstractions

- **`Backend` trait** — Storage interface (commits, trees, files, symlinks, conflicts). Implementations: `GitBackend`, `SimpleOpStore`, `SecretBackend`.
- **`MergedTree`** — Tree that represents conflicts explicitly (not just textual markers).
- **`Revset`** — DSL for selecting commits. Parsed by pest grammar (`revset.pest`), evaluated in `revset.rs`.
- **`Template`** — Output formatting DSL. Parsed from `template.pest`, built in `template_builder.rs`.
- **`Fileset`** — Path pattern matching DSL, parsed from `fileset.pest`.

### Conflict Model

Conflicts are stored as explicit data structures in commits, not text markers. When a commit is rewritten, conflicts propagate automatically to descendants. Resolution in any descendant is recorded and can be applied elsewhere (native rerere-like behavior).

## Testing Patterns

Tests use **Insta snapshot testing** extensively. The standard pattern:

```rust
#[test]
fn test_something() {
    let test_env = TestEnvironment::default();
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    let work_dir = test_env.work_dir("repo");

    work_dir.run_jj(["new", "-m", "my commit"]).success();
    insta::assert_snapshot!(work_dir.run_jj(["log"]), @"...");
}
```

- `TestEnvironment` creates an isolated temp directory with config
- `TestWorkDir` wraps a working directory for running jj commands
- Snapshot strings use `@"..."` inline syntax — run `cargo insta review` to update
- Integration tests live in `cli/tests/`, unit tests inline in modules
- Data-driven tests via `datatest-stable` in some areas

## Code Conventions

- **No unsafe code** — `#![forbid(unsafe_code)]` in jj-lib
- **Error handling** — `thiserror` derive macros, no `.unwrap()` in production code
- **Async** — `async-trait` for trait methods, Tokio runtime
- **Parallelism** — `rayon` for CPU-bound work
- **Commit messages** — `topic: description` format (e.g., `revset: add new function`, `cli: fix log output`). NOT Conventional Commits.
- **23 Clippy lints** enforced at workspace level (see `[workspace.lints.clippy]` in root `Cargo.toml`)
- **Each commit should do one thing** — separate refactoring, features, and test changes

## Key Files for Orientation

- `lib/src/lib.rs` — Public API surface of jj-lib (all `pub mod` declarations)
- `cli/src/commands/mod.rs` — Command registration and dispatch
- `cli/src/cli_util.rs` — Shared CLI infrastructure (WorkspaceCommandHelper, etc.)
- `cli/src/template_builder.rs` — Template language implementation
- `lib/src/revset.rs` — Revset evaluation engine (largest module, ~6500 LOC)
- `lib/src/git.rs` — Git interop layer
- `CONTRIBUTING.md` — PR expectations, commit discipline, review process
