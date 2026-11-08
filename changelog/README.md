# Changelog notes

Each Markdown file in a subdirectory of this directory describes one unreleased
change. If you are making a contribution, add a note here instead of editing
`CHANGELOG.md`. The Python files in this directory implement the
`uv run changelog` tool.

When preparing a monthly release, the notes are compiled into a new section of
`CHANGELOG.md` and deleted.

## Adding a note

Run this anywhere in the repository and answer the questions:

```shell
uv run changelog new
```

The result is a new file in a subdirectory of `changelog/`. Add one note per
user-visible change. If a change is breaking and also needs a feature or fix
entry, add a separate note for each.

## Format

A note is a Markdown file in the subdirectory for the `CHANGELOG.md` section
the entry goes in:

| Directory                | Section           |
| ------------------------ | ----------------- |
| `changelog/feature/`     | New features      |
| `changelog/fix/`         | Fixed bugs        |
| `changelog/breaking/`    | Breaking changes  |
| `changelog/deprecation/` | Deprecations      |
| `changelog/packaging/`   | Packaging changes |
| `changelog/security/`    | Security fixes    |

The file name doesn't matter as long as it ends in `.md`. The file contains only
the changelog entry, for example `changelog/feature/jj-foo-bar.md`:

```markdown
`jj foo` supports a new `--bar` flag.
[#1234](https://github.com/jj-vcs/jj/issues/1234)
```

Don't start the entry with a list bullet. It must be formatted with
[mdformat](https://mdformat.readthedocs.io/) wrapped at 78 columns, which is
what `new` does. To check your notes, run:

```shell
uv run changelog check
```

## Releasing

See [How to do a release](../docs/releasing.md).
