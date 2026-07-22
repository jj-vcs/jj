# The evolution log

LINK TO https://docs.jj-vcs.dev/latest/cli-reference/#jj-evolog
LINK TO FAQ: I accidentally changed files..


### Splitting commits using the evolution log

Let's say we are editing a commit for "featureA", and we forgot to run `jj
new` or `jj commit` before doing some work that belongs in a new commit:

```console
$ jj log
@  lnvvtrzo jjfan@example.org 2025-02-28 21:01:10 31a347e0
│  featureA
◆  zzzzzzzz root() 00000000
$ cat file  # Oh no, the work on "feature B" should be in a separate commit!
Done with feature A
Working on feature B
```

The goal is to restore change `lnvvtrzo` to its previous state and split the
difference into a new child commit.

#### Step 1: Find the commit ID for the "last good version"

If you pushed `lnvvtrzo` already, then the version you probably want to restore
is the version on the remote. For example, if the bookmark is `feature-a`, then
the commit you want to restore is `feature-a@origin`.

Otherwise, you can find [all the past versions of the working copy revision that
`jj` has saved][predecessors] by running `jj evolog`, perhaps with the `--patch`
option. The obsolete versions will be marked as "hidden" and will have the same
change ID, but will have different commit IDs. This represents the [change]
evolving over time.

[predecessors]:
  #jj-is-said-to-record-the-working-copy-after-jj-log-and-every-other-command-where-can-i-see-these-automatic-saves

For example, this is what the evolog might look like after you made two edits to
the same change:

```console
$ # Note the word "hidden", the commit IDs on the right,
$ # and the unchanging change ID on the left.
$ jj evolog
@  lnvvtrzo jjfan@example.org 2025-02-28 21:01:10 31a347e0
│  featureA
│  -- operation 3cb7392c092c snapshot working copy
○  lnvvtrzo/1 jjfan@example.org 2025-02-28 21:00:51 b8004ab8 (hidden)
│  featureA
│  -- operation 1280bfaec893 snapshot working copy
○  lnvvtrzo/2 jjfan@example.org 2025-02-28 20:50:05 e4d831d (hidden)
   (no description set)
   -- operation 0418a5aa94b5 snapshot working copy
```

Since commit `b800` is hidden, it is considered obsolete and `jj log` (without
arguments) will not show it. However, most `jj` operations work normally on
such commits if you refer to them by their commit ID. Hidden commits can also
be referred to by their change ID, but they require a
[change offset][glossary_change_offset] to distinguish them (e.g. `b800` can
also be referred to as `lnv/1`, as shown in the evolog).

To find out which of these versions is the last time before we started working
on feature B (the point where we should have created a new change, but failed to
do so), we can look at the actual changes between the `evolog` commits by
running `jj evolog --patch`:

```console
$ # When was the last saved point before we started working on feature B?
$ jj evolog --patch --git  # We use `--git` to make diffs clear without colors
@  lnvvtrzo jjfan@example.org 2025-02-28 21:01:10 31a347e0
│  featureA
│  -- operation 3cb7392c092c snapshot working copy
│  diff --git a/file b/file
│  index 2b455c4207..2a7e05a01a 100644
│  --- a/file
│  +++ b/file
│  @@ -1,1 +1,2 @@
│   Done with feature A
│  +Working on feature B
○  lnvvtrzo/1 jjfan@example.org 2025-02-28 21:00:51 b8004ab8 (hidden)
│  featureA
│  -- operation 1280bfaec893 snapshot working copy
│  diff --git a/file b/file
│  index cb61245109..2b455c4207
│  --- a/file
│  +++ b/file
│  @@ -1,1 +1,1 @@
│  -Working on feature A
│  +Done with feature A
○  lnvvtrzo/2 jjfan@example.org 2025-02-28 20:50:05 e4d831d (hidden)
   (no description set)
   -- operation 0418a5aa94b5 snapshot working copy
   diff --git a/file b/file
   index 0000000000..cb61245109
   --- /dev/null
   +++ b/file
   @@ 0,0 +1,1 @@
   +Working on feature A
