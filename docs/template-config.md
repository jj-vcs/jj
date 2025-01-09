# Template configuration

## Templates

This configuration section configures the default. These values can use [template
aliases](#template-aliases).

### Log

Configures the output of `jj log` when no `-T` is specified.

```toml
[templates]
log = "builtin_log_compact"
```

If you want to see the full description when you do `jj log` you can add this to
your config:

```toml
[templates]
log = "builtin_log_compact_full_description"
```

### Log nodes

Configures the symbol used to represent commits in the log. In this template,
`self` is an `Option<Commit>`, so the expression should first check for `!self`.

```toml
[templates]
log_node = '''
coalesce(
  if(!self, "🮀"),
  if(current_working_copy, "@"),
  if(root, "┴"),
  if(immutable, "●", "○"),
)
'''
```

### Operation log

Configures the output of `jj operation log` when no `-T` is specified. In this
template, `self` is an `Operation`.

```toml
[templates]
op_log = "builtin_op_log_compact"
```

### Operation log nodes

Configures the symbol used to represent operations in the operation log. In this
template, `self` is an `Operation`.

```toml
[templates]
op_log_node = 'if(current_operation, "@", "○")'
```

### Show

Configures the output of `jj show` when no `-T` is specified.

```toml
[templates]
show = "builtin_log_detailed"
```

### Description editor contents

The editor content of a commit description can be populated by the
`draft_commit_description` template.

```toml
[templates]
draft_commit_description = '''
concat(
  description,
  surround(
    "\nJJ: This commit contains the following changes:\n", "",
    indent("JJ:     ", diff.stat(72)),
  ),
  "\nJJ: ignore-rest\n",
  diff.git(),
)
'''
```

To configure the value of the description itself, use the setting
[`ui.default-description`](config.md#default-description).

## Template aliases

Template aliases

Some template aliases are built-in to Jujutsu and can be used to customize its output.

See the [templating language reference](templates.md) for more information about
the templating language and defining your own template aliases.

### Display of change and commit IDs

#### `format_short_change_id()`

Customizes how short change IDs are formatted. Default value: `format_short_id(id)`

```toml
[template-aliases]
# Uppercase change ids. `jj` treats change and commit ids as case-insensitive.
'format_short_change_id(id)' = 'format_short_id(id).upper()'
```

#### `format_short_commit_id()`

Customizes how short commit IDs are formatted. Default value: `format_short_id(id)`

```toml
[template-aliases]
# Uppercase change ids. `jj` treats change and commit ids as case-insensitive.
'format_short_commit_id(id)' = 'format_short_id(id).upper()'
```

#### `format_short_id()`

By default, both `format_short_change_id(id)` and `format_short_commit_id(id)`
both simply call `format_short_id(id)`, so this can be used to update both of
them together.

```toml
[template-aliases]
# Highlight unique prefix and show at least 12 characters (default)
'format_short_id(id)' = 'id.shortest(12)'
# Just the shortest possible unique prefix
'format_short_id(id)' = 'id.shortest()'
# Show unique prefix and the rest surrounded by brackets
'format_short_id(id)' = 'id.shortest(12).prefix() ++ "[" ++ id.shortest(12).rest() ++ "]"'
# Always show 12 characters
'format_short_id(id)' = 'id.short(12)'
```

### Display of timestamps

#### `format_timestamp()`

Controls how timestamps are displayed. Default value: `timestamp`

```toml
[template-aliases]
# Full timestamp in ISO 8601 format
'format_timestamp(timestamp)' = 'timestamp'
# Relative timestamp rendered as "x days/hours/seconds ago"
'format_timestamp(timestamp)' = 'timestamp.ago()'
```

`jj op log` defaults to relative timestamps. To use absolute timestamps, you
will need to modify the `format_time_range()` template alias.

#### `format_time_range()`

Controls how time ranges are displayed.

```toml
[template-aliases]
'format_time_range(time_range)' = 'time_range.start() ++ " - " ++ time_range.end()'
```

#### `commit_timestamp()`

Determines which timestamp is displayed for commits.

Commits have both an "author timestamp" and "committer timestamp". By default,
jj displays the committer timestamp, but can be changed to show the author
timestamp instead.

The function must return a timestamp because the return value will likely be
formatted with `format_timestamp()`.

Default value: `commit.committer().timestamp()`

```toml
[template-aliases]
'commit_timestamp(commit)' = 'commit.author().timestamp()'
```

### Display of commit authors

#### `format_short_signature()`

Controls how author and committer information are displayed in logs. Default
value: `signature.email()`

```toml
[template-aliases]
# Full email address (default)
'format_short_signature(signature)' = 'signature.email()'
# Both name and email address
'format_short_signature(signature)' = 'signature'
# Username part of the email address
'format_short_signature(signature)' = 'signature.username()'
```
