`jj gerrit upload` now uploads the ancestors of the given revisions that are
not on the remote instead of their mutable ancestors. It fails if any of them
are immutable, since they need to be rewritten to add a `Change-Id` trailer.
Revisions that are already on the remote are no longer rejected as immutable.
