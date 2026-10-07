# Treesets

Some commands take a revision only to get the tree (the file contents) of it.
For example, `jj diff --from/--to` compares two trees, and `jj restore --from`
copies file contents from a tree. These commands accept an expression in the
treeset language, which is an extension of the [revset language](revsets.md)
for selecting a tree.

A plain revset selects the tree of the single revision it resolves to. For
example, `jj diff --from main` compares the tree of the "main" bookmark to the
working copy. It's an error if the revset resolves to zero or several
revisions.

In addition, the functions below can be used at the top level of the
expression. They can't be used inside a revset (e.g. `tree_merge(x) | y` is
invalid). [Revset aliases](revsets.md#aliases) can expand to these functions.

## Functions

* `tree_merge(x)`: The merge of the trees of all revisions in the revset `x`.
  The revisions are merged in the order they would be listed by `jj log`. It's
  an error if `x` resolves to no revisions. For example, `jj diff --from
  'tree_merge(X-)' --to X` shows the same diff as `jj diff -r X`, also if `X` is
  a merge commit.

* `conflict_side(tree, n)`: Side `n` of the conflicts in `tree`, numbered from
  1. Conflicted paths get their content from side `n`. Other paths keep their
  content, so changes that were made on other sides and that didn't conflict
  are kept. It's an error if `tree` has fewer than `n` sides. Selecting side 1
  of a tree without conflicts returns the tree unchanged.

## Examples

Show the changes in a merge commit `X` compared to the merge of its parents:

```shell
jj diff --from 'tree_merge(X-)' --to X
```

Resolve all conflicts in the working-copy commit in favor of the first side:

```shell
jj restore --from 'conflict_side(@, 1)'
```

Show the changes in side 2 of the conflicts in the working-copy commit
compared to side 1:

```shell
jj diff --from 'conflict_side(@, 1)' --to 'conflict_side(@, 2)'
```
