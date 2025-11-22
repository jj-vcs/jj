## Jujutsu's Core Tenets

Jujutsu's core tenets are:

 * Separation of logic and UI: It should be easy to create new UIs (CLIs, GUIs,
   TUIs, servers) without having to duplicate logic.
 * Easy-to-use APIs: It should be easy to create new commands. For example,
   each command should not have to worry about concurrency, working-copy state,
   and rebasing descendants of rewritten commits.
 * User-friendliness: Making the working copy a commit is simpler. This is
   how the project started.
 * The repo is the source of truth: Most commands should operate on the commit
   graph. The working copy is just one way of editing commits.
 * Pluggable storage: Must be easy to integrate with different commit storage,
   virtual file systems and more.
 * Git-interop: Git is everywhere. We need to have good interop to be adopted.
 * All operations must be able to scale to Google-scale repos (lots of commits,
   lots of files): Laziness is important, must avoid accessing data
   unnecessarily.
 * Having as few states as possible.
 * Make it incredibly hard to lose work in your repo.
 * Concurrent modifications to the repo should be safe.
 * Allow concurrent edits on any commit, pending or finished.
 * Make a "stacked diffs" workflow as easy as possible.
