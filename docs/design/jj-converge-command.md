# jj converge (aka resolve-divergence) Command Design

Authors: [David Rieber](mailto:drieber@google.com),
[Martin von Zweigbergk](mailto:martinvonz@google.com)

**Summary:** This document is a proposal for a new `jj converge` command to help
users resolve (or reduce) divergence. The command will use heuristics --and
sometimes will prompt the user for input-- to rewrite the N visible commits for
a given change with a single new commit, without introducing new divergence in
the process. `jj resolve-divergence` will be an alias for `jj converge`.

## Objective

A [divergent change] occurs when multiple [visible commits] have the same change
ID. Divergence is not a desirable state, but is not a bad state either. In this
regard divergence is similar to conflicts: the user can choose when and how to
deal with divergence. The [Handling divergent commits] guide has some useful
tips, but nevertheless divergence is confusing to our users. We can do better
than that. It should be possible to solve divergence (after the fact) in many
scenarios with the help of this command. Solving divergence means rewriting the
commit graph to end up with a single visible commit for the given change id. For
the purposes of this design doc we call this commit the *"solution"*.

The command should produce informative messages to summarize any changes made,
and will prompt for user input in some situations. The user may of course not
like the solution. `jj undo` can be used in that case.

[divergent change]: ../glossary.md#divergent-change
[visible commits]: ../glossary.md#visible-commits
[Handling divergent commits]: ../guides/divergence.md

## Divergent changes

Divergent commits (for the same change-id) can differ:

*   In their commit description
*   In their commit trees
*   In the parent(s) of the commits (commits *B/0* and *B/1* for change *B* have
    different parents)
*   In the commit author
*   It is also possible divergence involves two commits with different
    timestamps that are otherwise identical

As you read this design doc it is important to not confuse the
*predecessor/successor* relationship versus the *ancestor/descendant*
relationship.

### Some divergence scenarios

Divergence can be introduced in many ways. This document does not aim to explain
any/all of those scenarios accurately, this section is only meant to be rough
background material. Here are some examples:

*   In one terminal you type `jj describe` to edit a commit description and
    while the editor is open you take a coffee break, when you come back you
    open another terminal and do something that rewrites the commit (for example
    you modify a file and run `jj log`, causing a snapshot). When you save the
    new description `jj describe` completes and you end up with 2 visible
    commits with the same change id.

*   In general any interactive jj command (`jj split -i`, `jj squash -i`, etc)
    can lead to divergence in a similar way.

*   You can introduce divergence by making some hidden predecessor of your
    change visible again. There are many ways this could happen.

*   Divergence can happen when mutating two workspaces. For example, assume you
    have workspaces w1 and w2 with working copy commits *C1* and *C2*
    respectively, where *C2* is a child of *C1*. In w1 you run `jj git fetch`
    and then rebase the whole branch onto main. Go back to w2 (which is now
    stale), modify some file on disk and take a snapshot (e.g. run `jj log`).
    This introduces divergence.

*   When using the Git backend jj propagates change-id. The change-id is stored
    in the commit header, so after jj git fetch you can end up with a second
    commit with the same change-id.

*   There is a Google-specific jj upload command to upload a commit to Google's
    review/test/submit system, and there is an associated Google-specific
    command to "download" a change from that system back to your jj repo. This
    can introduce divergence very much like in the Git scenario.

*   At Google, snapshotting operations can happen concurrently on different
    machines (e.g. two terminals, or more commonly, a terminal and an IDE).
    Often times they end up snapshotting the same content. Google's backend does
    not hold locks while snapshotting because it's a distributed filesystem, so
    locking would be slow. This can introduce divergence.

## Notation

The document does not use realistic commit ids or change ids: most of the time
we refer to the divergent change-id as `B`, and its divergent commits as `B/0`,
`B/1` and so on. Usually `P` denotes a commit in `B`'s evolution graph that is a
common predecessor of the divergent commits, and has change-id `B`. Later on we
make more precise how to determine this `P`.

We write `A⁻` to denote the parent trees of commit `A`.

## Strawman proposal

