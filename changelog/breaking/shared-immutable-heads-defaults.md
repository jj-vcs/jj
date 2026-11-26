The default `jj log` revset, the default `revsets.short-prefixes`, and the
default `revsets.op-diff-changes-in` now use the new
`shared_immutable_heads()` revset alias instead of `immutable_heads()`. If you
have overridden `immutable_heads()`, you may want to override
`shared_immutable_heads()` instead (or as well). For example, if you have
overridden it as `builtin_immutable_heads() | release@origin`, you probably
want to override `shared_immutable_heads()` as
`builtin_shared_immutable_heads() | release@origin` instead. See
[docs](docs/config.md#set-of-immutable-commits)
