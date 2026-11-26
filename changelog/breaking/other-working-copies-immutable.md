The working-copy commits of other workspaces (and their ancestors) are now
considered immutable by default, so you don't accidentally make the other
workspaces stale. This is done by adding `working_copies() ~ present(@)` to
`builtin_immutable_heads()`. If you have overridden `immutable_heads()` in
terms of `builtin_immutable_heads()` (e.g. as
`builtin_immutable_heads() | release@origin`), you will get the new behavior
too. If you have overridden it without referring to
`builtin_immutable_heads()`, you won't get the new behavior unless you add
`| (working_copies() ~ present(@))` to your definition. See
[docs](docs/config.md#set-of-immutable-commits)
