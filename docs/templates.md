# Templates

Jujutsu supports a functional language to customize output of commands.
The language consists of literals, keywords, operators, functions, and
methods.

A couple of `jj` commands accept a template via `-T`/`--template` option.

## Keywords

Keywords represent objects of different types; the types are described in
a follow-up section. In addition to context-specific keywords, the top-level
object can be referenced as `self`.

### Commit keywords

In `jj log` templates, all 0-argument methods of [the `Commit`
type](#commit-type) are available as keywords. For example, `commit_id` is
equivalent to `self.commit_id()`.

### Operation keywords

In `jj op log` templates, all 0-argument methods of [the `Operation`
type](#operation-type) are available as keywords. For example,
`current_operation` is equivalent to `self.current_operation()`.

## Operators

The following operators are supported.

* `x.f()`: Method call.
* `-x`: Negate integer value.
* `!x`: Logical not.
* `x * y`, `x / y`, `x % y`: Multiplication/division/remainder. Operands must
  be `Integer`s.
* `x + y`, `x - y`: Addition/subtraction. Operands must be `Integer`s.
* `x >= y`, `x > y`, `x <= y`, `x < y`: Greater than or equal/greater than/
  lesser than or equal/lesser than. Operands must be `Integer`s.
* `x == y`, `x != y`: Equal/not equal. Operands must be either `Boolean`,
  `Integer`, or `String`.
* `x && y`: Logical and, short-circuiting.
* `x || y`: Logical or, short-circuiting.
* `x ++ y`: Concatenate `x` and `y` templates.

(listed in order of binding strengths)

## Global functions

The following functions are defined.

* `fill(width: Integer, content: Template) -> Template`: Fill lines at
  the given `width`.
* `indent(prefix: Template, content: Template) -> Template`: Indent
  non-empty lines by the given `prefix`.
* `pad_start(width: Integer, content: Template, [fill_char: Template])`: Pad (or
  right-justify) content by adding leading fill characters. The `content`
  shouldn't have newline character.
* `pad_end(width: Integer, content: Template, [fill_char: Template])`: Pad (or
  left-justify) content by adding trailing fill characters. The `content`
  shouldn't have newline character.
* `pad_centered(width: Integer, content: Template, [fill_char: Template])`: Pad
  content by adding both leading and trailing fill characters. If an odd number
  of fill characters are needed, the trailing fill will be one longer than the
  leading fill. The `content` shouldn't have newline characters.
* `truncate_start(width: Integer, content: Template, [ellipsis: Template])`:
  Truncate `content` by removing leading characters. The `content` shouldn't
  have newline character. If `ellipsis` is provided and `content` was truncated,
  prepend the `ellipsis` to the result.
* `truncate_end(width: Integer, content: Template, [ellipsis: Template])`:
  Truncate `content` by removing trailing characters. The `content` shouldn't
  have newline character. If `ellipsis` is provided and `content` was truncated,
  append the `ellipsis` to the result.
* `hash(content: Stringify) -> String`:
  Hash the input and return a hexadecimal string representation of the digest.
* `label(label: Stringify, content: Template) -> Template`: Apply a custom
  [color label](#color-labels) to the content. The `label` is evaluated as a
  space-separated string.
* `raw_escape_sequence(content: Template) -> Template`: Preserves any escape
  sequences in `content` (i.e., bypasses sanitization) and strips labels.
  Note: This function is intended for escape sequences and as such, its output
  is expected to be invisible / of no display width. Outputting content with
  nonzero display width may break wrapping, indentation etc.
* `stringify(content: Stringify) -> String`: Format `content` to string. This
  effectively removes color labels.
* `json(value: Serialize) -> String`: Serialize `value` in JSON format.
* `if(condition: Boolean, then: Template, [else: Template]) -> Template`:
  Conditionally evaluate `then`/`else` template content.
* `coalesce(content: Template...) -> Template`: Returns the first **non-empty**
  content.
* `concat(content: Template...) -> Template`:
  Same as `content_1 ++ ... ++ content_n`.
* `join(separator: Template, content: Template...) -> Template`: Insert
  `separator` between `content`s.
* `separate(separator: Template, content: Template...) -> Template`: Insert
  `separator` between **non-empty** `content`s.
* `surround(prefix: Template, suffix: Template, content: Template) -> Template`:
  Surround **non-empty** content with texts such as parentheses.
* `config(name: StringLiteral) -> ConfigValue`: Look up configuration value by `name`.

## Built-in Aliases

* `hyperlink(url, text)`: Creates a clickable hyperlink using [OSC8 escape sequences](https://github.com/Alhadis/OSC8-Adoption).
  The `text` will be displayed and clickable, linking to the given `url` in
  terminals that support OSC8 hyperlinks.

## Types

### `AnnotationLine` type

_Conversion: `Boolean`: no, `Serialize`: no, `Template`: no_

The following methods are defined.

* `.commit() -> Commit`: Commit responsible for changing the relevant line.
* `.content() -> Template`: Line content including newline character.
* `.line_number() -> Integer`: 1-based line number.
* `.original_line_number() -> Integer`: 1-based line number in the original commit.
* `.first_line_in_hunk() -> Boolean`: False when the directly preceding line
  references the same commit.

### `Boolean` type

_Conversion: `Boolean`: yes, `Serialize`: yes, `Template`: yes_

No methods are defined. Can be constructed with `false` or `true` literal.

### `Commit` type

_Conversion: `Boolean`: no, `Serialize`: yes, `Template`: no_

This type cannot be printed. The following methods are defined.

* `.description() -> String`
* `.trailers() -> List<Trailer>`: The trailers at the end of the commit
  description that are formatted as `<key>: <value>`. These are returned in the
  same order as they appear in the description, and there may be multiple
  `Trailer`s with the same key.
* `.change_id() -> ChangeId`
* `.commit_id() -> CommitId`
* `.parents() -> List<Commit>`
* `.author() -> Signature`
* `.committer() -> Signature`
* `.signature() -> Option<CryptographicSignature>`: Cryptographic signature if
  the commit was signed.
* `.mine() -> Boolean`: Commits where the author's email matches the email of
  the current user.
* `.working_copies() -> List<WorkspaceRef>`: For multi-workspace repositories,
  returns a list of workspace references for each workspace whose working-copy
  commit matches the current commit.
* `.current_working_copy() -> Boolean`: True for the working-copy commit of the
  current workspace.
* `.bookmarks() -> List<CommitRef>`: Local and remote bookmarks pointing to the
  commit. A tracked remote bookmark will be included only if its target is
  different from the local one.
* `.local_bookmarks() -> List<CommitRef>`: All local bookmarks pointing to the
  commit.
* `.remote_bookmarks() -> List<CommitRef>`: All remote bookmarks pointing to the
  commit.
* `.tags() -> List<CommitRef>`: Local and remote tags pointing to the commit. A
  tracked remote tag will be included only if its target is different from the
  local one.
* `.local_tags() -> List<CommitRef>`: All local tags pointing to the commit.
* `.remote_tags() -> List<CommitRef>`: All remote tags pointing to the commit.
* `.divergent() -> Boolean`: True if the commit's change ID corresponds to multiple
  visible commits.
* `.hidden() -> Boolean`: True if the commit is not visible (a.k.a. abandoned).
* `.change_offset() -> Option<Integer>`: The [change offset](glossary.md#change-offset)
  of this commit. May not be available for some commits.
* `.immutable() -> Boolean`: True if the commit is included in [the set of
  immutable commits](config.md#set-of-immutable-commits).
* `.contained_in(revset: StringLiteral) -> Boolean`: True if the commit is included in
  [the provided revset](revsets.md).
* `.conflict() -> Boolean`: True if the commit contains merge conflicts.
* `.empty() -> Boolean`: True if the commit modifies no files.
* `.diff([files: StringLiteral]) -> TreeDiff`: Changes from the parents within [the
  `files` expression](filesets.md). All files are compared by default, but it is
  likely to change in future version to respect the command line path arguments.
* `.files([files: StringLiteral]) -> List<TreeEntry>`: Files that exist in this commit,
  matching [the `files` expression](filesets.md). Use `.diff().files()` to list
  changed files.
* `.conflicted_files() -> List<TreeEntry>`: Conflicted files in this commit.
* `.root() -> Boolean`: True if the commit is the root commit.

### `CommitEvolutionEntry` type

_Conversion: `Boolean`: no, `Serialize`: yes, `Template`: no_

This type cannot be printed. The following methods are defined.

* `.commit() -> Commit`: New commit.
* `.operation() -> Operation`: Operation where the commit was created or
  rewritten.

### `ChangeId` type

_Conversion: `Boolean`: no, `Serialize`: yes, `Template`: yes_

The following methods are defined.

* `.normal_hex() -> String`: Normal hex representation (0-9a-f) instead of the
  canonical "reversed" (z-k) representation.
* `.short([len: Integer]) -> String`
* `.shortest([min_len: Integer]) -> ShortestIdPrefix`: Shortest unique prefix.

### `CommitId` type

_Conversion: `Boolean`: no, `Serialize`: yes, `Template`: yes_

The following methods are defined.

* `.short([len: Integer]) -> String`
* `.shortest([min_len: Integer]) -> ShortestIdPrefix`: Shortest unique prefix.

### `CommitRef` type

_Conversion: `Boolean`: no, `Serialize`: yes, `Template`: yes_

The following methods are defined.

* `.name() -> RefSymbol`: Local bookmark or tag name.
* `.remote() -> Option<RefSymbol>`: Remote name if this is a remote ref.
* `.present() -> Boolean`: True if the ref points to any commit.
* `.conflict() -> Boolean`: True if [the bookmark or tag is
  conflicted](bookmarks.md#conflicts).
* `.normal_target() -> Option<Commit>`: Target commit if the ref is not
  conflicted and points to a commit.
* `.removed_targets() -> List<Commit>`: Old target commits if conflicted.
* `.added_targets() -> List<Commit>`: New target commits. The list usually
  contains one "normal" target.
* `.tracked() -> Boolean`: True if the ref is tracked by a local ref. The local
  ref might have been deleted (but not pushed yet.)
* `.tracking_present() -> Boolean`: True if the ref is tracked by a local ref,
    and if the local ref points to any commit.
* `.tracking_ahead_count() -> SizeHint`: Number of commits ahead of the tracking
  local ref.
* `.tracking_behind_count() -> SizeHint`: Number of commits behind of the
  tracking local ref.
* `.synced() -> Boolean`: For a local bookmark, true if synced with all tracked
  remotes. For a remote bookmark, true if synced with the tracking local
  bookmark.

### `ConfigValue` type

_Conversion: `Boolean`: no, `Serialize`: yes, `Template`: yes_

This type can be printed in TOML syntax. The following methods are defined.

* `.as_boolean() -> Boolean`: Extract boolean.
* `.as_integer() -> Integer`: Extract integer.
* `.as_string() -> String`: Extract string. This does not convert non-string
  value (e.g. integer) to string.
* `.as_string_list() -> List<String>`: Extract list of strings.

### `CryptographicSignature` type

_Conversion: `Boolean`: no, `Serialize`: no, `Template`: no_

The following methods are defined.

* `.status() -> String`: The signature's status (`"good"`, `"bad"`, `"unknown"`,
  `"invalid"`).
* `.key() -> String`: The signature's key id representation (for GPG and SSH,
  this is the public key fingerprint).
* `.display() -> String`: The signature's display string (for GPG, this is the
  formatted primary user ID; for SSH, this is the principal).

!!! warning

    Calling any of `.status()`, `.key()`, or `.display()` is slow, as it incurs
    the performance cost of verifying the signature (for example shelling out
    to `gpg` or `ssh-keygen`). Though consecutive calls will be faster, because
    the backend caches the verification result.

!!! info

    As opposed to calling any of `.status()`, `.key()`, or `.display()`,
    checking for signature presence through boolean coercion is fast:
    ```
    if(commit.signature(), "commit has a signature", "commit is unsigned")
    ```

### `DiffStats` type

_Conversion: `Boolean`: no, `Serialize`: no, `Template`: yes_

This type can be printed as a histogram of the changes. The following methods
are defined.

* `.files() -> List<DiffStatEntry>`: Per-file stats for changed files.
* `.total_added() -> Integer`: Total number of insertions.
* `.total_removed() -> Integer`: Total number of deletions.

### `DiffStatEntry` type

_Conversion: `Boolean`: no, `Serialize`: no, `Template`: no_

This type holds the diff stats per file. The following methods are defined.

* `.bytes_delta() -> Integer`: The difference in size of the file, in bytes.
* `.lines_added() -> Integer`: Number of lines added.
* `.lines_removed() -> Integer`: Number of lines deleted.
* `.path() -> RepoPath`: Path to the entry. If the entry is a copy/rename, this
  points to the target (or right) entry.
* `.display_diff_path() -> String`: Format path for display, taking into account copy/rename information.
* `.status() -> String`: One of `"modified"`, `"added"`, `"removed"`, `"copied"`, or `"renamed"`.
* `.status_char() -> String`: One of `"M"` (modified), `"A"` (added), `"D"` (removed),
  `"C"` (copied), or `"R"` (renamed).

### `Email` type

_Conversion: `Boolean`: yes, `Serialize`: yes, `Template`: yes_

The email field of a signature may or may not look like an email address. It may
be empty, may not contain the symbol `@`, and could in principle contain
multiple `@`s.

The following methods are defined.

* `.local() -> String`: the part of the email before the first `@`, usually the
  username.
* `.domain() -> String`: the part of the email after the first `@` or the empty
  string.

### `Integer` type

_Conversion: `Boolean`: no, `Serialize`: yes, `Template`: yes_

No methods are defined.

### `List` type

_Conversion: `Boolean`: yes, `Serialize`: maybe, `Template`: maybe_

A list can be implicitly converted to `Boolean`. The following methods are
defined.

* `.len() -> Integer`: Number of elements in the list.
* `.join(separator: Template) -> Template`: Concatenate elements with
  the given `separator`.
* `.filter(|item| expression) -> List`: Filter list elements by predicate
  `expression`. Example: `description.lines().filter(|s| s.contains("#"))`
* `.map(|item| expression) -> ListTemplate`: Apply template `expression`
  to each element. Example: `parents.map(|c| c.commit_id().short())`
* `.any(|item| expression) -> Boolean`: Returns true if any element satisfies
  the predicate `expression`. Example: `parents.any(|c| c.description().contains("fix"))`
* `.all(|item| expression) -> Boolean`: Returns true if all elements satisfy
  the predicate `expression`. Example: `parents.all(|c| c.mine())`

### `List<Trailer>` type

The following methods are defined. See also the `List` type.

* `.contains_key(key: Stringify) -> Boolean`: True if the commit description
  contains at least one trailer with the key `key`.

### `ListTemplate` type

_Conversion: `Boolean`: no, `Serialize`: no, `Template`: yes_

The following methods are defined.

* `.join(separator: Template) -> Template`: Concatenate elements with
  the given `separator`.

### `Operation` type

_Conversion: `Boolean`: no, `Serialize`: yes, `Template`: no_

This type cannot be printed. The following methods are defined.

* `.current_operation() -> Boolean`
* `.description() -> String`
* `.id() -> OperationId`
* `.tags() -> String`
* `.time() -> TimestampRange`
* `.user() -> String`
* `.snapshot() -> Boolean`: True if the operation is a snapshot operation.
* `.root() -> Boolean`: True if the operation is the root operation.
* `.parents() -> List<Operation>`

### `OperationId` type

_Conversion: `Boolean`: no, `Serialize`: yes, `Template`: yes_

The following methods are defined.

* `.short([len: Integer]) -> String`

### `Option` type

_Conversion: `Boolean`: yes, `Serialize`: maybe, `Template`: maybe_

An option can be implicitly converted to `Boolean` denoting whether the
contained value is set. If set, all methods of the contained value can be
invoked. If not set, an error will be reported inline on method call.

On comparison between two optional values or optional and non-optional values,
unset value is not an error. Unset value is considered less than any set values.

### `RefSymbol` type

_Conversion: `Boolean`: no, `Serialize`: yes, `Template`: yes_

[A `String` type](#string-type), but is formatted as revset symbol by quoting
and escaping if necessary. Unlike strings, this cannot be implicitly converted
to `Boolean`.

### `RepoPath` type

_Conversion: `Boolean`: no, `Serialize`: yes, `Template`: yes_

A slash-separated path relative to the repository root. The following methods
are defined.

* `.absolute() -> String`: Format as absolute path using platform-native
  separator.
* `.display() -> String`: Format path for display. The formatted path uses
  platform-native separator, and is relative to the current working directory.
* `.parent() -> Option<RepoPath>`: Parent directory path.

### `Serialize` type

An expression that can be serialized in machine-readable format such as JSON.

!!! note

    Field names and value types in the serialized output are usually stable
    across jj versions, but the backward compatibility isn't guaranteed. If the
    underlying data model is updated, the serialized output may change.

### `ShortestIdPrefix` type

_Conversion: `Boolean`: no, `Serialize`: yes, `Template`: yes_

The following methods are defined.

* `.prefix() -> String`
* `.rest() -> String`
* `.upper() -> ShortestIdPrefix`
* `.lower() -> ShortestIdPrefix`

### `Signature` type

_Conversion: `Boolean`: no, `Serialize`: yes, `Template`: yes_

The following methods are defined.

* `.name() -> String`
* `.email() -> Email`
* `.timestamp() -> Timestamp`

### `SizeHint` type

_Conversion: `Boolean`: no, `Serialize`: yes, `Template`: no_

This type cannot be printed. The following methods are defined.

* `.lower() -> Integer`: Lower bound.
* `.upper() -> Option<Integer>`: Upper bound if known.
* `.exact() -> Option<Integer>`: Exact value if upper bound is known and it
  equals to the lower bound.
* `.zero() -> Boolean`: True if upper bound is known and is `0`. Equivalent to
  `.upper() == 0`.

### `String` type

_Conversion: `Boolean`: yes, `Serialize`: yes, `Template`: yes_

A string can be implicitly converted to `Boolean`. The following methods are
defined.

* `.len() -> Integer`: Length in UTF-8 bytes.
* `.contains(needle: Stringify) -> Boolean`: Whether the string contains the
  provided stringifiable value as a substring.
* `.match(needle: StringPattern) -> String`: Extracts
  the first matching part of the string for the given pattern.

  An empty string is returned if there is no match.
* `.replace(pattern: StringPattern, replacement: Stringify, [limit: Integer]) -> String`:
  Replace occurrences of the given `pattern` with the `replacement` string.

  By default, all occurrences are replaced. If `limit` is specified, at most
  that many occurrences are replaced.

  Supports capture groups in patterns using `$0` (entire match), `$1`, `$2` etc.
* `.first_line() -> String`
* `.lines() -> List<String>`: Split into lines excluding newline characters.
* `.split(separator: StringPattern, [limit: Integer]) -> List<String>`: Split into
  substrings by the given `separator` pattern. If `limit` is specified, it
  determines the maximum number of elements in the result, with the remainder
  of the string returned as the final element. A `limit` of 0 returns an empty list.
* `.upper() -> String`
* `.lower() -> String`
* `.starts_with(needle: Stringify) -> Boolean`
* `.ends_with(needle: Stringify) -> Boolean`
* `.remove_prefix(needle: Stringify) -> String`: Removes the passed prefix, if
  present.
* `.remove_suffix(needle: Stringify) -> String`: Removes the passed suffix, if
  present.
* `.trim() -> String`: Removes leading and trailing whitespace
* `.trim_start() -> String`: Removes leading whitespace
* `.trim_end() -> String`: Removes trailing whitespace
* `.substr(start: Integer, end: Integer) -> String`: Extract substring. The
  `start`/`end` indices should be specified in UTF-8 bytes. Indices are 0-based
  and `end` is exclusive. Negative values count from the end of the string,
  with `-1` being the last byte. If the `start` index is in the middle of a UTF-8
  codepoint, the codepoint is fully part of the result. If the `end` index is in
  the middle of a UTF-8 codepoint, the codepoint is not part of the result.
* `.escape_json() -> String`: Serializes the string in JSON format. This
  function is useful for making machine-readable templates. For example, you
  can use it in a template like `'{ "foo": ' ++ foo.escape_json() ++ ' }'` to
  return a JSON/JSONL.

### `StringLiteral` type

A string literal known at parse time. Unlike `Stringify`, this cannot be a
dynamic expression - it must be a literal value like `"main"` or `"format"`.

String literals must be surrounded by single or double quotes (`'` or `"`).
A double-quoted string literal supports the following escape sequences:

* `\"`: double quote
* `\\`: backslash
* `\t`: horizontal tab
* `\r`: carriage return
* `\n`: new line
* `\0`: null
* `\e`: escape (i.e., `\x1b`)
* `\xHH`: byte with hex value `HH`

Other escape sequences are not supported. Any UTF-8 characters are allowed
inside a string literal, with two exceptions: unescaped `"`-s and uses of `\`
that don't form a valid escape sequence.

A single-quoted string literal has no escape syntax. `'` can't be expressed
inside a single-quoted string literal.

String literals have their own type so that the value can be validated at parse
time. For example, `contained_in(revset)` requires a literal so the revset can
be parsed and checked before the template is evaluated.

### `Stringify` type

An expression that can be converted to a `String`.

Any types that can be converted to `Template` can also be `Stringify`. Unlike
`Template`, color labels are stripped.

### `StringPattern` type

_Conversion: `Boolean`: no, `Serialize`: no, `Template`: no_

These are the exact same as the [String pattern type] in revsets, except that
quotes are mandatory.

Literal strings may be used, which are interpreted as case-sensitive substring
matching.

Currently `StringPattern` values cannot be passed around as values and may
only occur directly in the call site they are used in.

[String pattern type]: revsets.md#string-patterns

### `Template` type

_Conversion: `Boolean`: no, `Serialize`: no, `Template`: yes_

Most types can be implicitly converted to `Template`. No methods are defined.

### `Timestamp` type

_Conversion: `Boolean`: no, `Serialize`: yes, `Template`: yes_

The following methods are defined.

* `.ago() -> String`: Format as relative timestamp.
* `.format(format: StringLiteral) -> String`: Format with [the specified strftime-like
  format string](https://docs.rs/chrono/latest/chrono/format/strftime/).
* `.utc() -> Timestamp`: Convert timestamp into UTC timezone.
* `.local() -> Timestamp`: Convert timestamp into local timezone.
* `.after(date: StringLiteral) -> Boolean`: True if the timestamp is exactly at or
  after the given date. Supported date formats are the same as the revset
  [Date pattern type].
* `.before(date: StringLiteral) -> Boolean`: True if the timestamp is before, but
  not including, the given date. Supported date formats are the same as the
  revset [Date pattern type].

[Date pattern type]: revsets.md#date-patterns

### `TimestampRange` type

_Conversion: `Boolean`: no, `Serialize`: yes, `Template`: yes_

The following methods are defined.

* `.start() -> Timestamp`
* `.end() -> Timestamp`
* `.duration() -> String`

### `Trailer` type

_Conversion: `Boolean`: no, `Serialize`: no, `Template`: yes_

The following methods are defined.

* `.key() -> String`
* `.value() -> String`

### `TreeDiff` type

_Conversion: `Boolean`: no, `Serialize`: no, `Template`: no_

This type cannot be printed. The following methods are defined.

* `.files() -> List<TreeDiffEntry>`: Changed files.
* `.color_words([context: Integer]) -> Template`: Format as a word-level diff
  with changes indicated only by color.
* `.git([context: Integer]) -> Template`: Format as a Git diff.
* `.stat([width: Integer]) -> DiffStats`: Calculate stats of changed lines.
* `.summary() -> Template`: Format as a list of status code and path pairs.

### `TreeDiffEntry` type

_Conversion: `Boolean`: no, `Serialize`: no, `Template`: no_

This type cannot be printed. The following methods are defined.

* `.path() -> RepoPath`: Path to the entry. If the entry is a copy/rename, this
  points to the target (or right) entry.
* `.display_diff_path() -> String`: Format path for display, taking into account copy/rename information.
* `.status() -> String`: One of `"modified"`, `"added"`, `"removed"`,
  `"copied"`, or `"renamed"`.
* `.source() -> TreeEntry`: The source (or left) entry.
* `.target() -> TreeEntry`: The target (or right) entry.

### `TreeEntry` type

_Conversion: `Boolean`: no, `Serialize`: no, `Template`: no_

This type cannot be printed. The following methods are defined.

* `.path() -> RepoPath`: Path to the entry.
* `.conflict() -> Boolean`: True if the entry is a merge conflict.
* `.conflict_side_count() -> Integer`: Number of sides in the merge conflict (1 if not
  conflicted, 2 or more for multi-way merges).
* `.file_type() -> String`: One of `"file"`, `"symlink"`, `"tree"`,
  `"git-submodule"`, or `"conflict"`.
* `.executable() -> Boolean`: True if the entry is an executable file.

### `WorkspaceRef` type

_Conversion: `Boolean`: no, `Serialize`: yes, `Template`: yes_

The following methods are defined.

* `.name() -> RefSymbol`: Returns the workspace name as a symbol.
* `.target() -> Commit`: Returns the working-copy commit of this workspace.

## Color labels

You can [customize the output colors][config-colors] by using color labels. `jj`
adds some labels automatically; they can also be added manually.

Template fragments are usually **automatically** labeled with the command name,
the context (or the top-level object), and the method names. For example, the
following template is labeled as `op_log operation id short` automatically:

```sh
jj op log -T 'self.id().short()'
```

The exact names of such labels are often straightforward, but are not currently
documented. You can discover the actual label names used with the
`--color=debug` option, e.g.

```sh
jj op log -T 'self.id().short()' --color=debug
```

Additionally, you can **manually** insert arbitrary labels using the
`label(label, content)` function. For example,

```sh
jj op log -T '"ID: " ++ self.id().short().substr(0, 1) ++ label("id short",  "<redacted>")'
```

will print "ID:" in the default style, and the string `<redacted>` in the same
style as the first character of the id. It would also be fine to use an
arbitrary template instead of the string `"<redacted>"`, possibly including
nested invocations of `label()`.

You are free to use custom label names as well. This will only have a visible
effect if you also [customize their colors][config-colors] explicitly.

[config-colors]: config.md#custom-colors-and-styles

## Configuration

The default templates and aliases() are defined in the `[templates]` and
`[template-aliases]` sections of the config respectively. The exact definitions
can be seen in the [`cli/src/config/templates.toml`][1] file in jj's source
tree.

[1]: https://github.com/jj-vcs/jj/blob/main/cli/src/config/templates.toml

<!--- TODO: Find a way to embed the default config files in the docs -->

New keywords and functions can be defined as aliases, by using any
combination of the predefined keywords/functions and other aliases.

Alias functions can be overloaded by the number of parameters. However, builtin
functions will be shadowed by name, and can't co-exist with aliases.

For example:

```toml
[template-aliases]
'commit_change_ids' = '''
concat(
  format_field("Commit ID", commit_id),
  format_field("Change ID", change_id),
)
'''
'format_field(key, value)' = 'key ++ ": " ++ value ++ "\n"'
```

## Examples

Get short commit IDs of the working-copy parents:

```sh
jj log --no-graph -r @ -T 'parents.map(|c| c.commit_id().short()).join(",")'
```

Show machine-readable list of full commit and change IDs:

```sh
jj log --no-graph -T 'commit_id ++ " " ++ change_id ++ "\n"'
```

Print the description of the current commit, defaulting to `(no description set)`:

```sh
jj log -r @ --no-graph -T 'coalesce(description, "(no description set)\n")'
```
