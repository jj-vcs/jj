---
title: How to do a release
---

## Update changelog and Cargo versions

Send a PR similar to <https://github.com/jj-vcs/jj/pull/7954>. Start by
compiling the changelog notes in `changelog/` into a new section of
`CHANGELOG.md`:

```shell
uv run changelog release 0.<number>.0
```

This adds a `## [0.<number>.0] - <date>` section above the previous release
(the date defaults to today in UTC; pass `--date YYYY-MM-DD` to override it),
fills in its "Contributors" section, adds the link comparing the previous
version tag with the new one at the bottom of `CHANGELOG.md`, and deletes the
notes. To see the section without changing anything, run
`uv run changelog preview`.

The contributors are the authors of the commits between the previous version's
tag and `main` on GitHub, excluding bots. Listing them requires the
[GitHub CLI](https://cli.github.com/): install it and run `gh auth login`, or
pass `--no-contributors` and add the section by hand.

Then copy-edit the new section in order to:

* Add a `### Release highlights` section if relevant, at the top (below
  "Security fixes", if any)
* Add GitHub usernames to contributors listed by name only (their commits'
  email addresses aren't linked to a GitHub account), if you can find them
* Put more important items first so the reader doesn't miss them
* Make items consistent when it comes to language and formatting
* Catch any misplaced changelog items, such as a breaking change that was
  filed as a new feature

Get the PR through review and get it merged as usual.

## Create a tag and a GitHub release

1. Go to <https://github.com/jj-vcs/jj/releases> and click "Draft a new release"
2. Click "Choose a tag" and enter "v0.\<number\>.0" (e.g. "v0.26.0") to create a
   new tag
3. Click "Target", then "Recent commits", and select the commit from your merged
   PR
4. Use the name (e.g. "v0.26.0") as "Release title". Paste the changelog entries
   into the message body
5. Check "Create a discussion for this release"
6. Click "Publish release"

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
(cd lib/proc-macros && cargo publish)
(cd lib && cargo publish)
(cd cli && cargo publish)
```

[^1]: We recommend publishing from a new clone because `cargo publish` will
      archive ignored files if they match the patterns in `[include]`
      ([example](https://github.com/jj-vcs/jj/blob/b95628c398c6c3d11f41bdf53d0aef11f92ee96d/lib/Cargo.toml#L15-L22)),
      so it's a security risk to run it an existing clone where you may have
      left sensitive content in an ignored file.