At any point there can be zero, one or more divergent change-ids. The command
needs to first find all divergent commits, grouped by change-id. If there are
none there is nothing to do. If there are multiple divergent change-ids, the
command will ask the user to choose one (in the future we can add logic to
choose one automatically).

```rust
/* in jj_lib/src/converge.rs */

/// Maps change-ids to commits with that change-id.
pub type CommitsByChangeId = HashMap<ChangeId, HashMap<CommitId, Commit>>;

/// Evaluates the revset expression and returns those commits that are
/// divergent, in the sense that the expression matches two or more commits in
/// the result with the same change-id.
pub fn find_divergent_changes(
    repo: &Arc<ReadonlyRepo>,
    revset_expression: Arc<ResolvedRevsetExpression>,
) -> Result<CommitsByChangeId, RevsetEvaluationError> { ... }

/// Prompts the user to choose a change-id to converge, if there are multiple
/// divergent change-ids.
pub fn choose_change<'a>(
    converge_ui: Option<&dyn ConvergeUI>,
    divergent_changes: &'a CommitsByChangeId,
) -> Result<Option<&'a ChangeId>, ConvergeError> { ... }

/// Interface for user interactions during converge. This is only available
/// during interactive converge, to communicate with the user whenever input is
/// required.
pub trait ConvergeUI {
    /// Prompts the user to choose a change-id to converge.
    /// Converge returns immediately if this method returns None. This method is
    /// only invoked if there are multiple divergent change-ids.
    fn choose_change<'a>(
        &self,
        divergent_changes: &'a CommitsByChangeId,
    ) -> Result<Option<&'a ChangeId>, ConvergeError>;

    ... other methods, see below ...
}
```

Once a divergent change-id has been chosen, the command tries to create a
solution commit that solves that change-id, possibly prompting the user along
the way if necessary. The change-id of the solution commit is the change-id we
are converging. The divergent commits are recorded as predecessors of the
solution commit. Also, the solution commit "rewrites" the divergent commits:
this makes the divergent commits hidden and thus "converges" the change back to
a single visible commit. The descendants of the divergent commits get rebased on
top of the solution commit. Assuming there are no concurrent operations while jj
converge is running, it is guaranteed the algorithm will not introduce other
divergent changes, or increase divergence on any change. In the first version of
jj converge, the command will converge a single divergent change per invocation.
In the future we could explore converging more than one change per invocation.

It is desirable to be able to invoke the converge logic as a library, perhaps on
the server or from other jj commands. For this reason the implementation will be
mostly in jj-lib, and will abstract user interactions under a `ConvergeUI` trait
to make it possible to run the algorithm in non-interactive mode. The
implementation of `jj converge` under jj-cli will pass a concrete implementation
of the `ConvergeUI` trait to the `converge_change` function, a non-interactive
client would pass `NONE`.

```rust
/* in jj_lib/src/converge.rs */

/// Attempts to solve divergence in the given divergent commits.
/// Does not modify the repo.
pub async fn converge_change(
    repo: &Arc<ReadonlyRepo>,
    converge_ui: Option<&dyn ConvergeUI>,
    divergent_commits: &[Commit],
) -> Result<ConvergeResult<Box<ConvergeCommit>>, ConvergeError> { ... }

/// Encapsulates the solution to a problem, where the problem may be divergence
/// as a whole, or determining a specific aspect of the solution such
/// as the author, description, parents or tree of the converge commit.
pub enum ConvergeResult<T> {
    /// The proposed solution.
    Solution(T),
    /// Need user input to find a solution, but there is no ConvergeUI available.
    NeedUserInput(String),
    /// The user aborted the operation.
    Aborted,
}

/// The proposed solution for converging a change.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ConvergeCommit {
    /// The change-id of the change being converged.
    pub change_id: ChangeId,
    /// The divergent commits that are being converged.
    pub divergent_commit_ids: Vec<CommitId>,
    /// The proposed author.
    pub author: Signature,
    /// The proposed description.
    pub description: String,
    /// The proposed parents.
    pub parents: Vec<CommitId>,
    /// The proposed tree IDs.
    pub tree_ids: Merge<TreeId>,
    /// Conflict labels.
    pub conflict_labels: ConflictLabels,
}

pub trait ConvergeUI {
    ... other methods, see above ...

    /// Prompts the user to choose the author for the solution commit.
    fn choose_author(
        &self,
        divergent_commits: &[Commit],
        evolution_fork_point: &Commit,
    ) -> Result<Option<Signature>, ConvergeError>;
    /// Prompts the user to choose the parents for the solution commit.
    fn choose_parents(
        &self,
        divergent_commits: &[Commit],
    ) -> Result<Option<Vec<CommitId>>, ConvergeError>;
    /// Prompts the user to merge the description.
    fn merge_description(
        &self,
        divergent_commits: &[Commit],
        evolution_fork_point: &Commit,
    ) -> Result<Option<String>, ConvergeError>;
}

/// Applies the proposed solution to the repo.
pub fn apply_solution(
    solution: Box<ConvergeCommit>,
    repo_mut: &mut MutableRepo,
) -> Result<Commit, ConvergeError> { ... }
```

