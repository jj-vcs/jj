# Configuring `revsets.log`

## Explaining the default log revset

## Large repositories

For large repositories with many contributors the default log revset is very
noisy, to improve upon that many people from the community define a `stack()`
revset. They often differ in minor detail but all have a common base, the
`reachable(ancestors(@), 5)` expression.


```text

```



[revset]: ../revsets.md
TODO: everything
