# jj Independent Workspace Strawman Proposal

-   **Author:** drieber@google.com
-   **Status:** Draft

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
test. However it is easy to run into divergence when using multiple workspaces,
especially if two or more actors are involved: two humans, a human and an agent,
or two agents. This is unfortunate. It should be possible to have many agents
working in a repo at the same time, concurrently and independently, possibly at
the same time as the human owner of the repo.

Concurrent mutating operations (including snapshotting) lead to divergence in
the operation log (two or more op heads). When a jj command runs, if it finds
multiple op heads, it first issues a "reconcile divergent operations"
transaction to unify them and go back to a single op head. Reconciliation can
introduce commit divergence.

Even if commit divergence is somehow avoided, op log manipulation is very tricky
when working on multiple workspaces: jj undo will undo the current op head,
which may not be the one the user (or agent) just run, but an unrelated
operation created in another workspace.

We propose an extension of jj's current repo/workspace/opheads model that should
provide a much richer framework for concurrent work and should enable new
features and capabilities, especially in the realm of agent-based development.

This document is a strawman proposal; further design work remains to evaluate
feasibility, edge cases, and implementation details.

## Strawman Proposal: Independent Workspaces

Every jj repo has an operation log. The OpHeads are the most recent entries in
the op log (typically there is just one, but sometimes there are two or more
OpHeads). jj reconciles divergent OpHeads into a single OpHead and this single
operation head points to a View. The View in turn sets the stage for revset
evaluation. Today all workspaces in a jj repo share the (unique) oplog in that
repo.

We propose introducing a new kind of workspace: **independent workspace**. Just
like regular workspaces, operations run in an independent workspace are stored
in the repo's unique global operation store. But unlike regular workspaces, each
independent workspace has its own operation heads.

Every jj command runs in a specific workspace, typically determined by the
current working directory. A command run in a **regular** workspace starts by
reading the global repo OpHeads, performs reconciliation if necessary, executes
the command's logic, and if it publishes an operation, it appends it to the
global repo OpHeads (replacing the previous op head).

A command run in an **independent** workspace operates on the
**workspace-specific** OpHeads. Note that in this proposal all workspaces,
independent or not, share the same commit store and operation/view store. Only
the OpHeads are separate.

A repository can have:

1.  Zero or more **regular workspaces** (standard workspaces as they exist
    today).
2.  Zero or more **independent workspaces**.

To create an independent workspace we will need some changes to the
`jj workspace add` command, for example:

```
`jj workspace add [--independent] ...
```

Alternatively we could introduce a new `jj workspace add-independent` command.
More importantly we need to decide on the semantics and features of creating an
independent workspace. Say you are in workspace W1 and run the command to create
a new independent workspace W2. W1 may be regular or dependent. We think it is
best to NOT record the add-independent operation in the oplog of W1. A record of
the new independent workspace W2 is created in the (global) `WorkspaceStore`.
This should include: the workspace name, the workspace type (independent or
regular), and the workspace filesystem path (every jj command will need to
determine which workspace it is running in, and what type of workspace it is).

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
the independent workspace heads are stored (we will probably want to use some
identifier other than the workspace name in that path, maybe a hash of the
workspace name, and we will want to store the mapping in the `WorkspaceStore` so
we can handle workspace renames correctly).

The op log of an independent workspace could be a continuation of op head
present in the workspace where the `jj workspace add --independent` command was
run. If a user wants to create an independent workspace with an empty op log,
they can do so with a command like `jj workspace add --independent
--empty-oplog ...`. We can provide an optional revset argument to specify which
commits (evaluated in the context where `jj workspace add` is run) are visible
in the initial View of the independent workspace.

## Peeking over the fence

Say W1 and W2 are regular workspaces while W3 and W4 are independent workspaces,
with workspace roots ~/ws1, ~/ws2, ~/ws3, and ~/ws4 respectively.

If the user's cwd under ~/ws1, jj log already shows the commit graph according
to the View of the global op heads, so revsets can be used to see the working
copy commits of both W1 and W2 and their descendants. Under this proposal `jj
log` will NOT show anything pertaining to W3 and W4 (although it may be the case
that any number of commits reachable from W3's or W4's op heads ARE part of the
revset).

If the user's cwd is under ~/ws3, `jj log` will show the commit graph according
to the View of W3's op heads, and only W3's op heads.

That should be the default behavior of all revset evaluation. However we believe
it is important for both humans and agents to have a way to "peek" over the
fence to inspect the state of any workspace. For this we take inspiration from
the `at_operation(OP, REVSET)` revset function: we propose to introduce a new
revset function `at_workspace(WORKSPACE_NAME, REVSET)`.

Just like any other revset function, `at_workspace` can be used inside a larger
revset expression. The basic idea is that the nested expression (the second
argument) is evaluated in the context of the View of the op head of the named
workspace. The semantics are as follows:

*   If the named workspace is the same as the current workspace then the
    sub-expression is evaluated directly.
*   If the named workspace and the current workspace are both regular, the
    sub-expression is evaluated directly.
*   Otherwise we have a regular workspace peeking into an independent workspace,
    or an independent workspace peeking into a regular workspace, or an
    independent workspace peeking into another independent workspace. In all
    these cases we will use the `OpHeadsStore` to get the op heads of the named
    workspace and evaluate the sub-expression in that context.

Say we are in the last case above, with the command running in WSA and
evaluating a revset expression that contains `at_workspace(WSB, REVSET)`. It is
important that this should not introduce any side-effects or contention in WSB.
So `~/wsA$ jj log -r 'at_workspace(WSB, xyz)'` will snapshot ~/wsA if necessary,
but it will not snapshot or do anything to WSB.

Since one of the main goals of this proposal is to enable concurrent agentic
workflows, we believe it is ok and in fact probably desirable (maybe even
necessary) to make this cross-workspace revset evaluation use some form of
cached, forward-moving, possibly stale OpHead data (of the workspace name
mentioned in `at_workspace`, i.e. WSB in the example above).

The design and implementation of this revset function is one of the more
interesting parts of this proposal and will be explored further in a separate
document. Having said that, we believe there is still a lot of value in the
independent workspace proposal, even without that function.
