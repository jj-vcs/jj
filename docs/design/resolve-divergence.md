# Resolve-Divergence Command Design

Authors: [David Rieber](mailto:drieber@google.com)

**Summary:** This document is a high-level proposal for a new `jj resolve-divergence` command to help users resolve (or reduce) divergence.

## Objective

Divergence is confusing to our users. In many common scenarios we believe it should be possible to completely eliminate divergence with the help of this command. However not all theoretically possible cases of divergence are easy to solve. It is not a goal to solve every single case, but instead the most common ones.

Why a new command? why not automatically resolve divergence during every existing command? Some backends (e.g. the Google backend) use a cloud architecture. In such backends multiple clocks are involved. Because of this it is not always possible to establish a total chronological ordering of jj operations. Because of this we believe it is best to introduce a new command. The command allows us to obtain input from the user to guide the algorithm when we lack concrete rules or heuristics to make certain decisions.

The command may be unable to make any changes, or may only make partial progress. It should produce informative messages to summarize any changes made, and/or guide the user towards possible next steps. It is a soft goal to make the algorithm deterministic, mainly to make testing easier.

It is possible that in some cases the result will not be what the user expected. The user can of course undo the command.

## Divergent Changes

Changes are divergent when two or more visible commits have the same change-id. Divergence can occur:

* In the commit description (including tags)
* In the commit trees
* In the parents of the commit (commits A and B for change X have different parents)

The command will bail out if there is divergence in other commit metadata, e.g. user/committer name.

## Strawman Proposal

This section is a high-level conceptual description of the algorithm.

### ResolveDivergence

ResolveDivergence is the main library function behind the `jj resolve-divergence` command. The main steps in ResolveDivergence are:

1. Identify all divergent changes in the current view.
1. ChooseDivergentChange: choose one divergent change to resolve, say change $C$.
1. Find all visible commits for $C$. Lets call this set $DivergentCommits$.
1. Build a truncated evolog graph for $C$, starting with $DivergentCommits$ and including predecessors transitively, up to the common predecessor. Call this $EvologGraph$.
1. ChooseParents: choose the desired parent(s) for the new non-divergent commit.
1. GetCommitDescription: find the description we will use for the new commit.
1. ResolveDivergentChange: tries to eliminate divergence in change $C$.
1. RebaseDescendants
1. Persist changes

For now we will focus on the case where all commits in $DivergentCommits$ are mutable. TODO: decide what to do when there are immutable commits (maybe ignore them for now?).

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
1. For each comming $X$ in $remaining$: replace the $R->X$ edge and any edge from $next$ to $X$ with an edge from $R'$ to $X$, while keeping other predecessors of $X$. In other words, $X$ is now a successor of $R'$.
1. Similarly, for each successor $X$ of any commit in $next$, make $X$ a successor of $R'$, while keeping other predecessors of $X$ (if any).
1. Set $R$ = $R'$ and go back to step 2.

We repeat the steps above until we exit the loop with a single commit $R$ with the given description and parents. In the happy case the tree in commit $R$ does not have conflicts, but sometimes it will have conflicts. ResolveDivergentChange returns a summary of the changes made, including $R$ and $CommitsToHide$.

The algorithm describe above moves in the "forward" direction. A similar "backwards" algorithm (possibly simpler?) can be implemented. It is hard to say if one is better than the other, it probably depends heavily on the specific situation. It is quite possible we will need heuristics to decide when to give up, for example if conflicts with many sides are produced.

### MergeDivergentTrees

This takes $successors$ (a non-empty set of commits), and $base$ (a single commit). If $successors$ has just one commit, returns that commit. Say $successors$ is $K, L, M$:

* Applies the interdiff between $base$ and $M$ to $L$ to produce $L'$
* Applies the interdiff between $base$ and $L'$ to $K$ to produce $K'$
* Returns $K'$

### Rebasing Descendants and Persisting

The last step is to rebase all descendants of $CommitsToHide$ on top of the new commit for $C$ (commit $R$ above), persist the changes and record the operation in the op log. $CommitsToHide$ become hidden, $R$ becomes visible.

### Limiting the set of divergent commits to work on

In some cases the user may want to instruct jj to focus on specific commits in the divergent set. We should consider command-line arg(s) for doing that. It seems this should be fairly straightforward to do. The user provided commit ids must all be visible and all have the same change-id. We simply set $DivergentCommits$ to the user provided value. There may be very rare cases where other commits get picked up by the truncated evolog graph, but that's fine.

### Updating bookmarks

TODO: Write this.