To produce the solution commit, we need to determine the solution's author,
description, parent commit(s) and MergedTree. We propose an algorithm that
attempts to automatically produce a value for each of those attributes based on
the evolution history of the divergent commits. The algorithm is applied
separately and independently to determine the author, description and parent
attributes. Once the parents are determined, a similar algorithm is used to
determine the solution's MergedTree. The automatic algorithm may fail to produce
a value for author and/or description and/or parents. In such cases:

*   If a `ConvergeUI` is available the corresponding method of the `ConvergeUI`
    is invoked to ask the user for help. For example, to determine the parents
    of the solution the `choose_parents` method is invoked: this prompts the
    user to choose one of the divergent commits, and uses the parents of the
    chosen commit as the parents of the solution.
*   If `converge_ui` is `NONE` then `converge_change` returns `NeedUserInput`.

### The TruncatedEvolutionGraph

We do not use the complete evolution graph, which could be quite large, as that
seems unnecessary. Instead we introduce the *"TruncatedEvolutionGraph for B/0,
B/1, ... , B/n"*, where *B/0, B/1, ... , B/n* are the commits we are converging.
As its name implies, this is a sub-graph of the complete evolution history. Its
nodes are commits for change-id *B* and the edges are from a commit to its
immediate successors. Here is the API:

```rust
/* in jj_lib/src/converge.rs */

/// The truncated evolution graph for a divergent change.
pub struct TruncatedEvolutionGraph {
    /// The commits in the change that are being converged (typically the
    /// visible & mutable commits for the given change-id).
    pub divergent_commit_ids: Vec<CommitId>,
    /// The evolution graph of the divergent commits, with edges X->Y if commit
    /// X is a predecessor of commit Y and both X and Y have the same
    /// divergent change-id. The start node is the evolution fork point.
    pub flow_graph: FlowGraph<CommitId>,
    /// The evolution entries for the commits in the graph.
    pub commits: HashMap<CommitId, CommitEvolutionEntry>,
}

impl TruncatedEvolutionGraph {
    /// Builds a truncated evolution graph for the given divergent commits,
    /// which are expected to all have the same change-id.
    pub fn new(
        repo: &ReadonlyRepo,
        divergent_commits: &[Commit],
        max_evolution_nodes: usize,
    ) -> Result<Self, ConvergeError> { ... }
    /// Returns the change-id of the commits in the graph.
    pub fn change_id(&self) -> &ChangeId { ... }
    /// Returns the commit for the given commit id.
    pub fn get_commit(&self, commit_id: &CommitId) -> Result<&Commit, ConvergeError> {
    /// Returns the evolution fork point.
    pub fn get_evolution_fork_point(&self) -> &Commit { ... }
}

/* in jj_lib/src/graph_dominators.rs */

/// A FlowGraph is a directed graph with a designated start node.
pub struct FlowGraph<N>
where
    N: Clone + Eq + Hash + PartialEq,
{
    /// The graph.
    pub graph: SimpleDirectedGraph<N>,
    /// The start node.
    pub start_node: N,
}

/// An immutable directed graph with nodes of type N.
pub struct SimpleDirectedGraph<N>
where
    N: Clone + Eq + Hash + PartialEq,
{
    /// The adjacency map of the graph.
    adj: IndexMap<N, IndexSet<N>>,
}
```

