# Style guide

## Panics

Panics are not allowed, especially in code that may run on a server. Calling
`.unwrap()` is okay if it's guaranteed to be safe by previous checks or
documented invariants. For example, if a function is documented as requiring
a non-empty slice as input, it's fine to call `slice[0]` and panic.

## Markdown

Try to wrap at 80 columns. We don't have a formatter yet.

## Prefer lower-level tests to end-to-end tests

When possible, prefer lower-level tests that don't use the `jj` binary.
End-to-end tests are much slower than similar tests that create a repo using
`jj-lib` (roughly 100x slower). It's also often easier to test edge cases in
lower-level tests.

It can still be useful to add a test case or two to check that the lower-level
functionality is correctly hooked up in the CLI. For example, the end-to-end
tests for `jj log` don't need to test that all kinds of revsets are evaluated
correctly (we have tests in `jj-lib` for that), but they should check that the
`-r` flag is respected.

Use end-to-end tests for testing the CLI commands themselves.
