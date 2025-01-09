# Revset configuration

## Configurable revsets

Settings in the `revsets` section configure Jujutsu itself. These revsets may
use any [revset aliases](#revset-aliases) that you have defined.

### `log`: Default log revisions {#revsets-log}

You can configure the revisions `jj log` would show when neither `-r` nor any
paths are specified.

```toml
[revsets]
# Show commits that are not in `main@origin`
log = "main@origin.."
```

The default value for `revsets.log` is
`'present(@) | ancestors(immutable_heads().., 2) | present(trunk())'`.

### `short-prefixes`: Commit and change ID short prefixes {#revsets-short-prefixes}

To control which revisions get priority for shorter prefixes, set
`revsets.short-prefixes`:

```toml
[revsets]
# Prioritize the current bookmark
short-prefixes = "(main..@)::"
```

## Revset aliases

Some revset aliases are built-in to Jujutsu and used to control its behavior.

See the [revset language reference](revsets.md) for more information about the
revset language and defining your own revset aliases.

### `trunk()`: Head of the main line of development {#trunk}

Most teams have a main line of development, usually named `main`, `master`, or
`trunk`. The `trunk()` alias resolves to the bookmark for this branch.

This alias is used in the default values for the [log
revset](#revsets-log) and for
[`immutable_heads()`](#immutable_heads).

When Jujutsu clones a Git repository, it uses the remote's HEAD to set this
revset aliases in the repo's config.

```toml
[revset-aliases]
"trunk()" = "dev@origin"
```

### `immutable_heads()`: Change the set of immutable commits {#immutable_heads}

Controls which commits are immutable. Jujutsu will refuse to modify these
commits unless `--ignore-immutable` is specified.

Many teams have a main line of development, usually named `main`, `master`, or
`trunk`.

Ancestors of the configured set are also immutable. The root commit is always
immutable even if the set is empty.

Default value: `builtin_immutable_heads()`, which in turn is defined as
`present(trunk()) | tags() | untracked_remote_bookmarks()`.

For example, to also consider the `release@origin` bookmark immutable:

```toml
[revset-aliases]
"immutable_heads()" = "builtin_immutable_heads() | release@origin"
```

To prevent rewriting commits authored by other users:

```toml
# The `trunk().. &` bit is an optimization to scan for non-`mine()` commits
# only among commits that are not in `trunk()`.
[revset-aliases]
"immutable_heads()" = "builtin_immutable_heads() | (trunk().. & ~mine())"
```