```

In this example, the version of the change when we were actually done with
feature A is when we edited the file to say "Done with feature A". This state
was saved in the commit with ID `b80` (the second one in the list). The
following edit (commit `31a`) belongs in a new change.

#### Step 2: Create a new change on top of the original revision

The "featureA" change is currently at commit `31a`:

```console
$ jj log
@  lnvvtrzo jjfan@example.org 2025-02-28 21:01:10 31a347e0
│  featureA
◆  zzzzzzzz root() 00000000
```

We'd like to create a new "featureB" change with the contents of the current
commit `31a`, and we'd like the "featureA" change to be reverted to its former
state at commit `b80` (see step 1 above for how we found that commit ID).

First, we create a new empty child commit on top of `b80`:

```console
$ jj new b80 -m "featureB"
Working copy  (@) now at: pvnrkl 47171aa (empty) featureB
Parent commit (@-)      : lnvvtr/1 b8004ab (divergent) featureA
```

There are now two visible commits with change ID `lnvvtr` (commit `b8004ab`
and `31a347e0`), so we call these [divergent][glossary_divergence]. Similarly
to hidden commits, divergent commits also require a
[change offset][glossary_change_offset] when using the change ID to refer to
them, so you can see that `b8004ab` is still shown as `lnvvtr/1` in the output.
This temporary divergence is okay and will be resolved in the next steps.

[glossary_divergence]: glossary.md#divergent-change
[glossary_change_offset]: glossary.md#change-offset

Next, restore the contents of `31a347e0` into the working copy:

```console
$ jj restore --from 31a347e0
Working copy  (@) now at: pvnrkl 468104c featureB
Parent commit (@-)      : lnvvtr/1 b8004ea (divergent) featureA
$ cat file
Done with feature A
Working on feature B
```

#### Step 3: Move any bookmarks to the original revision

```console
$ jj bookmark move --from 31a347e0 --to b8004ea8
```

#### Step 4: Abandon the unwanted revision

```console
$ jj abandon 31a347e0
```

Now, we have achieved the exact state we desired:

```
$ jj log -p --git
@  pvnrklkn jjfan@example.org 2025-02-28 21:39:29 468104c2
│  featureB
│  diff --git a/file b/file
│  index 2b455c4207..2a7e05a01a 100644
│  --- a/file
│  +++ b/file
│  @@ -1,1 +1,2 @@
│   Done with feature A
│  +Working on feature B
○  lnvvtrzo jjfan@example.org 2025-02-28 21:00:51 b8004ab8
│  featureA
│  diff --git a/file b/file
│  new file mode 100644
│  index 0000000000..2b455c4207
│  --- /dev/null
│  +++ b/file
│  @@ -0,0 +1,1 @@
│  +Done with feature A
◆  zzzzzzzz root() 00000000
$ jj diff --from b80 --to @- # No output means these are identical
$ jj diff --from 31a --to @  # No output means these are identical
```


### Splitting commits using the evolution log

FROM #8482



Let's say we are editing a commit for "featureA", and we forgot to run `jj
new` or `jj commit` before doing some work that belongs in a new commit:

```console
$ jj log
@  lnvvtrzo jjfan@example.org 2025-02-28 21:01:10 31a347e0
│  featureA
◆  zzzzzzzz root() 00000000
$ cat file  # Oh no, the work on "feature B" should be in a separate commit!
Done with feature A
Working on feature B
```

The goal is to restore change `lnvvtrzo` to its previous state and split the
difference into a new child commit.

#### Step 1: Find the commit ID for the "last good version"

If you pushed `lnvvtrzo` already, then the version you probably want to restore
is the version on the remote. For example, if the bookmark is `feature-a`, then
the commit you want to restore is `feature-a@origin`.

Otherwise, you can find [all the past versions of the working copy revision that
`jj` has saved][predecessors] by running `jj evolog`, perhaps with the `--patch`
option. The obsolete versions will be marked as "hidden" and will have the same
change ID, but will have different commit IDs. This represents the [change]
evolving over time.

[predecessors]:
  #jj-is-said-to-record-the-working-copy-after-jj-log-and-every-other-command-where-can-i-see-these-automatic-saves

For example, this is what the evolog might look like after you made two edits to
the same change:

```console
$ # Note the word "hidden", the commit IDs on the right,
$ # and the unchanging change ID on the left.
$ jj evolog
@  lnvvtrzo jjfan@example.org 2025-02-28 21:01:10 31a347e0
│  featureA
│  -- operation 3cb7392c092c snapshot working copy
○  lnvvtrzo/1 jjfan@example.org 2025-02-28 21:00:51 b8004ab8 (hidden)
│  featureA
│  -- operation 1280bfaec893 snapshot working copy
○  lnvvtrzo/2 jjfan@example.org 2025-02-28 20:50:05 e4d831d (hidden)
   (no description set)
   -- operation 0418a5aa94b5 snapshot working copy