The graph is said to be truncated for the following reasons. First of all, any
commits in the evolution history with unrelated change-ids are ignored and not
included in the TruncatedEvolutionGraph. Also, while traversing the evolution
history to construct the TruncatedEvolutionGraph, if the traversal finds more
than N commits (for change-id B) it stops the traversal and pretends that the
"boundary" commits (at the time traversal stops) are all successors of the root
commit. And finally, the graph contains a single start node which we call the
"evolution fork point of B/0, B/1, ... , B/n" (more on this below); any commits
"older" in the evolution graph than the evolution fork point are not included in
the TruncatedEvolutionGraph.

TruncatedEvolutionGraph::new follows these steps:

1.  An adjacency list is built by traversing the operation log and associated
    View objects (using `jj_lib::evolution::walk_predecessors`), adding nodes
    and predecessor edges as needed. Nodes are added this way until there are no
    more edges pointing to predecessors with change-id `B` (or too many nodes
    have been traversed). This traversal keeps track of "initial nodes": these
    are commits in the evolution history that have no predecessors (for
    change-id `B`, they may have unrelated predecessors). Typically there will
    be a single initial node, but if there are multiple ones, we pretend the
    root is a common predecessor of all initial nodes.
1.  The adjacency list is then reversed: edge now point from a commit to their
    successors.
1.  A FlowGraph is created from the adjacency list, using the single initial
    node as the start node.
1.  To find the evolution fork point, the `find_closest_common_dominator` method
    is invoked on the FlowGraph, passing the divergent commit ids as the target
    set. This returns the unique commit id of the commit X that is:
    *   a dominator of all divergent commits (in the flow graph) and
    *   is "closest" than any other common dominator, i.e. if Y is another
        common dominator, then Y dominates X.
1.  Once we know the evolution fork point, the final FlowGraph is constructed
    from the FlowGraph produced in step 3 by removing nodes and edges for nodes
    "older" than the evolution fork point.

Please note the closest common dominator of a collection of nodes is NOT the
same concept as the least common ancestor of those nodes (although sometimes
they coincide). The least common ancestor is not uniquely defined: there may be
zero, one or more LCAs. Also, dominators are defined in any directed graph, even
graphs with cycles, LCA is only meaningful in DAGs. In a FlowGraph there is
always a unique closest common ancestor to any target set. The reason we choose
to define the evolution fork point as the closest common dominator is because
this way we the TruncatedEvolutionGraph captures the complete evolution history,
stemming from a single commit, an LCA may not have this property.

Node: the evolution history traversal keeps track of visited commits to avoid
infinite loops [^footnote-about-evolog-cycles] [^virtual-evolution-fork-point].

### The converge_attribute algorithm

As mentioned above, the converge library attempts to automatically produce an
author, description and parents for the solution commit independently of each
other with the help of the TruncatedEvolutionGraph. This section describe the
core `converge_attributes` function. The function is generic and is relatively
simple:

```rust
fn converge_attribute<T, VF>(
    divergent_commits: &[Commit],
    graph: &TruncatedEvolutionGraph,
    value_fn: VF,
) -> Result<Option<T>, ConvergeError>
where
    T: Eq + Hash + Clone,
    VF: Fn(&Commit) -> Result<T, ConvergeError>,
{
    let dominator_value = find_dominator_value(graph, &value_fn)?;
    let mut merge_builder = MergeBuilder::default();
    // ADD
    merge_builder.extend([dominator_value.clone()]);
    for divergent_commit in divergent_commits {
        let commit_value = value_fn(divergent_commit)?;
        // REMOVE, ADD
        merge_builder.extend([dominator_value.clone(), commit_value]);
    }
    let merge = merge_builder.build();
    Ok(merge.resolve_trivial(SameChange::Accept).cloned())
}
```

If the value_fn returns the same value V for all divergent commits then the
merge resolves trivially to V. Here is an example of how `converge_attribute` is
used to produce the solution's description:

