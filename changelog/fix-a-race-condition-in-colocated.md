---
type: fix
---
Fix a race condition in colocated workspaces where concurrent `jj git fetch`
and `jj` commands could import partially fetched refs, causing divergence.
[#7349](https://github.com/jj-vcs/jj/issues/7349)
