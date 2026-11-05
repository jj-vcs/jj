Concurrent working-copy checkout races (e.g. a snapshot from another process
while an editor is open) now produce a user-facing warning explaining that the
operation was recorded but the working copy stayed where the other process
left it, instead of an internal error.
[#9408](https://github.com/jj-vcs/jj/issues/9408)