```rust
fn converge_description(
    converge_ui: Option<&dyn ConvergeUI>,
    divergent_commits: &[Commit],
    graph: &TruncatedEvolutionGraph,
) -> Result<ConvergeResult<String>, ConvergeError> {
    let value_fn = |c: &Commit| Ok(c.description().to_string());
    if let Some(value) = converge_attribute(divergent_commits, graph, value_fn)? {
        return Ok(ConvergeResult::Solution(value));
    }
    let ui_chooser = |converge_ui: &dyn ConvergeUI| {
        converge_ui.merge_description(divergent_commits, graph.get_evolution_fork_point())
    };
    let Some(converge_ui) = converge_ui else {
        return Ok(ConvergeResult::NeedUserInput(format!(
            "cannot converge description automatically"
        )));
    };
    match ui_chooser(converge_ui)? {
        Some(value) => Ok(ConvergeResult::Solution(value)),
        None => Ok(ConvergeResult::Aborted),
    }
}
```

TODO: Describe `find_dominator_value`.
TODO: Describe how `converge_trees` works.

We now look at some examples to illustrate what the command should do, starting
with simple cases and moving on to more complex ones.

### Examples and expected behavior (with basic evolution graph)

The first few examples assume commits *B/0* and *B/1* are visible commits for
change *B*. First we assume *B/0* and *B/1* evolve directly from a common
predecessor commit *P*, which is now hidden (no longer visible). Later we look
at more complex evolution graphs. We assume *P*'s change id is also *B*.

```
Evolution graph for examples 1, 2, 3 and 4
------------------------------------------
Predecessors(B/0) = {P}
Predecessors(B/1) = {P}
P, B/0 and B/1 are all for change-id B, P is hidden.

B/0
|  B/1
| /
P
```

#### Example 1: two commits for change *B*, same parent

```console
$ jj log
B/0
|
| B/1
|/
A
```

In this simple case it is clear the solution should be a child of *A*:

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

Let's now consider two cases: when *P*'s parent is *A*, and when *P* has some
other parent. First, if *P*'s parent is *A* we have:

```console
$ jj log
B/0
| B/1
|/
| P (not visible)
|/
A
```

Here *P*, *B/0* and *B/1* are siblings. `converge_attribute` is used to
determine the description, parents, and author of the solution. Loosely speaking
the solution for each attribute can be expressed as the merge:

```
value_fn(P) + (value_fn(B/0) - value_fn(P)) + (value_fn(B/1) - value_fn(P))
```

The description is merged as a String value. If the description does not
trivially resolve, the user's merge tool will be invoked, with conflict markers.
If author does not trivially resolve, the user will be presented with the
options to choose from. Once that's all done we have our solution commit *B*.
All descendants of *B/0* and *B/1* are rebased onto *B*. The command records the
operation in the operation log with a new View where *B* is a visible commit
with *{B/0, B/1}* as predecessors. *B/0* and *B/1* become hidden commits.

Note that in some cases the solution may be identical to either *B/0* or *B/1*
(in all regards except the commit timestamp): we choose to create a new commit
*B* to make the evolution graph and op log more clearly show that jj converge
was invoked. Alternatively we could keep the matching commit instead of creating
a new commit (this could result in cycles in the evolog).

#### Example 2: two commits for change *B* with same parent (predecessor has a different parent)

Now lets consider the case where *P* has a different parent:

```console
$ jj log
B/0
|
| B/1
|/
A
|  P (not visible)
| /
X
```

In this case we first rebase *P* onto *A* (in-memory) to produce `P' = A + (P -
P⁻)`. This essentially reduces the problem to the previous case. We now produce
the solution as before: `B = P' + (B/0 - P') + (B/1 - P')`. Note that again the
parent of the solution is *A*.

#### Example 3: divergent commits with different parents

```console
$ jj log
B/0
|
|  B/1
|  /
| C
|/
A
```

In this case it is not immediately obvious which commit should be the parent of
the solution. Let's first consider the case where *P* is a child of *A*.

```console
$ jj log
B/0
|
|  B/1
|  /
| C
|/
|  P (not visible)
| /
A
```

We determine the parent(s) of the solution as follows:

```
parents = P⁻ + (B/0⁻ - P⁻) + (B/1⁻ - P⁻)
```

