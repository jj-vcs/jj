New `jj file move [-s REVSETS] ORIG_NAME NEW_NAME` command to move or rename a
file or directory in the given revisions (default: `@`) and all their
descendants. Descendants are reparented rather than rebased, so the move never
introduces conflicts.
