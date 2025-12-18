# Resolve-Divergence Command Design

Authors: [David Rieber](mailto:drieber@google.com)

**Summary:** This document is a high-level proposal for a new `jj resolve-divergence` command to help users resolve (or reduce) divergence. NOTE: the actual command name is TBD, `jj converge` is another candidate name.

## Objective

A [divergent change] occurs when multiple [visible commits] have the same change ID. Divergence is not a desirable state, but is not a "bad" state either. In this regard divergence is similar to conflicts: the user can choose when and how to deal with divergence. The [handling divergent commits] guide has some useful tips, but nevertheless divergence is confusing to our users. We can do better than that. It should be possible to "solve" divergence (after the fact) in many scenarios with the help of this command. Solving divergence means rewriting the commit graph to end up with a single visible commit for the given change id. For the purposes of this design doc we call this commit the "solution".

The command may be unable to make any changes, or may only make partial progress. It should produce informative messages to summarize any changes made, and/or guide the user towards possible next steps, and will prompt for user input in some situations. The user may of course not like the solution. `jj undo` can be used in that case.

[divergent change]: ../glossary.md#divergent-change
[visible commits]: ../glossary.md#visible-commits
[handling divergent commits]: ../guides/divergence.md

## Divergent changes

Divergence can occur:

* In the commit description (including tags)
* In the commit trees
* In the parent(s) of the commits (commits A and B for change X have different parents)
* In the commit author

It is also possible divergence involves two commits with different timestampts that are otherwise identical.

### Some divergence scenarios

Divergence can be introduced in many ways. Here are some examples:

* In one terminal you type `jj describe` to edit a commit description and while the editor is open you take a coffee break, when you come back you open another terminal and do something that rewrites the commit (for example you modify a file and run `jj log`, causing a snapshot). When you save the new description `jj describe` completes and you end up with 2 visible commits with the same change id.

* In general any interactive jj command (`jj split -i`, `jj squash -i`, etc) can lead to divergence.

* You can introduce divergence by making some "invisible" predecessor of your change visible again. There are many ways this could happen.

* Divergence can happen when mutating two workspaces:
  * In workspace1: A is the working copy commit
  * In workspace2: B (a child of A) is the working copy commit
  * In workspace2: you run `jj git fetch` and then rebase the whole branch to "main"
  * Workspace1 is now stale
  * Go back to workspace1: modify the files on disk and snapshot (e.g. run `jj log`)... this introduces divergence

* There is a Google-specific jj command to "upload" a commit to Google's "review/test/submit" system, and there is an associated Google-specific jj command to "download" a change from that system back to your jj repo. This can introduce divergence.
  * The same thing happens with Git because jj propagates change-id, after the PR is merged. With Git the change-id is stored in the commit header so after jj git fetch you can end up with a second commit with the same change-id.
  * two branches, one on top of the other, push to remote, remote rebases the bottom branch... both commits are visible: the rebased one and the bottom one ...!?!?!?!

### Examples and expected behavior (with basic evolution graph)

We will use `A⁻`  to denote the parent trees of commit `A`.

#### Example 1: two commits for change B, same parent

```console
$ jj log
B/0
| B/1
|/
A
```

In this simple case it is clear the solution should be a child of A:

```console
$ jj log
 B (solution)
 |
 | B/0 (not visible)
 |/
 | B/1 (not visible)
 | /
 A
```

Continuing with this example, assume further that B/0 and B/1 are direct successors (in the evolog sense) of commit P (later on we will look into more complex evolution graphs). Note that P's change id is also B. Lets now consider two cases: when P's parent is A, and when P has some other parent. First, if P's parent is A we have:

```console
$ jj log
B/0
| B/1
|/
| P (not visible)
|/
A
```

Here P, B/0 and B/1 are siblings. To find the solution, the command will independently merge in-memory the various pieces of a commit (description, contents, etc). Loosely speaking it will apply `B/0 + (B/1 - P)` to each piece. If the merged description does not trivially resolve, the user's merge tool will be invoked. If author does not trivially resolve, the user will be presented with the options to choose from. Once that's all done we have our solution commit B. All descendants of B/0 and B/1 are rebased onto B. jj will record the operation with a new View where B is a visible commit with predecessors {B/0, B/1}, and B/0 and B/1.

