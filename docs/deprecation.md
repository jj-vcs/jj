# Deprecation Policy & Breaking Changes

This documentation gives a short outline of our deprecation strategy in the
project. The general policy is here to ensure that we don't break our users
workflows when renaming a command or making an argument required. The basis of
this policy was introduced in [PR #1911] when we removed
`--allow-large-revsets` in favor of `all:`.

Binary format changes are currently not mentioned, as they currently are
supported for a year or longer.

## User-facing commands and their arguments

When we rename a command or make a previously optional argument required,
we usually try to keep the old command invocations working for 6
months (so 6 releases, since we release monthly) with a deprecation message.
The message should inform the user that the previous workflow is deprecated
and to be removed in the future.

## Niche commands

For commands with a niche user audience or something we assume is rarely used
(we sadly have no data), we take the liberty to remove the old behavior within
two releases. This means that you can change the old command to immediately
return a error. A example is if we want to rename `jj debug reindex` to
`jj util reindex` in a release, then we make `jj debug reindex` an error in the
same patchset. Since `util` and `debug` commands generally shouldn't be used as
large API surface, it still should be fine to extend them without a required
deprecation period.

## Third-party dependencies

For third-party dependencies which previously were used for a core functionality
like `libgit2` was before the `[git.subprocess]` option was added, we're free
to remove most codepaths and move it to a `cargo` feature which we support
up to 6 releases, this is to ease transition for package managers. This is done
on a case-by-case basis to allow the project to remove third-party dependencies
as soon as possible.

[PR #1911]: https://github.com/jj-vcs/jj/pull/1911
