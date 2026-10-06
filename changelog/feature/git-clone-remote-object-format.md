`jj git clone` now uses the object format (SHA-1 or SHA-256) of the remote
repository, unless `--object-hash` is given. The format of an empty remote
can't be detected yet, so `git.object-hash` is still used in that case.
[#3813](https://github.com/jj-vcs/jj/issues/3813)
