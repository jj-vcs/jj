Add `remotes.<name>.ref-push-max-batch-size` setting to push a maximum of n
refs in a single push and for more use sequential git push commands rather
than updating all at once. Useful for some remote restrictions, for example
for multiple history rewrites or limited number of CI runs per push, which
usually guard against `git push --all` or `git push --mirror` but get tripped
by normal jj usage.