In this example the expression evaluates to `{A} + ({A} - {A}) + ({C} - {A}) =
{C}`. Since this expression resolves trivially to *{C}*, we use that as the
parents of the solution.

Note that this simple algorithm produces the desired output in example 1 and
example 2. In example 2, the expression looks like this:

```
parents = P⁻ + (B/0⁻ - P⁻) + (B/1⁻ - P⁻)
        = {X} + ({A} - {X}) + ({A} - {X})
        = {A} + ({A} - {X})
```

That expression resolves trivially to *{A}* when using SameChange::Accept.

#### Example 4: divergent commits with different parents, must prompt user to choose parents

If instead *P* is a child of some other commit *X*, the story is a bit
different:

```console
$ jj log
B/0
|
| B/1
| |
| C
|/
A
|  P (not visible)
| /
X
```

In this case parents will be

```
{X} + ({A} - {X}) + ({C} - {X}) = {A} + ({C} - {X})
```

Since this does not trivially resolve, the command prompts the user to select
the desired parents for the solution: either *{A}* or *{C}*.

Assume the user chooses *{C}*. The command then rebases (in memory) *B/0*, *B/1*
and *P* onto the chosen parents:

```
In-memory commits after rebasing B/0, B/1 and P on top of C (edges represent
parent/child relationship):

# B/0' = C + (B/0 - A)
# B/1' = C + (B/1 - C) = B/1
# P' = C + (P - X)

B/0'
|
|  B/1'
|/
|  P'
| /
C
```

As a result we obtain *B/0'*, *B/1'* and *P'*, and these are sibling commits. At
this point the command does a 3-way merge of `MergedTree` objects (in reality it
is enough to rebase the commit *trees*).

#### Example 5: more than 2 divergent commits

There can be more than 2 visible commits for a given change-id. We are assuming
here *B/0*, *B/1* and *B/2* are all direct successors of commit *P* (which is
invisible).

```console
$ jj log
B/0
|
| B/1
| |
| | B/2
| |/
|/
A
```

This is completely analogous to the first example, we simply have more terms on
all merges. The same thing applies to all previous examples, in all cases we can
deal with any number of divergent commits for change *B*.

### Examples and expected behavior (with arbitrary evolution graph)

So far we only considered simple cases where all divergent commits are direct
successors of a common predecessor *P*. Now we extend the ideas to arbitrary
evolution history.

#### Example 6: a two-level evolution graph

We continue by looking at a truncated evolution graph that is slightly more
complex than the basic 3-commit case. This will serve as motivation for the
general case. Here is our truncated evolution graph (remember the edges here
represent change evolution, not parent-child relations):

```
Truncated evolution graph. B/0, B/1 and Q may have other predecessors for
unrelated change-ids. P is the evolution fork point (it may have predecessors,
even for change-id B):

B/0     (description: "v3")
|
|  B/1  (description: "v2")
Q  /    (description: "v2")
| /
P       (description: "v1")
```

Commit *P* evolved into *Q* and *B/1*, and *Q* evolved into *B/0*. As before
*B/0* and *B/1* are visible, *P* and *Q* are not. Since both sides of the
evolution transitioned from "v1" to "v2", and then one side further transitioned
to "v3", it seems a good heuristic to take "v3" as the description of the
solution. Note that this observation would not be possible if the algorithm only
considered the leafs (*B/0*, *B/1*) and their evolution fork point (*P*).

Note: Why do we care about divergence producing two commits with the exact same
change? It may seem this would be a very uncommon scenario, however, as
mentioned in the last bullet point in the "Some divergence scenarios" section,
this is in fact fairly common at Google due to the distributed nature of
Google's backend filesystem.

In example 6, `find_dominator_value` returns "v2" (produced by Q and B/1),
therefore `converge_description` automatically picks "v3" as the solution.

Here is another example:

```
Truncated evolution graph:

B/0     ( foo.txt contents: "v3" )
|
|  B/1  ( foo.txt contents: "v2" )
Q  /    ( foo.txt contents: "v1" )
| /
P       ( foo.txt contents: "v1" )
```

