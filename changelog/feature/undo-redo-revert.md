`jj undo` and `jj redo` now revert the operation (like `jj op revert`) instead
of restoring the repo to an earlier operation. The resulting repo state is the
same, but they no longer print a "Restored to operation" line.
