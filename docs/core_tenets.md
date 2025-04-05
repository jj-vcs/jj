## Jujutsu's Core Tenets

Jujutsu's core tenets are:

 * User-friendliness: Making the  working copy a commit is simpler. This is
 how the project started.
 * The "repository", so the commit graph is the source of truth. The working
 copy is just one way of editing commits.
 * All operations must be able to scale to Google-scale repos (lots of commits
 , lots of files): Laziness is important, must avoid accessing data
 unnecessarily.
 * Having as few states as possible.
 * Make it incredibily hard to lose work in your repository.
 * Allow concurrent edits on any commit, pending or finished.
 * Make a "stacked diffs" workflow as easy as possible.
 * Git-interop: Git is everywhere. We need to have good interop to be adapted.
 * Pluggable storage: Must be easy to integrate with different commit storage,
 virtual file systems and more.
