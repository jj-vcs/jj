---
title: Git command table
---

Note that all `jj` commands can be run on any commit (not just the working-copy
commit), but that's left out of the table to keep it simple. For example,
`jj squash -r <revision>` will move the diff from that revision into its
parent.

::yaml-table{file="git-command-table.yml"}
