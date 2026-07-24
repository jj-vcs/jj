# Revset Evaluation

Whenever a revset is evaluated, it must pass through several stages. These
stages gradually transform the revset to a lower level representation that can
be efficiently evaluated.

## Parsing and Alias Expansion

As a revset is parsed, any uses of revset aliases are substituted with their
definitions. For instance, given the following aliases:

```toml
[revset-aliases]
"trunk()" = "main@origin"
"f(x, n)" = "x | ancestors(x.., n)"
```

Revsets are expanded as follows:

* `trunk()` expands to `main@origin`
* `f(trunk(), 2)` expands to `main@origin | ancestors(main@origin.., 2)`
* `f(main | tags(), 2)` expands to `(main | tags()) | ancestors((main | tags()).., 2)`

Notably, revset expansion works similarly to macro expansion in other languages;
if a revset alias uses a parameter twice, then the entire argument is
substituted into the result twice. This can lead to inefficient evaluations if
an argument is expensive to evaluate.

## Symbol Resolution

Revsets are eventually evaluated by the `Index` implementation, but the `Index`
only stores information about commits and change IDs; it doesn't know about the
current state of the `Repo`. Therefore, any revsets relying on state from the
`Repo` must be resolved early.

Some examples include:

* Working copies (e.g. `@`, `workspace@`)
* Commit ID prefixes (e.g. `123`)
* Change ID prefixes (e.g. `xyz`, `xyz/1`)
* Bookmark/tag names (e.g. `v1.0.0`, `main@origin`)
* `bookmarks()`/`tags()`
* `remote_bookmarks()`/`remote_tags()`

If any of these revsets fail to resolve, a symbol resolution error will be
emitted immediately. `present()` can be used to suppress these errors in cases
where it is expected that a symbol might not exist (e.g. when `main` doesn't
exist, `present(main)` will return `none()` instead of failing).

This stage also handles `at_operation()`, and it also inserts `WithinVisibility`
nodes to keep track of the commit IDs in `visible_heads()`.

Symbol resolution substitutes these revset functions with a list of commit IDs,
meaning the `Index` doesn't need to handle these functions while evaluating a
revset. It is also possible to extend `jj` with custom symbol resolvers which
are also handled by the symbol resolution stage.

*Note: Although the `Index` does store information about commit ID prefixes
and change ID prefixes, we still need to resolve these symbols during this
stage because resolving unique prefixes also requires information about
visible commits and the `revsets.short-prefixes` setting.*

## Optimization

Revset optimization consists of a series of passes that recursively rewrite
revset expressions. The ordering of these passes is important, since one pass
may enable further optimizations in a later pass. These are the current passes:

1. Resolve referenced commits
2. Unfold ranges and differences
3. Fold redundant expressions
4. Fold generations
5. Flatten intersections
6. Sort negations and ancestors
7. Fold ancestors union
8. Internalize filters
9. Fold heads range
10. Fold ranges and differences
11. Fold not in ancestors

### 1. Resolve referenced commits

This pass finds all referenced commit IDs and inserts `WithinReference` nodes to
keep track of referenced commit IDs. This information will be used when lowering
revsets to backend expressions later, since `all()` must also include all
ancestors of commits referenced in the revset in addition to ancestors of
`visible_heads()`. This pass comes first because later passes may remove
redundant expressions.

### 2. Unfold ranges and differences

This pass unfolds range (`x..y`) and difference (`x ~ y`) operations into
intersections. This enables further optimizations in later passes.

Rules:

* `x..y` => `::y & ~(::x)`
* `x ~ y` => `x & ~y`

*Note: During parsing, `x..` is already converted to `~::x`, `..x` is converted
to `root()..x`, and `..` is converted to `~root()`.*

### 3. Fold redundant expressions

This pass optimizes intersections and unions with `all()` and `none()`, as well
as negations.

Rules:

