# Git command table

Note that all `jj` commands can be run on any commit (not just the working-copy
commit), but that's left out of the table to keep it simple. For example,
`jj squash -r <revision>` will move the diff from that revision into its
parent.

<table>
  <thead>
    <tr>
      <th>Use case</th>
      <th>Git command</th>
      <th>Jujutsu command</th>
    </tr>
  </thead>
  <tbody>
    <tr>
      <td>Create a new repo</td>
      <td><code>git init</code></td>
      <td><code>jj git init [--colocate]</code></td>
    </tr>
    <tr>
      <td>Clone an existing repo</td>
      <td><code>git clone &lt;source&gt; &lt;destination&gt; [--origin &lt;remote name&gt;]</code></td>
      <td><code>jj git clone &lt;source&gt; &lt;destination&gt; [--remote &lt;remote name&gt;]</code> (there is no support
          for cloning non-Git repos yet)</td>
    </tr>
    <tr>
      <!-- TODO: Mention that you might need to do a `jj bookmark track branch@remote`
        -- to see the bookmark in `jj log`
        -->
      <td>Update the local repo with all bookmarks/branches from a remote</td>
      <td><code>git fetch [&lt;remote&gt;]</code></td>
      <td><code>jj git fetch [--remote &lt;remote&gt;]</code> (there is no
          support for fetching into non-Git repos yet)</td>
    </tr>
    <tr>
      <!-- TODO: This only affects tracked branches now -->
      <td>Update a remote repo with all bookmarks/branches from the local repo</td>
      <td><code>git push --all [&lt;remote&gt;]</code></td>
      <td><code>jj git push --all [--remote &lt;remote&gt;]</code> (there is no
          support for pushing from non-Git repos yet)</td>
    </tr>
    <tr>
      <td>Update a remote repo with a single bookmark from the local repo</td>
      <td><code>git push &lt;remote&gt; &lt;bookmark name&gt;</code></td>
      <td><code>jj git push --bookmark &lt;bookmark name&gt;
                [--remote &lt;remote&gt;]</code> (there is no support for
                pushing from non-Git repos yet)</td>
    </tr>
    <tr>
      <td>Add a remote target to the repo</td>
      <td><code>git remote add &lt;remote&gt; &lt;url&gt;</code></td>
      <td><code>jj git remote add &lt;remote&gt; &lt;url&gt;</code></td>
    </tr>
    <tr>
      <td>Show summary of current work and repo status</td>
      <td><code>git status</code></td>
      <td><code>jj st</code></td>
    </tr>
    <tr>
      <td>Show diff of the current change</td>
      <td><code>git diff HEAD</code></td>
      <td><code>jj diff</code></td>
    </tr>
    <tr>
      <td>Show diff of another change</td>
      <td><code>git diff &lt;revision&gt;^ &lt;revision&gt;</code></td>
      <td><code>jj diff -r &lt;revision&gt;</code></td>
    </tr>
    <tr>
      <td>Show diff from another change to the current change</td>
      <td><code>git diff &lt;revision&gt;</code></td>
      <td><code>jj diff --from &lt;revision&gt;</code></td>
    </tr>
    <tr>
      <td>Show diff from change A to change B</td>
      <td><code>git diff A B</code></td>
      <td><code>jj diff --from A --to B</code></td>
    </tr>
    <tr>
      <td>Show all the changes in A..B</td>
      <td><code>git diff A...B</code></td>
      <td><code>jj diff -r A..B</code></td>
    </tr>
    <tr>
      <td>Show description and diff of a change</td>
      <td><code>git show &lt;revision&gt;</code></td>
      <td><code>jj show &lt;revision&gt;</code></td>
    </tr>
    <tr>
      <td>Add a file to the current change</td>
      <td><code>touch filename; git add filename</code></td>
      <td><code>touch filename</code></td>
    </tr>
    <tr>
      <td>Remove a file from the current change</td>
      <td><code>git rm filename</code></td>
      <td><code>rm filename</code></td>
    </tr>
    <tr>
      <td>Modify a file in the current change</td>
      <td><code>echo stuff >> filename</code></td>
      <td><code>echo stuff >> filename</code></td>
    </tr>
    <tr>
      <td>Finish work on the current change and start a new change</td>
      <td><code>git commit -a</code></td>
      <td><code>jj commit</code></td>
    </tr>
    <tr>
      <td>See log of ancestors of the current commit</td>
      <td><code>git log --oneline --graph --decorate</code></td>
      <td><code>jj log -r ::@</code></td>
    </tr>
    <tr>
      <td>See log of all reachable commits</td>
      <td><code>git log --oneline --graph --decorate --branches</code></td>
      <td><code>jj log -r 'all()'</code> or <code>jj log -r ::</code></td>
    </tr>
    <tr>
      <td>Show log of commits not on the main branch</td>
      <td>(TODO)</td>
      <td><code>jj log</code></td>
    </tr>
    <tr>
      <td>List versioned files in the working copy</td>
      <td><code>git ls-files --cached</code></td>
      <td><code>jj file list</code></td>
    </tr>
    <tr>
      <td>Search among files versioned in the repository</td>
      <td><code>git grep foo</code></td>
      <td><code>grep foo $(jj file list)</code>, or <code>rg --no-require-git foo</code></td>
    </tr>
    <tr>
      <td>Abandon the current change and start a new change</td>
      <td><code>git reset --hard</code> (cannot be undone)</td>
      <td><code>jj abandon</code></td>
    </tr>
    <tr>
      <td>Make the current change empty</td>
      <td><code>git reset --hard</code> (same as abandoning a change since Git
          has no concept of a "change")</td>
      <td><code>jj restore</code></td>
    </tr>
    <tr>
      <td>Abandon the parent of the working copy, but keep its diff in the working copy</td>
      <td><code>git reset --soft HEAD~</code></td>
      <td><code>jj squash --from @-</code></td>
    </tr>
    <tr>
      <td>Discard working copy changes in some files</td>
      <td><code>git restore &lt;paths&gt;...</code> or <code>git checkout HEAD -- &lt;paths&gt;...</code></td>
      <td><code>jj restore &lt;paths&gt;...</code></td>
    </tr>
    <tr>
      <td>Edit description (commit message) of the current change</td>
      <td>Not supported</td>
      <td><code>jj describe</code></td>
    </tr>
    <tr>
      <td>Edit description (commit message) of the previous change</td>
      <td><code>git commit --amend --only</code></td>
      <td><code>jj describe @-</code></td>
    </tr>
    <tr>
      <td>Temporarily put away the current change</td>
      <td><code>git stash</code></td>
      <td><code>jj new @-</code> (the old working-copy commit remains as a sibling commit)<br />
          (the old working-copy commit X can be restored with <code>jj edit X</code>)</td>
    </tr>
    <tr>
      <td>Start working on a new change based on the &lt;main&gt; bookmark/branch</td>
      <td><code>git switch -c topic main</code> or
        <code>git checkout -b topic main</code> (may need to stash or commit
        first)</td>
      <td><code>jj new main</code></td>
    </tr>
    <tr>
      <td>Merge branch A into the current change</td>
      <td><code>git merge A</code></td>
      <td><code>jj new @ A</code></td>
    </tr>
    <tr>
      <td>Move bookmark/branch A onto bookmark/branch B</td>
      <td><code>git rebase B A</code>
          (may need to rebase other descendant branches separately)</td>
      <td><code>jj rebase -b A -d B</code></td>
    </tr>
    <tr>
      <td>Move change A and its descendants onto change B</td>
      <td><code>git rebase --onto B A^ &lt;some descendant bookmark&gt;</code>
          (may need to rebase other descendant bookmarks separately)</td>
      <td><code>jj rebase -s A -d B</code></td>
    </tr>
    <tr>
      <td>Reorder changes from A-B-C-D to A-C-B-D</td>
      <td><code>git rebase -i A</code></td>
      <td><code>jj rebase -r C --before B</code></td>
    </tr>
    <tr>
      <td>Move the diff in the current change into the parent change</td>
      <td><code>git commit --amend -a</code></td>
      <td><code>jj squash</code></td>
    </tr>
    <tr>
      <td>Interactively move part of the diff in the current change into the
          parent change</td>
      <td><code>git add -p; git commit --amend</code></td>
      <td><code>jj squash -i</code></td>
    </tr>
    <tr>
      <td>Move the diff in the working copy into an ancestor</td>
      <td><code>git commit --fixup=X; git rebase -i --autosquash X^</code></td>
      <td><code>jj squash --into X</code></td>
    </tr>
    <tr>
      <td>Interactively move part of the diff in an arbitrary change to another
          arbitrary change</td>
      <td>Not supported</td>
      <td><code>jj squash -i --from X --into Y</code></td>
    </tr>
    <tr>
      <td>Interactively split the changes in the working copy in two</td>
      <td><code>git commit -p</code></td>
      <td><code>jj split</code></td>
    </tr>
    <tr>
      <td>Interactively split an arbitrary change in two</td>
      <td>Not supported (can be emulated with the "edit" action in
          <code>git rebase -i</code>)</td>
      <td><code>jj split -r &lt;revision&gt;</code></td>
    </tr>
    <tr>
      <td>Interactively edit the diff in a given change</td>
      <td>Not supported (can be emulated with the "edit" action in
          <code>git rebase -i</code>)</td>
      <td><code>jj diffedit -r &lt;revision&gt;</code></td>
    </tr>
    <tr>
      <td>Resolve conflicts and continue interrupted operation</td>
      <td><code>echo resolved > filename; git add filename; git
          rebase/merge/cherry-pick --continue</code></td>
      <td><code>echo resolved > filename; jj squash</code> (operations
          don't get interrupted, so no need to continue)</td>
    </tr>
    <tr>
      <td>Create a copy of a commit on top of another commit</td>
      <td><code>git co &lt;destination&gt;; git cherry-pick &lt;source&gt;</code></td>
      <td><code>jj duplicate &lt;source&gt; -d &lt;destination&gt;</code></td>
    </tr>
    <tr>
      <td>Find the root of the working copy (or check if in a repo)</td>
      <td><code>git rev-parse --show-toplevel</code></td>
      <td><code>jj workspace root</code></td>
    </tr>
    <tr>
      <td>List bookmarks/branches</td>
      <td><code>git branch</code></td>
      <td><code>jj bookmark list</code> or <code>jj b l</code> for short</td>
    </tr>
    <tr>
      <td>Create a bookmark/branch</td>
      <td><code>git branch &lt;name&gt; &lt;revision&gt;</code></td>
      <td><code>jj bookmark create &lt;name&gt; -r &lt;revision&gt;</code></td>
    </tr>
    <tr>
      <td>Move a bookmark/branch forward</td>
      <td><code>git branch -f &lt;name&gt; &lt;revision&gt;</code></td>
      <td><code>jj bookmark move &lt;name&gt; --to &lt;revision&gt;</code>
        or <code>jj b m &lt;name&gt; --to &lt;revision&gt;</code> for short</td>
    </tr>
    <tr>
      <td>Move a bookmark/branch backward or sideways</td>
      <td><code>git branch -f &lt;name&gt; &lt;revision&gt;</code></td>
      <td><code>jj bookmark move &lt;name&gt; --to &lt;revision&gt; --allow-backwards</code></td>
    </tr>
    <tr>
      <td>Delete a bookmark/branch</td>
      <td><code>git branch --delete &lt;name&gt;</code></td>
      <td><code>jj bookmark delete &lt;name&gt; </code></td>
    </tr>
    <tr>
      <td>See log of operations performed on the repo</td>
      <td>Not supported</td>
      <td><code>jj op log</code></td>
    </tr>
    <tr>
      <td>Undo an earlier operation</td>
      <td>Not supported</td>
      <td><code>jj [op] undo &lt;operation ID&gt;</code>
          (<code>jj undo</code> is an alias for <code>jj op undo</code>)
      </td>
    </tr>
    <tr>
      <td>Create a commit that cancels out a previous commit</td>
      <td><code>git revert &lt;revision&gt;</code></td>
      <td><code>jj backout -r &lt;revision&gt;</code></td>
    </tr>
    <tr>
      <td>Show what revision and author last modified each line of a file</td>
      <td><code>git blame &lt;file&gt;</code></td>
      <td><code>jj file annotate &lt;path&gt;</code></td>
    </tr>
  </tbody>
</table>
