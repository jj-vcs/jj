# Bare workspaces

A **bare repo** is a repository with no default working copy of its own.
Instead of having a single checkout alongside the repo data, you create
named workspaces as separate directories. Each workspace has its own working
copy, and they all share the same commit history.

This mirrors the `git init --bare` + `git worktree add` workflow that many
Git users rely on, but as a first-class Jujutsu concept.

## Creating a bare repo

```sh
jj git init --bare my-project
cd my-project
```

This creates a `my-project/` directory containing only `.jj/`. There is no
working copy here — it is pure repo storage.

## Adding workspaces

From inside the bare repo root, add workspaces with `jj workspace add`:

```sh
jj workspace add main
jj workspace add feat-payment
```

The resulting layout looks like this:

```
my-project/
├── .jj/            ← repo storage (no working copy)
├── main/           ← workspace
└── feat-payment/   ← workspace
```

Each workspace directory contains a `.jj/` that points back to the shared
repo. You can `cd` into any workspace and use `jj` commands normally.

```sh
cd main
jj st
jj new -m "add readme"
```

## Working from the bare repo root

A few commands work from the bare repo root without needing to `cd` into a
workspace:

- `jj workspace add <name>` — add a new workspace
- `jj workspace list` — list all workspaces and their current commits
- `jj log` — show the working-copy commits for all workspaces

All other commands require you to be inside a workspace.

## Compared to Git bare repos + worktrees

If you use Git's bare+worktree layout:

```sh
git init --bare my-project
cd my-project
git worktree add main
git worktree add feat-payment
```

The equivalent with Jujutsu is:

```sh
jj git init --bare my-project
cd my-project
jj workspace add main
jj workspace add feat-payment
```

The main difference is that jj workspaces are independent: none of them is
"privileged" or required for others to function. You can delete any workspace
directory without breaking the others.

## Advantages over a normal workspace layout

In a normal `jj git init` repository, the first workspace and the repo data
live in the same directory. Adding more workspaces puts them alongside the
first one, making one workspace implicitly special (it owns the `.jj/` repo
directory). If you delete it, the other workspaces break.

With a bare repo:

- The `.jj/` directory sits at the project root, not inside any workspace.
- All workspaces are peers — none depends on another.
- Adding and removing workspaces is safe at any time.
- Shell prompts and tools that look for `.jj/` to identify the project root
  find it at the right level.

## Limitations

- `jj log` from the bare root shows only the working-copy commit for each
  workspace, not the full history. Run `jj log -r ::` from inside a workspace
  for the complete graph.
- Template keywords like `@` refer to whichever workspace was used as context
  when running the command. From the bare root, this is the first workspace
  alphabetically.
- `--revision` is not supported for `jj workspace add` when run from the
  bare root. Use `jj new -r <rev>` from inside the workspace after adding it.
