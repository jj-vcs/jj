The Git backend now keeps new objects in memory until the operation that
creates them is committed, and only then writes them to disk. Commands that
fail therefore usually leave no new objects behind in the Git repo, and
read-only commands no longer write objects (such as auto-merged parent trees)
to disk. To bound memory usage, objects are written to disk early once they
add up to 100 MiB. Operations that write many objects (such as large rebases)
now write a single pack file instead of many loose objects.
