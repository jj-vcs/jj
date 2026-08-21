<!-- automatic-jj managed snippet: start -->
When a task edits or creates files in a directory that may use Jujutsu, invoke `@automatic-jj`.

`@automatic-jj` policy:
- inside a healthy jj workspace, use jj as the backup/history layer and do not create `.bak` files for normal edits
- if the directory is a Git repo without active jj, ask whether to run `jj git init --colocate`
- if the directory is not an active Git or jj workspace, ask whether to run `jj git init`
- if jj is unavailable, broken, or initialization is declined, fall back to the normal `.bak` backup rule
- default finalization is one jj task-finalization checkpoint when the task is complete
- only if the user explicitly asks, enable per-change checkpointing with `automatic-jj: commit-every-change`
- disable per-change checkpointing and return to default behavior with `automatic-jj: normal-mode`
<!-- automatic-jj managed snippet: end -->
