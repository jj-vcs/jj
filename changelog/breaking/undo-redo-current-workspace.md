`jj undo` and `jj redo` now only undo/redo operations performed in the current
workspace, skipping operations from other workspaces. Since they revert the
operation (like `jj op revert`), changes made from other workspaces in the
meantime are kept.