Note that it is possible B will identical to either B/0 or B/1, nevertheless a new commit B is created with a new timestamp. This makes the evolution graph and op log more clearly match what the operation does.

#### Example 2: two commits for change B with same parent (predecessor has a different parent)

Now lets consider the case where P has a different parent:

```console
$ jj log
B/0
| B/1
|/
A
|  P (not visible)
| /
X
```

In this case we first rebase P onto A to produce `P' = A + (P - P⁻)`. This essentially reduces the problem to the previous case. We now produce the solution `B = B/0 + (B/1 - P')`.

#### Example 3: divergent commits with different parents

```console
$ jj log
B/0
|  B/1
|  /
| C
|/
A
```

In this case it is not immediately obvious which commit should be the parent of the solution. Lets first consider the case where P --the direct common predecessor of B/0 and B/1-- is a child of A.

```console
$ jj log
B/0
|  B/1
|  /
| C
|/
|  P (not visible)
| /
A
```

In this case the command first needs to determine the parent(s) of the solution. The algorithm builds a `parents: Merge<Vec<CommitId>>` object as `parents = B/0⁻ + (B/1⁻ - P⁻)`. If that trivially resolves we have the desired parents; otherwise the command prompts the user to choose between the two options. In the example above parents resolves to `{C}`.

Once we have resolved parents, the algorithm rebases B/0, B/1 and P on top of the parents:

```
B/0' = C + (B/0 - A)
B/1' = C + (B/1 - C) = B/1
P' = C + (P - A)
```

Now we proceed as above.

#### Example 4: divergent commits with different parents, must prompt user to choose parents

```console
$ jj log
B/0
|  B/1
|  /
| C
|/
A
|  P (not visible)
| /
X
```

In this case the user must choose parents: either {A} or {C}. Say the user chooses {C}, then we proceed as above:

```
B/0' = C + (B/0 - A)
B/1' = C + (B/1 - C) = B/1
P' = C + (P - X)

B/0'
|  B/1
|/
|  P'
| /
C
```

#### Example 5: more than 2 divergent commits

This simply illustrates that divergence can involve >2 commits. We are assuming here B/0, B/1 and B/2 are all direct successors of commit P (which is invisible).

```console
$ jj log
B/0
| B/1
| | B/2
| |/
|/
A
```

This is completely analogous to the first example, we simply have more terms on all merges. The same thing applies to all previous examples, in all cases we can deal with any number of divergent commits for change B.

#### Example 6: divergent commits in a chain

```console
$ jj log
B/0
| B/1
| |
| B/2
| |
| C
|/
A
```

While determining the parents of the solution, we need to be careful to not include any of the divergent commits in the parent set.
TODO: this section probably needs more work. For one I am not sure if this ever happens, although I think in theory it is possible. Maybe the first version of this command should fail with some error: "unsupported blah blah blah". More precisely the command would error out if any divergent commit is an ancestor (possibly indirectly) of any other divergent commit.

#### Example 7: multiple divergent changes

```console
$ jj log
D/0
| D/1
|/
|
| B/0
| |  B/1
| | /
| C
|/
A
```

There are situations where there are two or more divergent *changes*. The command could apply some heuristic to choose one to focus on, or we could prompt the user to choose. In the first version it is probably best to prompt the user.

#### Example 8: intertwined divergent changes

This made up scenario is technically possible, but should be less common. Nevertheless it would be nice if the proposed command can handle something like this.

```console
$ jj log
D/0
| D/1
| |
| B/0
| |  B/1
| | /
| C
|/
A
```

It is easy to come up with even more convoluted graphs:

```console
$ jj log
D/0
|     B/1
B/0  /
|  D/1
| /
A
```

The algorithm should behave as described above. Say the user chooses to deal with B's commits first. It should be clear that D/0 and D/1 will be rewritten once B's solution is in place. The command could stop once B's solution is produced, or could proceed to deal with D all in one go. In the first version we should probably do the former.