In this case `find_dominator_value` returns "v1" (produced by P and Q) so
`converge_description` cannot automatically determine a value (because the merge
is "V1" + "V2" - "V1" + "V3" - "V1"). The ConvergeUI is used to ask the user to
merge the description (the command invokes the user's merge-tool with base "v1"
and sides "v2"/"v3").

### Edge cases when choosing the parents of the solution

The "adds" in the `Merge<Vec<CommitId>>` used to try to produce the solution
parents automatically are all the parents of the divergent commits. If the merge
does not resolve trivially, the ConvergeUI is used to ask the user to choose one
of the divergent commits and then we use the parent(s) of that commit as the
solution parents. Either way the parents of the solution are always the parents
of some divergent commit. In some corner cases this could suggest solution
parents that are problematic. Let's say the algorithm (or the user) chooses {P}
as the solution parents, where P is the parent of B/2, and furthermore let's
assume B/1 is an ancestor of P (this should be pretty rare):

```
Commit graph snippet: B/1 is an ancestor of P and P is a parent of B/2

B/2
 |
 P
 |
...
 |
B/1
```

In this situation we cannot apply the solution because we cannot rebase B/1 on
top of the solution commit S (because that would introduce a cycle in the commit
graph). To avoid this problem `converge_parents` only considers divergent
commits that are not descendants of other divergent commits. Since the commit
graph is a DAG, there is at least one such divergent commit.

## Multiple divergent change-ids

If there are multiple divergent change-ids, the command could prompt the user to
choose one, or apply heuristics to choose one programmatically. In the first
version it is OK to prompt the user.

If the command successfully resolves divergence in the first divergent
change-id, it could continue to process the next divergent change-id, and so on.
To avoid complexity the first implementation will only deal with one divergent
change per invocation.

### Rebasing descendants and persisting

The last step is to rebase all descendants of the divergent commits on top of
the new solution commit, persist the changes and record the operation in the op
log. The command will move local bookmarks pointing to any of the rewritten
divergent commits to point to the solution commit.

## Other edge cases

When the command starts it needs to find the divergent change-ids and their
corresponding visible commits. If the portion of the visible commit graph
leading up to immutable heads is too big, the command should error out.

There could be pathological cases where the evolution history is too long. When
building the truncated evolution graph, if we have traversed too many nodes (say
50) and we have not yet completed the traversal, the algorithm will not traverse
any more commits. We could simply error out, or we could use an incomplete
truncated evolution graph by adding a virtual evolution fork point. It is
probably best to error out.

## Open questions

*   Do we ever have divergence of committer? Is it safe to mess with committer?

## Alternatives considered

### Automatically resolving divergence

It would be nice if divergence was avoided in the first place, at least in some
cases, at the point where jj is about to introduce the second (or third or
fourth etc) visible commit for a given change id. This should be investigated
separately.

### Resolve divergence two commits at a time

The algorithm in this proposal should work when there are any number of
divergent commits (for a given change id). In practice we expect most often
there will be just 2 or perhaps a few divergent commits. We could design an
algorithm for just 2 commits, but we chose to think about the more general case.

### Only considering the evolution fork point and visible commits

Instead of using the value history graph, `converge_attribute` could do an n-way
merge with the author/commit/description of the evolution fork as the base of
the merge and the author/commit/description of the divergent commits.
Essentially the same thing could be done in `converge_trees` (after the trees
are properly rebased on top of the solution parents). This would be simpler, but
we prefer to use the value history graph because it allows for more cases to be
automatically merged and seems to better capture what we think the user will
likely want.

[^footnote-about-evolog-cycles]: It is unclear if the evolution history can
    contain cycles today, but there has been some
    discussion about `jj undo` possibly producing
    cycles. In any case, it is very easy to deal
    with that possibility, so we may as well handle
    it?
[^virtual-evolution-fork-point]: Today there should always be a single evolution
    fork point. However, we could handle cases
    where a change-id "emanates from multiple
    initial commits" by adding a single *virtual
    evolution fork point* commit with empty state:
    empty description, empty tree and empty author,
    and having the root commit as its parent, and
    treating it as a predecessor of all initial
    commits. Again, we probably don't need to worry
    about this, but it is good to know we could
    handle it.
