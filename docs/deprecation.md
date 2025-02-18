# Deprecation Policy & Breaking Changes

This documentation gives a short outline of our deprecation strategy in the
project.

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
same patchset.

## Third-party dependencies

For third-party dependencies which previously were used for a core functionality
like `libgit2` was before the `[git.subprocess]` option was added, we're free
to remove most codepaths and move it to a `cargo` feature which we support
up to 6 releases, this is to ease transition for package managers.