### Examples and expected behavior (with arbitrary evolution graph)

So far we have only considered simple cases where all divergent commits are direct successors of a common predecessor P.

Lets say the command is working on divergent change B. The command first finds all divergent commits for B: B/0, B/1, ... , B/n. The command then traverses the op log to build a truncated evolution graph of change B, starting with B/0, B/1, ... , B/n and adding predecessor edges for each of them pointing to previous commits for change B (predecessors with other change ids are ignored). This algorithm stops with an error if a loop is detected (this should never/rarely happen). Otherwise the algorithm keeps going until a common predecessor is found. Note that upon success this will always produce a DAG with a single root (when looking at it in the "forward" sense, that is from older versions to newer versions).

#### Example 9: a two-level evolution graph

We start with a slightly more complex case, as motivation for the general case. Assume the following truncated evolution graph for change B:

```
B/0     ( foo.txt contents: "v3" )
|
|  B/1  ( foo.txt contents: "v2" )
Q  /    ( foo.txt contents: "v2" )
| /
P       ( foo.txt contents: "v1" )
```

For simplicity the diagram above shows the content of a single file foo.txt to illustrate the principle behind the proposed solution. The idea is to merge the content while traversing the truncated evolution graph one level at a time. First we merge `Q + (B/1 - P) = "v2" + ("v2" - "v1")`, since this trivially resolves foo.txt as V2, we conclude the user probably wants to keep V3. Note that if we only looked at the leafs (B/0 and B/1) and the common predecessor (P) we could not apply this heuristic. That is why we operate on the truncated evolution graph.

Note that if we instead kept merging terms we would get `Q + (B/1 - P) + (B/0 - Q) = B/0 + (B/1 - P) = "v3" + ("v2" - "v1")`, which again does not trivially resolve; this is why the algorithm will look out for levels that do trivially resolve.

With first-level simplification: we end up with "v3" (which seems better)
Without first-level simplification: we end up with "v3" + ("v2" - "v1")

Another example:

```
B/0     ( foo.txt contents: "v3" )
|
|  B/1  ( foo.txt contents: "v2" )
Q  /    ( foo.txt contents: "v1" )
| /
P       ( foo.txt contents: "v1" )
```

With first-level simplification: we end up with "v3"
Without first-level simplification: we end up with "v3" + ("v2" - "v1") .... which seems better
TODO: continue this...

Assume the truncated evolution graph looks something like this:

```
B/0  B/1   B/2
| \   |   /
|  \  |  /
|   \ | /
Q     R
|   /
| /
P
```

All commits are change-id B. The only visible ones are B/0, B/1 and B/2. The predecessors of B/0 are Q and R, the predecessor of B/1 is R, the predecessor of B/2 is R. P is the predecessor of both Q and R (remember: any of those commits could have additional predecessors for unrelated change-ids, we ignore those).

When looking for the solution, the algorithm will do a breadth-first traversal in topological order, starting at P and moving in the forward direction, one level at time. To make it more concrete, say we are looking for the solution's description. We do the following:

* First level: `merged_description = desc(Q) + (desc(R) - desc(P))`
  * If merged_description resolves trivially, we simplify it and continue to the second level: `merged_description = desc(B/0) + (desc(B/1) - simplified) + (desc(B/2) - simplified)`
  * Otherwise: `merged_description = desc(Q) + (desc(R) - desc(P)) + (desc(B/0) - ???) + (desc(B/1) - desc(R)) + (desc(B/2) - desc(R))`

## Strawman proposal

The command will focus on a single divergent change-id (lets call this change-id `C`) and tries to produce a single commit for `C` that replaces all divergent commits for `C`, i.e. a new successor, making the divergent commits hidden (*). The command will rebase all descendants of the divergent commits on top of the solution.

Loosely speaking, the solution should neatly encapsulate the changes produced in the divergent commits. As mentioned above, this is not always an easy task: we expect sometimes the user will like the result, but not always.

