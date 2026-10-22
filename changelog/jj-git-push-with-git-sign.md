---
type: feature
---
`jj git push` with `git.sign-on-push` enabled now signs commits in a separate
transaction before the push, so commits stay signed if the push fails and a
new push doesn't have to sign them again.