```

Since commit `b800` is hidden, it is considered obsolete and `jj log` (without
arguments) will not show it. However, most `jj` operations work normally on
such commits if you refer to them by their commit ID. Hidden commits can also
be referred to by their change ID, but they require a
[change offset][glossary_change_offset] to distinguish them (e.g. `b800` can
also be referred to as `lnv/1`, as shown in the evolog).

To find out which of these versions is the last time before we started working
on feature B (the point where we should have created a new change, but failed to
do so), we can look at the actual changes between the `evolog` commits by
running `jj evolog --patch`:

```console
$ # When was the last saved point before we started working on feature B?
$ jj evolog --patch --git  # We use `--git` to make diffs clear without colors
@  lnvvtrzo jjfan@example.org 2025-02-28 21:01:10 31a347e0
│  featureA
│  -- operation 3cb7392c092c snapshot working copy
│  diff --git a/file b/file
│  index 2b455c4207..2a7e05a01a 100644
│  --- a/file
│  +++ b/file
│  @@ -1,1 +1,2 @@
│   Done with feature A
│  +Working on feature B
○  lnvvtrzo/1 jjfan@example.org 2025-02-28 21:00:51 b8004ab8 (hidden)
│  featureA
│  -- operation 1280bfaec893 snapshot working copy
│  diff --git a/file b/file
│  index cb61245109..2b455c4207
│  --- a/file
│  +++ b/file
│  @@ -1,1 +1,1 @@
│  -Working on feature A
│  +Done with feature A
○  lnvvtrzo/2 jjfan@example.org 2025-02-28 20:50:05 e4d831d (hidden)
   (no description set)
   -- operation 0418a5aa94b5 snapshot working copy
   diff --git a/file b/file
   index 0000000000..cb61245109
   --- /dev/null
   +++ b/file
   @@ 0,0 +1,1 @@
   +Working on feature A
```

In this example, the version of the change when we were actually done with
feature A is when we edited the file to say "Done with feature A". This state
was saved in the commit with ID `b80` (the second one in the list). The
following edit (commit `31a`) belongs in a new change.

#### Step 2: Create a new revision for the recent changes

We'd like to create a new "featureB" change with the contents of the current
commit `31a`, and we'd like the "featureA" change to be reverted to its former
state at commit `b80` (see step 1 above for how we found that commit ID).

First, we create a new commit "featureB":

```console
$ jj new -m "featureB"
Working copy  (@) now at: xluvuyvk c632a66a (empty) featureB
Parent commit (@-)      : lnvvtrzo 31a347e0 featureA
```

For now the new commit is empty (has an empty diff), because it has the same
content as "featureA". This will change once we restore "featureA" to its
intended version.

#### Step 3: Restore the previous commit to its last good version

We now ask to restore the "featureA" commit, which is now the parent revision
`@-`, to its last known-good state `b80`; this will remove the work on feature
B from this commit. For this we use `jj restore`, but we must use the option
`--restore-descendants` to *preserve* the state of the descendants of
"featureA", that is our current commit "featureB", instead of rebasing the
change and thus removing the work on feature B from "featureB" as well.

```console
$ jj restore --from b80 --into @- --restore-descendants
Rebased 1 descendant commits (while preserving their content)
Working copy  (@) now at: xluvuyvk 4715d767 featureB
Parent commit (@-)      : lnvvtrzo b8004ab8 featureA
```

Thanks to the `--restore-descendants` option, the content of our current
commit "featureB" is unchanged, so it now has the expected diff:

```
$ jj show --git
[...]
diff --git a/file b/file
index 3cac3830aa..cb61245109 100644
--- a/file
+++ b/file
@@ -1,1 +1,2 @@
 Done with feature A
+Working on feature B
```

Note: instead of restoring "featureA" to a previous state wholesale, more
finer-grained to split the commit are available. For example you can try
`jj diffedit --from @-- --to @- --restore-descendants`, which provides an
interface similar to `jj split --interactive`, you choose the parts of the
diff to keep between "featureA" and its parent.
