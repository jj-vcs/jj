# jj graft

Run `jj show yovwyxmm` and look at the change. We're going to be adding a command called "graft" to Jujutsu. Graft is designed to "re-root" some given set of commits at a new location in a tree. This is designed to help users manage specific workflows such as vendoring code into a directory.

Let's think about this abstractly. A -> B -> C describes a commit graph, where each node represents a snapshot of an (abstract) filesystem tree some point in time. Call the set of files described by a commit `Fs_N = FilesInSnapshot(N)` for some commit `N`. Let `D_N(F)` represent the data of a file `F` and some commit `N`, where `F` must be in the set `Fs`.

Consider the name of any given file `F`; as `Fs` is a set this name is always unique. We may rename this file in a manner that is *bidirectional* for example, the transformation `R(F) = "a/" ++ F` has an obvious inverse `R^{-1}(F) = strip_prefix("a/", F)` with the obvious semantics. Such bidirectional renames are what we call "translations of `F`" in the same manner that a translation of a polygon on the plane is a (reversible) change of coordinates.

Graft works on the observation that the expression `D_N(F)` is _invariant under the translation `R`_ -- in other words, the following holds:

    forall N. forall Fs_N. D_N(F) = D_N(R(F))

i.e. for every file in every commit, renaming the file way does not change the contents.

This means that rewriting any commit graph such as `A -> B -> C` is simple, given an appropriate choice of `R`:

  - Given the topologically sorted graph `T`,
  - For each node `N` in `T`,
  - For each file `F` in `Fs_N`,
  - Apply the translation `R(F)` to the name of the file
    - this is the new location of the file in a new commit `M`
    - `M` is a "translated" version of `N`

The result is a new topologically sorted graph `T'` that has every file moved into a new place. By using the inverse translation `R^{-1}` this transformation can be undone.

Having said all of that: this is the algorithm and purpose of 'jj graft': given some set of commits, apply a rename translation to each file path, producing a new set of commits. The user should be able to choose the destination of the files and the commits to duplicate

Let's consider the following fun example. I'm in a repository and I want to vendor some code underneath a directory. I could do something like the following: first, import all the commits from a repository

```
$ jj git remote add foo-upstream https://github.com/bazquxx/foo
$ jj git fetch --remote foo-upstream
```

This incorporates all the git objects from the upstream repository.

    NOTE: Because Jujutsu has a virtual root commit (like mercurial) these two distinct repositories live in the same commit graph with least-common ancestor commit `root()`

Next, we will "graft" the files from a subdirectory of `foo` onto a onto a path inside the repository:

```
jj graft tree \
  --from ::main@foo-upstream \
  --path src/foo \
  --onto ./vendor/foo \
  --destination XYZ # default '@-'
```

This would traverse the revset graph `G = ::main@foo-upstream` (i.e. all commits from the upstream repo), find all commits that modify `src/foo`, and then apply the translation `R(F) = "./vendor/foo/" ++ F` to each file within each matching snapshot. Note that there may be commits in the graph `G` that do not touch files underneath `foo/src`; in such a case this commit may be dropped and removed from the graph appropriately. The resulting new commits will be "rooted" and become children of `xyz`, the default being `@-` i.e. root the new commits onto the parent of the working copy.

Example: Let's say that the above revset `G` resolves to commits `A -> B -> C -> D` and the current commit our working copy is on top of is base `XYZ -> @` -- furthermore, let us say `C` does not modify any of the matching files. Then the result would be a commit graph `XYZ -> A' -> B' -> D'` with the resultant commits translated by `R` (note that the original working copy graph `XYZ -> @` would remain, i.e. `@` would also be a child of `XYZ` in this case, and so the working copy is therefore a sibling of `A'`

We need to implement this graft command. The above functionality is enough for a simple proof of concept. We do not need to support inverse translations `R^{-1}` at all. The above algorithm given the relative description should be straightforward: traverse the graph, filter out appropriate commits that touch the files, and then reroot the trees onto the new path. This algorithm, which would actually require creating new commits in the underlying store, is what we might call a "deep" copy of the graph; nodes are not shared in any way between the two trees, we simply always create new commits from the ether. This is also fine for a POC, but will not scale well to extremely large subtrees we want to graft.

Make sure that there are also tests for grafting from othe repos in this manner. You can find tons of CLI tests that do similar things; there's already a scaffold for you to fill out
