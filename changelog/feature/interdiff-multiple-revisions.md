`jj interdiff`'s `--from` and `--to` options now accept revsets that resolve
to multiple revisions; each side is treated as if its revisions were squashed
into a single revision first.
[#8281](https://github.com/jj-vcs/jj/issues/8281)
