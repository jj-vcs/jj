`jj git push` now checks commits in `immutable()` for empty descriptions,
missing author/committer, conflicts, and being private
(`git.private-commits`). Commits that are already on the remote being pushed
to are still not checked.
