When a working copy is recovered (for example by `jj workspace update-stale`
after its operation was lost), conflicted files that were written with longer
conflict markers are no longer snapshotted as resolved files containing the
markers. [#10796](https://github.com/jj-vcs/jj/issues/10796)