* `~(~x)` => `x`
* `~none()` => `all()`
* `~all()` => `none()`
* `none() | x` => `x`
* `x | none()` => `x`
* `all() | x` => `all()`
* `x | all()` => `all()`
* `none() & x` => `none()`
* `x & none()` => `none()`
* `all() & x` => `x`
* `x & all()` => `x`

### 4. Fold generations

The internal representation of ancestors/parents/descendants/children revsets is
slightly different from the normal revset syntax. There are two basic
operations:

* `ancestors(heads, generation)`
* `descendants(roots, generation)`

In this representation, `generation` is an exclusive range that specifies how
far to walk backwards/forwards from the `heads`/`roots` (unlike the standard
revset syntax, where `generation` is a single number).

For example, this is how the following revsets are represented internally:

* `ancestors(x)` => `ancestors(x, 0..MAX)`
* `ancestors(x, n)` => `ancestors(x, 0..n)`
* `parents(x)` => `ancestors(x, 1..2)`
* `parents(x, n)` => `ancestors(x, n..n+1)`
* `descendants(x)` => `descendants(x, 0..MAX)`
* `descendants(x, n)` => `descendants(x, 0..n)`
* `children(x)` => `descendants(x, 1..2)`
* `children(x, n)` => `descendants(x, n..n+1)`

This optimization pass folds nested `ancestors()`/`descendants()` operations
into a single operation using the following rules:

* `ancestors(ancestors(x, a..b), c..d)` => `ancestors(x, (a..b) + (c..d))`
* `descendants(descendants(x, a..b), c..d)` => `descendants(x, (a..b) + (c..d))

Where the addition of non-empty ranges `a..b` and `c..d` is defined as
`(a + c)..(b + d - 1)`.

Therefore, this pass allows these types of optimizations:

* `x--` => `ancestors(x, 2..3)`
* `::x-` => `ancestors(x, 1..MAX)`
* `ancestors(x---, 5)` => `ancestors(x, 3..8)`

There is also some additional logic for handling `first_parent()` and
`first_ancestors()` correctly.

### 5. Flatten intersections

This pass flattens intersections to be left-associtive. This makes later
optimizations easier to implement. For example:

* `x & (y & z)` => `(x & y) & z`
* `(w & (x & y)) & z` => `((w & x) & y) & z`

### 6. Sort negations and ancestors

This pass sorts the elements in an intersection based on 4 categories:

1. Negated ancestors (`~ancestors(x, n..MAX)`)
2. Ancestors (`ancestors(x, n..MAX)`)
3. Other non-negated expressions
4. Other negated expressions (`~x`)

Negated ancestors are moved to the left to enable the "fold ancestors union"
pass. Placing the ancestors next to the negated ancestors next allows them to be
folded into a range easily in a later pass, and putting other negated
expressions at the end allows them to be folded into a difference easily in a
later pass as well.

Examples:

* `~w & x & ~::y & ::z` => `~::y & ::z & x & ~w`
* `::w & ~::x & ::y & ~::z` => `~::x & ~::z & ::w & ::y`

### 7. Fold ancestors union

This pass folds unions of ancestors into ancestors of unions. The main purpose
of this optimization is to allow combining intersections of ranges into a single
range in later passes (e.g. `a..b & c..` can be optimized to `(a | c)..b`).

Rules:

* `::x | ::y` => `::(x | y)`
* `~::x & ~::y` => `~::(x | y)`

### 8. Internalize filters

Some revsets are implemented as filters. This means that the `Index` can't
directly query for these revsets. Instead, it must check the filter against each
commit individually to test whether it matches.

Examples of filter revsets:

* `merges()`
* `empty()`/`files()`
* `description()`/`subject()`
* `mine()`/`author_*()`/`committer_*()`
* `diff_lines()`
* `conflicts()`
* `signed()`
* `divergent()`

For instance, to evaluate `x..y & conflicts()`, the `Index` implementation first
evaluates `x..y`, then it checks every commit in that range to see whether it
has a conflict or not. If there is no base set of commits, the `Index`
implementation will instead have to check the filter against every commit in
`all()`, which can be expensive on large repos.

The purpose of this optimization pass is to group all filters together at the
end of an intersection, meaning that all non-filter expressions can be
intersected before checking the filter. This reduces the amount of commits that
need to be checked.

Rules:

* `~filter(x)` => `filter(~x)`
* `filter(x) | filter(y) => filter(x | y)`
* `filter(x) | y` => `filter(x | y)`
* `x | filter(y)` => `filter(x | y)`
* `filter(x) & filter(y) => filter(x & y)`
* `filter(x) & y` => `y & filter(x)`
* `(x & filter(y)) & filter(z)` => `x & filter(y & z)`
* `(x & filter(y)) & z` => `(x & z) & filter(y)`

Examples:

* `(::y | empty()) & ::x` => `::x & filter(::y | empty())`
* `conflicts() & ::x & empty()` => `::x & filter(conflicts() & empty())`

### 9. Fold heads range

Often, `heads()` is used to find the most recent commit satisfying some
property. For instance, `heads(::@ ~ empty())` can be used to find the most
recent non-empty ancestors of `@`. If we were to naively evaluate this revset,
we would first have to find every non-empty commit which is an ancestor of `@`,
and then take the `heads()` of that set. This would be very slow, since
`empty()` is an expensive filter to check.

This pass converts `heads()` on a filtered range of commits into a special
`heads_range(roots, heads, [filter])` operation which can be evaluated more
efficiently because it is able to stop scanning ancestor commits whenever a head
is found.

Examples:

* `heads(x..y)` => `heads_range(x, y)`
* `heads(x..y & conflicts())` => `heads_range(x, y, conflicts())`
* `heads(::x)` => `heads_range(none(), x)`
* `heads(~::x)` => `heads_range(x, visible_heads_or_referenced())`
* `heads(filter(x))` => `heads_range(none(), visible_heads_or_referenced(), x)`

Since `::x` is equivalent to `::heads(x)`, this pass also inserts
`heads_range()` operations inside of `ancestors()` when possible:

* `::(x..y)` => `::heads_range(x, y)`
* `::(~mine())` => `::heads_range(none(), visible_heads_or_referenced(), ~mine())`

### 10. Fold ranges and differences

This pass eliminates negated expressions by folding them into ranges and
differences when possible. Negated expressions like `~x` are expensive to
evaluate because they will be converted into `all() ~ x`, meaning they require
iterating over every commit in the repo. Therefore, we want to eliminate as many
negations as possible.

Rules:

* `::x & ~::y` => `y..x`
* `~::x & ::y` => `x..y`
* `x & ~y` => `x ~ y`
* `~x & y` => `y ~ x` (if `y` isn't a filter)

### 11. Fold not in ancestors

This pass eliminates any remaining negated ancestors expressions by converting
them to ranges. The heads of the range are all visible heads and all referenced
commits in the revset.

Rules:

* `~::x` => `x..visible_heads_or_referenced()`

## Visibility Resolution

After optimization, the revset is lowered to a `ResolvedExpression` that the
`Index` implementation can evaluate directly without being aware of commit
visibility. The main differences between a revset expression and a
`ResolvedExpression` are:

* `all()` is converted to `::visible_heads_or_referenced()`.

* Negated expressions like `~x` are converted to
  `::visible_heads_or_referenced() ~ x`.

* Filters are represented by a separate `ResolvedPredicateExpression` type which
  can only appear in certain contexts. For instance, there is a separate
  `FilterWithin` operation to handle intersections with filters. Filters outside
  of intersections are handled as `::visible_heads_or_referenced() & filter(f)`.

* All visibility information is removed from the expression, so
  `visible_heads()` and `visible_heads_or_referenced()` are replaced by lists of
  commit IDs.

## Evaluation by Index

Finally, the `ResolvedExpression` is passed to the `Index` implementation to be
evaluated into a `Revset` value. There is a built-in default index
implementation, but `jj` can be extended with alternative implementations.

A `Revset` can then be converted into a lazy stream of commits in topological
order (with children before parents) or into a function that tests whether a
`CommitId` is included in the revset.
