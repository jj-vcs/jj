# Style guide

## Panics

Panics are not allowed, especially in code that may run on a server. Calling
`.unwrap()` is okay if it's guaranteed to be safe by previous checks or
documented invariants. For example, if a function is documented as requiring
a non-empty slice as input, it's fine to call `slice[0]` and panic.

## Markdown

Try to wrap at 80 columns. We don't have a formatter yet.
