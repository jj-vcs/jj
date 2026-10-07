# How to do a release

## Create release PR

Create a PR with the following steps, and get it reviewed and merged as usual.
See <https://github.com/jj-vcs/jj/pull/7954> for an example, or see
<https://github.com/jj-vcs/jj/pulls?q=is:pr+release+in:title> for all previous
release PRs.

For ease of explanation, the following will use `$OLD_VERSION` and
`$NEW_VERSION` as placeholders. For example, `$OLD_VERSION` could be `0.32.0`
and `$NEW_VERSION` could be `0.33.0`.

### Update changelog

Start by compiling the changelog notes in `changelog/` into a new section of
`CHANGELOG.md`:

```shell
uv run changelog release $NEW_VERSION
```

This script does the following:

- Adds a `## [$NEW_VERSION] - <date>` section above the previous release
  (the date defaults to today; pass `--date YYYY-MM-DD` to override it)
- Fills in its "Contributors" section
- Adds the link comparing `$OLD_VERSION` with `$NEW_VERSION` at the bottom of
  `CHANGELOG.md`
- Deletes the notes from the `changelog/` directory

To see the section without changing anything, run `uv run changelog preview`.

Listing contributors requires the [GitHub CLI](https://cli.github.com/).

Then copy-edit the new section in order to:

* Add a `### Release highlights` section if relevant, at the top (below
  "Security fixes", if any)
* Add GitHub usernames to contributors if any are missing
* Put more important items first so the reader doesn't miss them
* Make items consistent when it comes to language and formatting
* Catch any miscategorized changelog items, such as a breaking change that was
  filed as a new feature

### Update Cargo versions

Search and replace `$OLD_VERSION` for `$NEW_VERSION` in `Cargo.toml`, then run
`cargo check` to update `Cargo.lock` (don't do that one manually). For example:

```shell
# Escape the dots in the pattern so it doesn't match as a wildcard.
sed -i 's/"0\.32\.0"/"0.33.0"/g' Cargo.toml
# On macOS, pass an empty 'extension' to `-i`:
sed -i '' 's/"0\.32\.0"/"0.33.0"/g' Cargo.toml
```

The version changes in `Cargo.toml` should update `workspace.package.version`
and all `workspace.dependencies.jj-*.version` values. Make sure to check that
other packages aren't affected.

## Create a tag and a GitHub release

1. Go to <https://github.com/jj-vcs/jj/releases> and click "Draft a new release"
2. Click "Select tag" and enter "v0.\<number\>.0" (e.g. "v0.33.0") to create a
   new tag
3. Click "Target", then "Recent commits", and select the commit from your merged
   PR
4. Use the name (e.g. "v0.33.0") as "Release title"
5. Paste this header into the message body. Note that it includes a pinned docs
   link to `$NEW_VERSION`:
   ```md
   ### About

   jj is a Git-compatible version control system that is both simple and powerful.
   See the [installation instructions](https://docs.jj-vcs.dev/v$NEW_VERSION/install-and-setup/) to get started.
   ```
6. After that, paste everything in the changelog for the current version from
   your PR in the rest of the message body (only include the level-3 headings)
7. Keep "Latest" selected for "Release label" (should be default)
8. Check "Create a discussion for this release" (Category: General)
9.  Click "Publish release"

## Publish the crates to crates.io

Go to a terminal and create a new clone of the repo [^1]:

```shell
cd $(mktemp -d)
jj git clone https://github.com/jj-vcs/jj
cd jj
jj new v0.<number>.0
```

Publish each crate:

```shell
cargo publish --workspace
```

[^1]: We recommend publishing from a new clone because `cargo publish` will
      archive ignored files if they match the patterns in `[include]`
      ([example](https://github.com/jj-vcs/jj/blob/b95628c398c6c3d11f41bdf53d0aef11f92ee96d/lib/Cargo.toml#L15-L22)),
      so it's a security risk to run it in an existing clone where you may have
      left sensitive content in an ignored file.

## Discord announcement

In the [#announcements] channel, post a message about the new version release
with a link to the releases page (e.g.,
<https://github.com/jj-vcs/jj/releases/tag/v0.33.0>).

[#announcements]: https://discord.com/channels/968932220549103686/1245847953726378124
