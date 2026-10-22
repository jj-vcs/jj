`jj workspace add` now creates the new working-copy commit on top of the
closest immutable ancestor (`heads(::@ & immutable())`) instead of on the
parents of the current working-copy commit. Pass `-r @-` or set
`revsets.workspace-add = "@-"` for the old behavior.
