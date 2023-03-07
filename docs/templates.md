# Templates

Jujutsu supports a functional language to customize output of commands.
The language consists of literals, keywords, operators, functions, and
methods.

A couple of `jj` commands accept a template via `-T`/`--template` option.

## Keywords

Keywords represent objects of different types; the types are described in
a follow-up section.

### Commit keywords

The following keywords can be used in `jj log`/`jj obslog` templates.

* `description: String`
* `change_id: ChangeId`
* `commit_id: CommitId`
* `parent_commit_ids: List<CommitId>`
* `author: Signature`
* `committer: Signature`
* `working_copies: String`: For multi-workspace repository, indicate
  working-copy commit as `<workspace name>@`.
* `current_working_copy: Boolean`: True for the working-copy commit of the
  current workspace.
* `branches: String`
* `tags: String`
* `git_refs: String`
* `git_head: String`
* `divergent: Boolean`: True if the change id corresponds to multiple visible
  commits.
* `conflict: Boolean`: True if the commit contains merge conflicts.
* `empty: Boolean`: True if the commit modifies no files.

### Operation keywords

The following keywords can be used in `jj op log` templates.

* `current_operation: Boolean`
* `description: String`
* `id: OperationId`
* `tags: String`
* `time: TimestampRange`
* `user: String`

## Operators

The following operators are supported.

* `x.f()`: Method call.
* `x ++ y`: Concatenate `x` and `y` templates.

## Global functions

The following functions are defined.

* `indent(prefix: Template, content: Template) -> Template`: Indent
  non-empty lines by the given `prefix`.
* `label(label: Template, content: Template) -> Template`: Apply label to
  the content. The `label` is evaluated as a space-separated string.
* `if(condition: Boolean, then: Template[, else: Template]) -> Template`:
  Conditionally evaluate `then`/`else` template content.
* `concat(content: Template...) -> Template`:
  Same as `content_1 ++ ... ++ content_n`.
* `separate(separator: Template, content: Template...) -> Template`:
  Insert separator between **non-empty** contents.

## Types

### Boolean type

No methods are defined.

### CommitId / ChangeId type

The following methods are defined.

* `.short([len: Integer]) -> String`
* `.shortest([min_len: Integer]) -> ShortestIdPrefix`: Shortest unique prefix.

### Integer type

No methods are defined.

### List type

No methods are defined.

### OperationId type

The following methods are defined.

* `.short([len: Integer]) -> String`

### ShortestIdPrefix type

The following methods are defined.

* `.prefix() -> String`
* `.rest() -> String`
* `.upper() -> ShortestIdPrefix`
* `.lower() -> ShortestIdPrefix`

### Signature type

The following methods are defined.

* `.name() -> String`
* `.email() -> String`
* `.username() -> String`
* `.timestamp() -> Timestamp`

### String type

A string can be implicitly converted to `Boolean`. The following methods are
defined.

* `.contains(needle: Template) -> Boolean`
* `.first_line() -> String`
* `.upper() -> String`
* `.lower() -> String`

### Template type

Any types can be implicitly converted to `Template`. No methods are defined.

### Timestamp type

The following methods are defined.

* `.ago() -> String`: Format as relative timestamp.

### TimestampRange type

The following methods are defined.

* `.start() -> Timestamp`
* `.end() -> Timestamp`
* `.duration() -> String`

## Configuration

[The default templates and aliases](../src/config/templates.toml) are defined
in the `[templates]` and `[template-aliases]` sections respectively.

New keywords and functions can be defined as aliases, by using any
combination of the predefined keywords/functions and other aliases.

For example:

```toml
[template-aliases]
'commit_change_ids' = '''
concat(
  format_field("Commit ID", commit_id),
  format_field("Change ID", commit_id),
)
'''
'format_field(key, value)' = 'key ++ ": " ++ value ++ "\n"'
```
