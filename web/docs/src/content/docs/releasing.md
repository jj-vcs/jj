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
adds the link comparing the previous version tag with the new one at the bottom
of `CHANGELOG.md`, and deletes the notes. To see the section without changing
anything, run `uv run changelog preview`.

Then copy-edit the new section in order to:

* Add a `### Release highlights` section if relevant, at the top (below
  "Security fixes", if any)
* Add a `### Contributors` section at the end (see below)
* Put more important items first so the reader doesn't miss them
* Make items consistent when it comes to language and formatting
* Catch any misplaced changelog items, such as a breaking change that was
  filed as a new feature

Producing the list of contributors is a bit annoying. The current suggestion is
to run something like this:

```shell
root=$(jj log --no-graph -r 'heads(tags(glob:"v*.*.*") & ::trunk())' -T commit_id)
filter='
   map(.commits[] | select(.author.login | endswith("[bot]") | not))
   | unique_by(.author.login)
   | map("* \(.commit.author.name) (@\(.author.login))")
   | .[]
'
gh api "/repos/jj-vcs/jj/compare/$root...main" --paginate | jq -sr "$filter" | sort -f
```

<https://docs.github.com/en/rest/commits/commits?apiVersion=2022-11-28#compare-two-commits>

Alternatively, the list can be produced locally:

```shell
jj log --no-graph -r 'heads(tags())..main' -T '"* " ++ author ++ "\n"' | sort -fu
```

Then try to find the right GitHub username for each person and copy their name
and username from the GitHub page for the person
(e.g. <https://github.com/martinvonz>).

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