The algorithm needs to determine the parents, description, author and tree of the solution. To do that it will build an in-memory DAG of the truncated evolution history of change `C`, having the divergent commits as leafs, and rooted in their common ancestor, with edges pointing from a commit to its successors. We call this the DivergenceHistoryGraph. All commits in the DivergenceHistoryGraph are for change `C`, this graph will not contain commits for unrelated change-ids, even if those commits are part of the evolog of the divergent commits.

The algorithm first attempts to deduce the right set of parents for the solution. To deduce the parents we do a breadth-first traversal of the DivergenceHistoryGraph nodes, while merging the parents of the nodes, one level at a time. If this merge can be trivially resolved then we have the desired parents. Otherwise the command prompts the user to choose the parents.

Once the parents have been determined, the algorithm needs to deduce the tree. To do this it performs a second breadth-first traversal of the DivergenceHistoryGraph, this time producing a merge of tree instead. More on this later.

The same approach is used to deduce author and description. If the description-merge cannot be trivially resolved, the command opens the merge tool to let the user manually merge it.

(*) It is possible the solution is one of the divergent commits; in this case it remains visible of course.

### ResolveDivergence

ResolveDivergence is the main library function behind the `jj resolve-divergence` command. The main steps in ResolveDivergence are:

1. Identify all divergent changes in the current view. Return immediately if there are none.
1. ChooseDivergentChange: choose one divergent change to resolve, say change $C$.
1. Find all visible commits for $C$. Lets call this set $DivergentCommits$.
1. If there are more than 2 commits in $DivergentCommits$, prompt the user to choose 2 of them. Set $DivergentCommits$ to this set of 2 commits.
1. Build a truncated evolog graph for $C$, starting with $DivergentCommits$ and including predecessors transitively, up to the common predecessor. Call this $EvologGraph$.
1. ChooseParents: choose the desired parent(s) for the new non-divergent commit.
1. GetCommitDescription: find the description we will use for the new commit.
1. ResolveDivergentChange: tries to eliminate divergence in change $C$.
1. RebaseDescendants
1. Persist changes

If we successfully resolve divergence in change $C$, or even if we fail to do it or only partially resolve it, the command could choose another divergent change and work on that, until there is no divergence. To avoid complexity the first implementation will only deal with one divergent change per invocation.

In the future we could apply some heuristics to choose the change to work on in step 2 above. In the first implementation the user must specify the change-id to resolve in the command-line: `jj resolve-divergence -c <REVSET>`. The revset must evaluate to a single change-id. If this change-id is not divergent the command exits immediately.

$EvologGraph$ is built by walking the operation log and reading predecessor information from the View objects, starting with the commits in $DivergentCommits$, until we find the common predecessor. If $EvologGraph$ contains cycles (this is unlikely to happen, but there has been some discussion about `jj undo` possibly producing cycles in the operation history), we bail out with an error message. With that edge case out of the way, we end up with a DAG with a single root commit. All commits in $DivergentCommits$ are in this graph, but of course there may be other commits corresponding to hidden versions. Note that some commits in this graph are for change $C$, but there may be commits with other change-ids.

### ChooseParents

The command will allow the user to optionally specify `--parents <REVSET>` (or perhaps `--onto`?) on the command-line to explicitly choose the parent(s) for the new commit. If the user does not specify it, we apply some heuristics:

* If all commits in $DivergentCommits$ have exactly the same set of parents, then we use those parents.
* Otherwise if exactly one commit in $DivergentCommits$ has children, we use the parents of that commit.
* Otherwise we bail out with a message asking the user to rerun the command with `--parents <REVSET>` set.

There must be one or more parents. The parents must be visible commits and must not be descendants of any of the commits in $DivergentCommits$, or in $DivergentCommits$.

### GetCommitDescription

We will provide an optional command-line `--description-source <COMMIT_ID>` argument for specifying which commit-id to use as the source of the description. If specified this must be one of the commits in $DivergentCommits$. If `--description-source` is not specified and all commits in $DivergentCommits$ have identical description, we use that. Otherwise we bail out with a message asking the user to rerun the command with `--description-source <COMMIT_ID>`. We should probably also allow `-m <DESCRIPTION>` as an alternative solution.

