# Changelog notes

Each Markdown file in this directory (except this README) describes one
unreleased change. If you are making a contribution, add a note here instead of
editing `CHANGELOG.md`. The Python files in this directory implement the
`uv run changelog` tool.

When preparing a monthly release, the notes are compiled into a new section of
`CHANGELOG.md` and deleted.

## Adding a note

Run this anywhere in the repository and answer the questions:

```shell
uv run changelog new
```

The result is a new file in the `changelog/` directory. Add one note per
user-visible change.

## Format

```markdown
---
type: feature
breaking_note: |
  `jj foo` no longer accepts `--baz`. Use `--bar` instead.
---
`jj foo` supports a new `--bar` flag.
[#1234](https://github.com/jj-vcs/jj/issues/1234)
```

The YAML front matter can contain:

* `type`: where the entry body goes in `CHANGELOG.md`. One of `feature` (New
  features), `fix` (Fixed bugs), `deprecation` (Deprecations), `packaging`
  (Packaging changes), or `security` (Security fixes).
* `breaking_note`: optional. A separate entry for the "Breaking changes"
  section describing what users need to know.

The changelog entry follows the front matter. Don't start it with a list bullet.

If introducing a breaking change, a note may omit the `type` and body.

The body and `breaking_note` must be formatted with
[mdformat](https://mdformat.readthedocs.io/) wrapped at 78 columns, which is
what `new` does. To check your notes, run:

```shell
uv run changelog check
```

## Releasing

See [How to do a release](../docs/releasing.md).
