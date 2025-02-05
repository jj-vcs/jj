# Deprecation Policy

## User-facing commands and their arguments

Whenever you rename a command or make a previously optional argument required,
we require that you preserve the old command invocations keeps on working for 6
months (so 6 releases, since we release monthly) with a deprecation message.
The message should inform the user that the previous workflow is deprecated
and to be removed in the future.

## Packaging commands and niche commands

For commands with a niche user audience or something we assume is rarely used
(we sadly have no data), we take the liberity to remove the old behavior within
two releases. This means that you can change the old command to immediately
error.

## Third-Party dependencies

For third-party dependencies which previously were used for a core functionality
like `libgit2` was before the `[git.subprocess]` option was added, we're free
to remove most codepaths and move it to a `cargo` feature which we support
up to 6 releases, this is to ease transition for package managers.
