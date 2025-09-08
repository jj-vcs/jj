# About

[Build status]: https://img.shields.io/github/actions/workflow/status/arxanas/scm-record/.github%2Fworkflows%2Flinux.yml
[link-build-status]: https://github.com/arxanas/scm-record/actions?branch=main
[Latest version]: https://img.shields.io/crates/v/scm-record.svg
[link-latest-version]: https://crates.io/crates/scm-record
[Docs]: https://img.shields.io/docsrs/scm-record
[link-docs]: https://docs.rs/scm-record/latest/scm_record/
[License]: https://img.shields.io/crates/l/scm-record
[link-license]: https://github.com/arxanas/scm-record/tree/main/scm-record

[![Build status]][link-build-status] [![Latest version]][link-latest-version] [![Docs]][link-docs] [![License]][link-license]

`scm-record` is a Rust library for a UI component to interactively select changes to include in a commit. It's meant to be embedded in source control tooling.

You can think of this as an interactive replacement for `git add -p`, or a reimplementation of `hg crecord`/`hg commit -i`. Given a set of changes made by the user, this component presents them to the user and lets them select which of those changes should be staged for commit.

The `scm-record` library is directly integrated into these projects:

- [git-branchless](https://github.com/arxanas/git-branchless): the `git record -i` command lets you interactively select and commit changes.
- [Jujutsu](https://github.com/martinvonz/jj): as the built-in diff editor; see the [`ui.diff-editor`](https://martinvonz.github.io/jj/latest/config/#editing-diffs) configuration option.

## Standalone executable

`scm-diff-editor` is a standalone executable that uses `scm-record` as the front-end. It can be installed via `cargo`:

```sh
$ cargo install --locked scm-diff-editor
```

The `scm-diff-editor` executable can be used with these tools:

- [Git](https://git-scm.org): as a [difftool](https://git-scm.com/docs/git-difftool).
- [Mercurial](https://www.mercurial-scm.org/): via [the `extdiff` extension](https://wiki.mercurial-scm.org/ExtdiffExtension).
- Likely other source control systems as well.

# Future work

## Feature wishlist

Here are some features in the UI which are not yet implemented:

- [ ] Make the keybindings easier to discover (https://github.com/arxanas/scm-record/issues/25).
- [ ] Support accessing the menu with the keyboard (https://github.com/arxanas/scm-record/issues/44).
- [ ] Edit one side of the diff in an editor (https://github.com/arxanas/scm-record/issues/83).
- [ ] Multi-way split UI to split a commit into more than 2 commits (https://github.com/arxanas/scm-record/issues/73).
- [ ] Support for use as a mergetool.
- [ ] Commands to select ours/theirs for diffs representing merge conflicts.

## Integration with other projects

Here's some projects that don't use `scm-record`, but could benefit from integration with it (with your contribution):

- [Sapling](https://sapling-scm.com/)
- [Stacked Git](https://stacked-git.github.io/)
- [Pijul](https://pijul.org/)
- [gitoxide/ein](https://github.com/Byron/gitoxide)
- [gitui](https://github.com/extrawurst/gitui)
- [Game of Trees](https://gameoftrees.org/)
