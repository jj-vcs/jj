# Independent Workspaces

Authors: [David Rieber](mailto:drieber@google.com)
Status: Under review

> [!NOTE] **Terminology**: This proposal introduces the term **independent
> workspace**. Other possible names are "parallel workspace" and "detached
> workspace". We felt "detached" is a loaded word in the Git world,
> so we decided against it. Similarly, the proposal needs to give a name to
> workspaces that are not independent, i.e. the classic workspaces. Some options
> are regular, dependent, normal, standard and global.

--------------------------------------------------------------------------------

## Background

jj workspaces can be useful when working on one commit if the user wants to
simultaneously look at the tree at another commit, for example to run a build or
test. However it is easy to run into divergence if the user makes changes in
both workspaces around the same time.

With some discipline this can be avoided. For example, say wsA and wsB are two
workspaces, if the user makes sure wsA@ and wsB@ (the working copy commits of
wsA and wsB) have no ancestor-descendant relationship, and avoids running any
command from workspace wsA that would rewrite wsB@ (or wsB@'s descendants), and
vice versa, then cross-workspace induced divergence can be avoided. This extends
to any number of workspaces, but of course the risk of accidentally running into
divergence grows with the number of workspaces.

Even if a user tries to be careful, some commands can easily cause problems. If
wsA@ and wsB@ are both descendants of a mutable commit C, running `jj fix` on
wsA can rewrite C, and thus introduce divergence. Another example: squashing
wsA@ into its parent may rewrite wsB@. One more example: at Google we have a jj
custom command to fetch the latest main from Google's monorepo and rebase the
user's draft commits on top of this new main (`jj sync --all`). This is similar
to `jj git fetch` followed by rebase onto the new main. Many users use the
`--all` flag (myself included). This frequently leads to trouble
[^merge-view-google].

Concurrent mutating operations (including snapshotting) lead to divergence in
the oplog (two or more opheads). When a jj command runs, if it finds multiple
opheads, it first issues a "reconcile divergent operations" transaction to unify
them and go back to a single ophead. Reconciliation can introduce commit
divergence. Interactive commands such as `jj describe`, `jj squash -i`, `jj
split -i` are often implicated. In Google's distributed environment users can
manipulate the repo from multiple machines and/or cloud-based editors, which
can result in concurrent operations.

Even if commit divergence is somehow avoided, oplog manipulation is very tricky
when working on multiple workspaces: `jj undo` will undo the current ophead,
which may not be the one the user (or agent) just created, but an unrelated
operation created in another workspace. Also, "reconcile divergent operations"
cannot be undone.

## Goals

We would like to extend jj's model to address these use-cases:

* Collaboration between two or more independent actors (users or AI agents), all
  working on the same repo

* Exploring multiple ways of evolving a repo's commits, in other words, allowing
  oplog to diverge arbitrarily in two workspaces associated with the same repo,
  as in parallel universes

* Being able to observe work in those parallel universes without causing
  unwanted interference, and being able to bring pieces of that work back
  together

Some applications include two users collaborating on a set of changes, without
stepping in each other's way and without having to resort to ad-hoc
collaboration protocols. Also, a user and an agent working at the same time, or
even many agents competing to produce the best solution to some problem (the
agents may also collaborate, and even occasionally look at each others work).
There are also interesting applications to `jj run`, where two or more
temporary independent workspaces could be used to rewrite parts of the commit
graph and the end result is later brought back to the main branch in a single
operation (so a single `jj undo` undoes the whole thing).

We propose an extension of jj's current repo/workspace/opheads model that will
provide a much richer framework for concurrent work and will enable new
features and capabilities to address the use-cases above. This extension should
provide good building blocks to enable new workflows, but should not be too
prescriptive. It should work with jj's default backends as well as custom
backends.

## Independent Workspaces

Every jj repo has an oplog. The opheads are the most recent entries in the oplog
(typically there is just one, but sometimes there are two or more opheads). jj
reconciles divergent opheads into a single ophead and this single operation
points to a View. The View in turn sets the stage for revset evaluation. Today
all workspaces in a jj repo share the (unique) oplog in that repo.

We propose introducing a new kind of workspace: **independent workspace**. Just
like regular workspaces, operations run in an independent workspace are stored
in the repo's unique global operation store. But unlike regular workspaces, each
independent workspace has its own opheads. Independent workspaces operate on
the same commit backend store as regular workspaces.

Every jj command runs in a specific workspace, typically determined by the
current working directory. A command run in a **regular** workspace starts by
reading the global repo opheads, performs reconciliation if necessary, executes
the command's logic, and if it publishes an operation, it appends it to the
global repo opheads (replacing the previous ophead).

A command run in an **independent** workspace operates on the
**workspace-specific** opheads. Note that in this proposal all workspaces,
independent or not, share the same commit store and operation/view store. Only
the opheads are separate.

A repository can have:

1.  Zero or more **regular workspaces** (standard workspaces as they exist
    today).
2.  Zero or more **independent workspaces**.

To create an independent workspace we will need some changes to the `jj
workspace add` command, for example:

```
jj workspace add [--independent] ...
```

Alternatively we could introduce a new `jj workspace add-independent` command.
More importantly we need to decide on the semantics and features of creating a
workspace, whether regular or independent. Say you are in workspace WX and run
the command to create a new independent workspace WI. WX may be regular or
dependent. We think it is best to NOT record the add-independent operation in
the oplog of WX. A record of the new independent workspace WI is created in the
(global) `WorkspaceStore`. This should include: the workspace name, the
workspace type (independent or regular), and the workspace filesystem path
(every jj command will need to determine which workspace it is running in, and
what type of workspace it is).

The filesystem layout will look like this:

```
~/myrepo/foo.txt
~/myrepo/.jj/working_copy/type
~/myrepo/.jj/working_copy/tree_state
~/myrepo/.jj/working_copy/checkout
~/myrepo/.jj/repo/op_store/operations/op123
~/myrepo/.jj/repo/op_store/operations/op456
~/myrepo/.jj/repo/op_store/operations/op789
~/myrepo/.jj/repo/op_store/views/...
~/myrepo/.jj/repo/index/...
~/myrepo/.jj/repo/workspace_store/index
~/myrepo/.jj/repo/op_heads/type
~/myrepo/.jj/repo/op_heads/heads/op123
~/myrepo/.jj/repo/op_heads/workspace_heads/my_independent_ws1/op456
~/myrepo/.jj/repo/op_heads/workspace_heads/my_other_independent_ws/op789
~/myrepo/.jj/repo/store/...
~/myrepo/.git/...

~/my_independent_ws1
~/my_independent_ws1/foo.txt
~/my_independent_ws1/bar.txt
~/my_independent_ws1/.jj/working_copy/type
~/my_independent_ws1/.jj/working_copy/tree_state
~/my_independent_ws1/.jj/working_copy/checkout
~/my_independent_ws1/.jj/repo

~/my_other_independent_ws
~/my_other_independent_ws/bar.txt
~/my_other_independent_ws/.jj/working_copy/type
~/my_other_independent_ws/.jj/working_copy/tree_state
~/my_other_independent_ws/.jj/working_copy/checkout
~/my_other_independent_ws/.jj/repo

~/some_regular_ws
~/some_regular_ws/baz.txt
~/some_regular_ws/.jj/working_copy/type
~/some_regular_ws/.jj/working_copy/tree_state
~/some_regular_ws/.jj/working_copy/checkout
~/some_regular_ws/.jj/repo

~/another_regular_ws
~/another_regular_ws/hello.rs
~/another_regular_ws/.jj/working_copy/type
~/another_regular_ws/.jj/working_copy/tree_state
~/another_regular_ws/.jj/working_copy/checkout
~/another_regular_ws/.jj/repo
```

Notice the `.jj/repo/op_heads/workspace_heads/<WS_NAME>` directory. That's where
the independent workspace opheads are stored (we will probably want to use some
identifier other than the workspace name in that path, maybe a hash of the
workspace name, and we will want to store the mapping in the `WorkspaceStore` so
we can handle workspace renames correctly).

The oplog of an independent workspace is initialized with the ophead
present in the workspace where the `jj workspace add --independent` command
runs. In other words, the oplog of an independent workspaces is a continuation
of the oplog where the workspace is created, and forks from there. Users can
create an independent workspace with an empty oplog with `jj workspace add
--independent --at-operation=000000000000`. More generally, users can use
`--at-operation=OP`.

By default the independent workspace starts with a `View` derived from the oplog
fork point, but optionally zero or more `-b` (`--branch`) revset args can be
specified to restrict the set of commits visible in the initial `View`.

The existing `--revision` and `--sparse-patterns` args to `jj workspace add`
apply to independent workspaces exactly like today.

Beyond what was discussed above, there is no parent-child relationship between
workspaces. Any workspace can be forgotten/removed at any point. The user can
create an independent workspace and run `jj undo` a bunch of times, even undoing
operations predating the oplog fork point: that's ok and does NOT affect the
oplog of other workspaces. The same applies to other oplog commands like `jj op
restore/revert/integrate`. We need to be careful in `jj op abandon` and `jj util
gc`: we must not break other workspaces when running those commands.

## Peeking over the fence

Say ws1r and ws2r are regular workspaces while ws3i and ws4i are independent
workspaces, with workspace roots ~/ws1r, ~/ws2r, ~/ws3i, and ~/ws4i
respectively.

If the user's cwd under ~/ws1r, `jj log` already shows the commit graph according
to the View of the global opheads, so revsets can be used to see the working
copy commits of both ws1r and ws2r and their descendants. Under this proposal
`jj log` will NOT show anything pertaining to ws3i or ws4i (although it may be
the case that any number of commits reachable from ws3i's or ws4i's opheads ARE
part of the revset). If the user's cwd is under ~/ws3i, `jj log` will show the
commit graph according to the View of ws3i's opheads, and only ws3i's opheads.

That should be the default behavior of all revset evaluation. However we believe
it is important for both humans and agents to have a way to "peek" over the
fence to inspect the state of any workspace. For this we take inspiration from
the `at_operation(OP, REVSET)` revset function: we propose to introduce a new
revset function `at_workspace(WORKSPACE_NAME, REVSET)`.

Just like any other revset function, `at_workspace` can be used inside a larger
revset expression. The basic idea is that the nested expression (the second
argument) is evaluated in the context of the View of the ophead of the named
workspace. The semantics are as follows:

*   If the named workspace is the same as the current workspace then the
    sub-expression is evaluated directly.

*   If the named workspace and the current workspace are both regular, the
    sub-expression is evaluated directly.

*   Otherwise we have a regular workspace peeking into an independent workspace,
    or an independent workspace peeking into a regular workspace, or an
    independent workspace peeking into another independent workspace. In all
    these cases we will use the `OpHeadsStore` to get the opheads of the named
    workspace and evaluate the sub-expression in that context.

Say we are in the last case above, with the command running in wsA and
evaluating a revset expression that contains `at_workspace(wsB, REVSET)`. It is
important that this should not introduce any side-effects or contention in wsB.
So `~/wsA$ jj log -r 'at_workspace(wsB, xyz)'` will snapshot ~/wsA if necessary,
but it will not snapshot or do anything to wsB.

## WorkspaceStore changes

Currently jj workspaces have only a workspace name and root path. We will add a
workspace type enum (Dependent or Independent) and APIs in `WorkspaceStore` to
store this attribute and to fetch it. `SimpleWorkspaceStore` will implement
those APIs, and custom workspace stores will have to implement them as well. It
is ok for a custom workspace store to reject creation of independent workspaces
if they choose to do so. These changes are fairly simple. All workspaces prior
to these changes will be treated as regular dependent workspaces.

A workspace can never switch type, in either direction. Note: currently
forgotten workspaces are automatically implicitly re-added when you run a
mutating jj operation in them. This needs care to avoid switching workspace
types. Recently the WorkspaceStore backend has been improved, so we can probably
remove this implicit auto-readd behavior.

## Keeping track of the current workspace

We need to know which workspace is current when a jj command runs. This
information is needed when reading/updating opheads, when evaluating revsets and
in a few other places. Here are some options:

Option 1: RepoLoader now has a `workspace_store: Arc<dyn WorkspaceStore>` field.
We can add `workspace_name` and `workspace_type` fields to it.

Option 2: We could instead add those fields to `ReadonlyRepo`.

Option 3: We could wire workspace name and type some other way, from cli to the
libraries that interact with OpHeadsStore and revset evaluation.

A prototype is built with option 1, and we are in the process of trying option2.

## Evaluating `at_workspace(WS, x)` revset expressions

Notice that when running

```
~/wsA$ jj log -r 'at_workspace(wsB, xyz)'
```

the revset is evaluated by a `RevsetExpressionEvaluator` pointing to a `Repo`
object created for **wsA**. The implementation of `at_workspace` will be similar
to `at_operation`: both bring into the context an operation (and corresponding
view) other than that of the "background" repo.

This introduces a challenge: if wsB's ophead is not an ancestor of the
background repo's ophead, some of its visible heads may be unknown to the
`Index` associated to the background repo (see
[reload_repo_at_operation](https://github.com/jj-vcs/jj/blob/bb9b8fac71fe23ce9f51ca2c6c2491d26bc68ef0/lib/src/revset.rs#L2628),
which is used by `fold_at_operation`). The `ResolvedRevsetExpression` is
evaluated
[here](https://github.com/jj-vcs/jj/blob/bb9b8fac71fe23ce9f51ca2c6c2491d26bc68ef0/lib/src/revset.rs#L702).

As you can see by the code pointer above `at_operation` suffers from this issue
already, but this is not a huge deal because users very rarely pass an operation
that is not an ancestor of the ophead. For independent workspaces the story is
quite different: `at_workspace` will almost always bring an unrelated operation,
that's the whole point of independent workspaces!
[PR/10203](https://github.com/jj-vcs/jj/pull/10203) was sent to fix this problem
for `at_operation(OP, x)` and that paves the way for the implementation of
`at_workspace` (the PR is currently under review). Google's custom Index backend
implementation is global. It covers every commit Google's jj-daemon has seen,
with the commits themselves stored in Google's commit cloud, so it does not
suffer from these issues.

## Implementation of `at_workspace` revset function

A new `AtWorkspace` variant will be added to the
`RevsetExpression<St: ExpressionState>` enum (and a new `type Workspace` in
trait `ExpressionState`).

`ExpressionStateFolder` will get a new `fold_at_workspace` method which will
first use the workspace store to determine the type of the named workspace, and
then will call `OpHeadsStore::get_op_heads(workspace_name, workspace_type)`. If
there are multiple opheads `fold_at_workspace` could create an unpublished
"union" operation (similar to the `merge_operations` logic, but simpler) and
pretend that is the workspace's ophead. Then `fold_at_workspace` will proceed
almost exactly as `fold_at_operation` does (see that method for details).

To resolve `@` somewhere in `x` inside `at_workspace(WS, x)` we need to resolve
it against the working copy commits of the `View` of the named workspace, not
the background workspace. To do this `fold_at_workspace` will load the repo at
the named workspace.

## OpHeadsStore changes

When looking up opheads or updating opheads, `OpHeadsStore` will need to know
which opheads to read/update: the repo-wide opheads (those shared by all regular
workspaces) versus opheads of an independent workspace. We could introduce new
opheads store methods for dealing with independent workspaces, but it is probably
best to add new arguments to the existing methods:

```
    async fn update_op_heads(
        &self,
        workspace_name: Option<&WorkspaceName>,
        old_ids: &[OperationId],
        new_id: &OperationId,
    ) -> Result<(), OpHeadsStoreError>;

    async fn get_op_heads(
        &self,
        workspace_name: Option<&WorkspaceName>,
    ) -> Result<Vec<OperationId>, OpHeadsStoreError>;
```

When reading the opheads of an independent workspace the caller must pass the
workspace name.

## Output of `jj log -r at_workspace(WS, x)`

The visibility of commits during revset evaluation should probably be extended
to track which commits are visible in the ophead of the background workspace,
versus those from other named workspaces that were brought into context via
`at_workspace` functions. Otherwise those commits would show up in the output
as `hidden`. Instead we want commits that are visible only via the named
workspace to have some other marker, perhaps `(wsname)`.

## Undo, restore, evolog and integrate

`jj op log` should naturally work with the opheads applicable to the workspace
where the user runs the command. The same is true of `undo`, `restore` and
`evolog`.

We should probably allow integrating operations across workspaces. When doing
that we can probably integrate it was a single operation that references to
foreign operation, so that a single undo reverses the whole thing.

# Future work

## Per-workspace ACLs

At Google we want to build per-workspace write ACLs. That is part of the
collaboration story (between users and/or agents). Workspace ACLs are not part
of this proposal, mainly because there does not seem to be a way to do it in jj
today. Looking at the sample directory layout above, say alice owns `myrepo` and
wants to collaborate with bob on some feature. alice creates
`my_independent_ws1` and wants to give write access to bob, but just to that
workspace. The obvious thing to do is to give bob write access to
`~/my_independent_ws1` in the filesystem. But this is not enough: jj operations
run by bob under `~/my_independent_ws1` need to ALSO write to
`~/myrepo/.jj/repo` (to the store|op_store|index|op_heads directories), and to
`~/myrepo/.git` (the working_copy store is not a problem).

At Google we believe this is not a major problem because Google's custom backends
persist objects in the distributed Google Commit Cloud. We believe other custom
backends, in combination with a jj-daemon and a commit cloud should be able to
implement per-workspace ACLs if they choose to do so.

## Bounded-staleness when fetching other workspace's opheads

Since one of the main goals of this proposal is to enable concurrent agentic
workflows, we believe it is ok and in fact maybe desirable to make the
cross-workspace revset evaluation use some form of cached, forward-moving,
possibly stale ophead data (of the workspace name mentioned in `at_workspace`).
This decoupling may be necessary if many independent workspaces are referenced,
for example if we extend `at_workspace` to apply to multiple workspaces:
`at_workspace(glob(ws-feature*), REVSET)`. At Google the jj-daemon working in
coordination with the Google Commit Cloud may help make this possible.

## `--at-workspace=WS` global command line arg

TODO: do we need a (global) `--at-workspace=WS`?

[^merge-view-google]: The process of merging opheads is subtly different at
    Google due to the design of Google's huge commit cloud index. See
    <https://github.com/jj-vcs/jj/blob/a497458e49e89a5559f32f3884e1d5294bd77a4f/lib/src/repo.rs#L1985>.
    The same issues may apply to other commit cloud implementations with huge
    stores. Having said that, this is not the core problem addressed by
    independent workspaces.
