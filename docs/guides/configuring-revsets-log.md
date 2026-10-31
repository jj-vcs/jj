# Configuring `revsets.log`

## Explaining the default log revset

The default log revset of `present(@) | ancestors(immutable_heads().., 2) | trunk()`
where the builtin definition of `immutable_heads()` is `trunk() | `
can be explained as show me all revisions which are an ancestor of the current
revision, mutable and intersected with _any_ currently tracked bookmarks and
tags in the repository. This is an extremely lengthy way to spell
"show me all current mutable revisions".

In many situations this doesn't align with the equivalent in Gits log. Out of
this reason we wrote this Guide.

## Large repositories

For large repositories with many contributors the default log revset is very
noisy, to improve upon that many people from the community define a `stack()`
revset. They often differ in minor detail but all have a common base, the
`reachable(ancestors(@), x)` expression.


```text

```



[revset]: ../revsets.md
TODO: everything
