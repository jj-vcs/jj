# Jujutsu for Git experts

People who are proficient with Git often ask what benefit there is to using
Jujutsu. This page aims to explain why Jujutsu is a good fit for people already
well-versed using Git.

## Git is still available to use

Jujutsu repositories are colocated by default, which means they can be used by
both the `jj` and `git` commands.

If you encounter something you can't do with Jujutsu or is simply easier with
Git, you have the option of using `git` for that command, then switching back.

## Automatic rebasing makes for less toil

> I already know how to use `git rebase --interactive`.

Rebasing is automatic, so it's one fewer step to take. To amend a commit not at the head of a branch,
one might do the following with Git:

- `git add file1 file2`
- `git commit --fixup abc`
- `git rebase -i --autosquash`

Compare with the process for doing this with Jujutsu:

- `jj squash --into file1 file2`

TODO: Acknowledge `git rebase --update-refs`

## Undo is more powerful than searching the reflog

In Git, the reflog tracks the history of each ref (branch). You can see where a
ref has been over time, and restore it to a previous state with `git reset
--hard`.

Jujutsu's operation log tracks the complete history of the entire repository,
not just each ref individually.

If you make a mistake, undoing is as easy as `jj undo`. You don't even have to
look at Jujutsu's operation log to step back through history.

For more complicated situations, you can view the repository as it existed at
any point in history. All commands can accept `--at-operation OPERATION_ID` to
work as though the repository were at that point in time.

TODO: Show example

TODO: show `jj op log -p`, `jj op diff`

## Conflict resolution can be deferred

When rebasing in Git and a conflict is encountered, the rebase pauses for the
user to resolve the conflicts before continuing. There is no option to defer
this, you can only continue or abort.

After rebasing in Jujutsu, you can resolve conflicts at your leisure. You might
want to defer resolving conflicts because they're particularly difficult to
resolve and you want to think about it for a while. Or, you might be in the
middle of resolving conflicts and need to fix a critical bug. You can work on
the bug, then come back and finish resolving the conflicts.

In fact, if you're doing a sequence of rebases, you may not need to resolve the
conflicts manually at all. When a commit becomes conflicted, you can leave the
commit in its conflicted state and continue rebasing. When you're done, some of
those conflicts may have been able to be resolved automatically.

## `jj absorb`

TODO

## Ideas

- Email patch workflow
- Gerrit support built-in
