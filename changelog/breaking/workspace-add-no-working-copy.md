`jj workspace add` now fails when the current workspace doesn't have a working
copy. It used to fall back to creating the new workspace's working copy on top
of the root commit.