### $ResolveDivergentChange(C, DivergentCommits, EvologGraph, Parents, Description, mutable Repo)$

ResolveDivergentChange operates EvologGraph, modifying it in place (in-memory). The steps of the main loop in ResolveDivergentChange are:

1. Starting at the root $R$ of $EvologGraph$.
1. If $R$ has no successors we exit this loop.
1. Put the successors of $R$ in one of two sets: those that have no predecessor other than $R$ are added to set $next$, the rest are added to $remaining$.
1. MergeDivergentTrees: merges the trees of the commits in $next$ with $R$ as base, producing $R'$ (see below). This may result in merge conflicts.
1. If $R$ is in $DivergentCommits$, add its commit-id to $CommitsToHide$ and remove it from $DivergentCommits$.
1. Add the intersection of $next$ and $DivergentCommits$ to $CommitsToHide$ and remove them from $DivergentCommits$.
1. Add $R'$ to $DivergentCommits$.
1. For each coming $X$ in $remaining$: replace the $R->X$ edge and any edge from $next$ to $X$ with an edge from $R'$ to $X$, while keeping other predecessors of $X$. In other words, $X$ is now a successor of $R'$.
1. Similarly, for each successor $X$ of any commit in $next$, make $X$ a successor of $R'$, while keeping other predecessors of $X$ (if any).
1. Set $R$ = $R'$ and go back to step 2.

We repeat the steps above until we exit the loop with a single commit $R$ with the given description and parents. In the happy case the tree in commit $R$ does not have conflicts, but sometimes it will have conflicts. ResolveDivergentChange returns a summary of the changes made, including $R$ and $CommitsToHide$.

The algorithm describe above moves in the "forward" direction. A similar "backwards" algorithm (possibly simpler?) can be implemented. It is hard to say if one is better than the other, it probably depends heavily on the specific situation. It is quite possible we will need heuristics to decide when to give up, for example if conflicts with many sides are produced.

### MergeDivergentTrees

This takes $successors$ (a set of two or more commits), and $base$ (a single commit). Say there are exactly two successors, $K and L$. We will use the notation $A^-$ to refer to the parent tree of commit $A$.

Say $successors$ is $K, L, M$:

* Applies the interdiff between $base$ and $M$ to $L$ to produce $L'$
* Applies the interdiff between $base$ and $L'$ to $K$ to produce $K'$
* Returns $K'$

### Rebasing descendants and persisting

The last step is to rebase all descendants of $CommitsToHide$ on top of the new commit for $C$ (commit $R$ above), persist the changes and record the operation in the op log. $CommitsToHide$ become hidden, $R$ becomes visible.

### Limiting the set of divergent commits to work on

In some cases the user may want to instruct jj to focus on specific commits in the divergent set. We should consider command-line arg(s) for doing that. It seems this should be fairly straightforward to do. The user provided commit ids must all be visible and all have the same change-id. We simply set $DivergentCommits$ to the user provided value. There may be very rare cases where other commits get picked up by the truncated evolog graph, but that's fine.

### Updating bookmarks

The command will move all local bookmarks pointing to any of the rewritten divergent commits, the bookmarks will then target the solution commit. Conflicting bookmarks also pointing unrelated commits need to be handled correctly.

## Open questions

* Do we ever have divergence of committer? Is it safe to mess with committer?

## Alternatives considered

### Automatically resolving divergence

It would be nice if divergence could be avoided in the first place, at least in some cases, at the point where jj is about to introduce the second (or third or fourth etc) visible commit for a given change id. In Google's environment this is a difficult ask because the distributed cloud environment makes some races unavoidable, and clock skew means it is sometimes impossible to assign a linear order to concurrent operations. It may be more realistic to instead resolve divergence during some (or most) jj commands, for example when snapshotting. Again, this should be investigated separately.

### Resolve divergence two commits at a time

The algorithm in this proposal should work when there are any number of divergent commits (for a given change id). In practice we expect most often there will be just 2 or perhaps a few divergent commits. We could design an algorithm for just 2 commits, but we chose to think about the more general case.
