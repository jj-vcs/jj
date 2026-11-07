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

At the top, replace the `Unreleased` header with:

```md
## [Unreleased]

### Release highlights

### Breaking changes

### Deprecations

### New features

### Fixed bugs

## [$NEW_VERSION] - YYYY-MM-DD
```

Feel free to copy-edit the changelog entries in order to:

* Populate "Release highlights" if relevant
* Remove empty sections
* Put more important items first so the reader doesn't miss them
* Make items consistent when it comes to language and formatting
* Catch any misplaced changelog items by looking at the CHANGELOG diff

To get the CHANGELOG diff, you can run

```shell
jj log -r 'heads(remote_tags())'  # Check that this shows $OLD_VERSION
jj diff --from 'heads(remote_tags())' --to main CHANGELOG.md
```

Make sure to add a corresponding reference link at the bottom of the
CHANGELOG for the new version's tag. It should be the GitHub URL comparing
`$OLD_VERSION` with `$NEW_VERSION` (e.g.,
`https://github.com/jj-vcs/jj/compare/v0.32.0...v0.33.0`). Also update the
`[unreleased]` link accordingly, comparing `$NEW_VERSION` against `HEAD`.

Producing the list of contributors is a bit annoying. The current suggestion is
to run something like this:

```shell
root=$(jj log -G -r 'heads(remote_tags(glob:"v*.*.*") & ::trunk())' -T commit_id)
filter='
map(.commits[] | select(.author.login | (. != null and endswith("[bot]") | not)))
  | unique_by(if .author.login != null then .author.login else .author.email end)
  | map(if .author.login != null
        then "* \(.commit.author.name) (@\(.author.login))"
        else "* \(.commit.author.name)"
        end)
  | .[]
'
gh api "/repos/jj-vcs/jj/compare/$root...main" --paginate | jq -sr "$filter" | sort -f
```

<https://docs.github.com/en/rest/commits/commits?apiVersion=2022-11-28#compare-two-commits>

Alternatively, the list can be produced locally:

```shell
jj log -G -r 'heads(remote_tags())..trunk()' \
   -T 'if(!self.author().name().ends_with("[bot]"), "* " ++ self.author() ++ "\n")' \
   | sort -fu
```

Then try to find the right GitHub username for each person and copy their name
and username from their GitHub profile (e.g., <https://github.com/martinvonz>).

It might be good to cross-reference both of these outputs just in case.

Add the list of contributors under the last subsection for this version (usually
`Fixed bugs`):

```md
### Contributors

Thanks to the people who made this release happen!

* Name 1 (@github1)
* Name 2 (@github2)
* ...
```

### Update Cargo versions

Search and replace `$OLD_VERSION` for `$NEW_VERSION` in `Cargo.toml` and
`Cargo.lock`. For example:

```shell
# Escape the dots in the pattern so it doesn't match as a wildcard.
sed -i 's/"0\.32\.0"/"0.33.0"/g' Cargo.toml Cargo.lock
# On macOS, pass an empty 'extension' to `-i`:
sed -i '' 's/"0\.32\.0"/"0.33.0"/g' Cargo.toml Cargo.lock
```

The package updates in `Cargo.lock` should only be for `gen-protos`,
`testutils`, and `jj-*` packages. `Cargo.toml` should update
`workspace.package.version` and all `workspace.dependencies.jj-*.version`
values.

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
