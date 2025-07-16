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

In `jj log`/`jj evolog` templates, all 0-argument methods of <code><a href="#commit-type">Commit</a></code> type are
available as keywords. For example, `commit_id` is equivalent to
`self.commit_id()`.

### Operation keywords

In `jj op log` templates, all 0-argument methods of the <code><a href="#operation-type">Operation</a></code> type are
available as keywords. For example, `current_operation` is equivalent to
`self.current_operation()`.

## Operators

The following operators are supported.

* `x.f()`: Method call.
* `-x`: Negate integer value.
* `!x`: Logical not.
* `x * y`, `x / y`, `x % y`: Multiplication/division/remainder. Operands must
  be <code><a href="#integer-type">Integer</a></code>s.
* `x + y`, `x - y`: Addition/subtraction. Operands must be <code><a href="#integer-type">Integer</a></code>s.
* `x >= y`, `x > y`, `x <= y`, `x < y`: Greater than or equal/greater than/
  lesser than or equal/lesser than. Operands must be <code><a href="#integer-type">Integer</a></code>s.
* `x == y`, `x != y`: Equal/not equal. Operands must be either <code><a href="#boolean-type">Boolean</a></code>,
  <code><a href="#integer-type">Integer</a></code>, or <code><a href="#string-type">String</a></code>.
* `x && y`: Logical and, short-circuiting.
* `x || y`: Logical or, short-circuiting.
* `x ++ y`: Concatenate `x` and `y` templates.

(listed in order of binding strengths)

## Global functions

The following functions are defined.

* <code>fill(width: <a href="#integer-type">Integer</a>, content: <a href="#template-type">Template</a>) -&gt; <a href="#template-type">Template</a></code>: Fill lines at
  the given `width`.
* <code>indent(prefix: <a href="#template-type">Template</a>, content: <a href="#template-type">Template</a>) -&gt; <a href="#template-type">Template</a></code>: Indent
  non-empty lines by the given `prefix`.
* <code>pad_start(width: <a href="#integer-type">Integer</a>, content: <a href="#template-type">Template</a>[, fill_char: <a href="#template-type">Template</a>])</code>: Pad (or
  right-justify) content by adding leading fill characters. The `content`
  shouldn't have newline character.
* <code>pad_end(width: <a href="#integer-type">Integer</a>, content: <a href="#template-type">Template</a>[, fill_char: <a href="#template-type">Template</a>])</code>: Pad (or
  left-justify) content by adding trailing fill characters. The `content`
  shouldn't have newline character.
* <code>pad_centered(width: <a href="#integer-type">Integer</a>, content: <a href="#template-type">Template</a>[, fill_char: <a href="#template-type">Template</a>])</code>: Pad
  content by adding both leading and trailing fill characters. If an odd number
  of fill characters are needed, the trailing fill will be one longer than the
  leading fill. The `content` shouldn't have newline characters.
* <code>truncate_start(width: <a href="#integer-type">Integer</a>, content: <a href="#template-type">Template</a>[, ellipsis: <a href="#template-type">Template</a>])</code>:
  Truncate `content` by removing leading characters. The `content` shouldn't
  have newline character. If `ellipsis` is provided and `content` was truncated,
  prepend the `ellipsis` to the result.
* <code>truncate_end(width: <a href="#integer-type">Integer</a>, content: <a href="#template-type">Template</a>[, ellipsis: <a href="#template-type">Template</a>])</code>:
  Truncate `content` by removing trailing characters. The `content` shouldn't
  have newline character. If `ellipsis` is provided and `content` was truncated,
  append the `ellipsis` to the result.
* <code>label(label: <a href="#stringify-type">Stringify</a>, content: <a href="#template-type">Template</a>) -&gt; <a href="#template-type">Template</a></code>: Apply label to
  the content. The `label` is evaluated as a space-separated string.
* <code>raw_escape_sequence(content: <a href="#template-type">Template</a>) -&gt; <a href="#template-type">Template</a></code>: Preserves any escape
  sequences in `content` (i.e., bypasses sanitization) and strips labels.
  Note: This function is intended for escape sequences and as such, its output
  is expected to be invisible / of no display width. Outputting content with
  nonzero display width may break wrapping, indentation etc.
* <code>stringify(content: <a href="#stringify-type">Stringify</a>) -&gt; <a href="#string-type">String</a></code>: Format `content` to string. This
  effectively removes color labels.
* <code>json(value: <a href="#serialize-type">Serialize</a>) -&gt; <a href="#string-type">String</a></code>: Serialize `value` in JSON format.
* <code>if(condition: <a href="#boolean-type">Boolean</a>, then: <a href="#template-type">Template</a>[, else: <a href="#template-type">Template</a>]) -&gt; <a href="#template-type">Template</a></code>:
  Conditionally evaluate `then`/`else` template content.
* <code>coalesce(content: <a href="#template-type">Template</a>...) -&gt; <a href="#template-type">Template</a></code>: Returns the first **non-empty**
  content.
* <code>concat(content: <a href="#template-type">Template</a>...) -&gt; <a href="#template-type">Template</a></code>:
  Same as `content_1 ++ ... ++ content_n`.
* <code>separate(separator: <a href="#template-type">Template</a>, content: <a href="#template-type">Template</a>...) -&gt; <a href="#template-type">Template</a></code>:
  Insert separator between **non-empty** contents.
* <code>surround(prefix: <a href="#template-type">Template</a>, suffix: <a href="#template-type">Template</a>, content: <a href="#template-type">Template</a>) -&gt; <a href="#template-type">Template</a></code>:
  Surround **non-empty** content with texts such as parentheses.
* <code>config(name: <a href="#string-type">String</a>) -&gt; <a href="#configvalue-type">ConfigValue</a></code>: Look up configuration value by `name`.

## Types

### `AnnotationLine` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: no, <code><a href="#template-type">Template</a></code>: no_

The following methods are defined.

* <code>.commit() -&gt; <a href="#commit-type">Commit</a></code>: Commit responsible for changing the relevant line.
* <code>.content() -&gt; <a href="#template-type">Template</a></code>: Line content including newline character.
* <code>.line_number() -&gt; <a href="#integer-type">Integer</a></code>: 1-based line number.
* <code>.first_line_in_hunk() -&gt; <a href="#boolean-type">Boolean</a></code>: False when the directly preceding line
  references the same commit.

### `Boolean` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: yes, <code><a href="#serialize-type">Serialize</a></code>: yes, <code><a href="#template-type">Template</a></code>: yes_

No methods are defined. Can be constructed with `false` or `true` literal.

### `Commit` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: yes, <code><a href="#template-type">Template</a></code>: no_

This type cannot be printed. The following methods are defined.

* <code>.description() -&gt; <a href="#string-type">String</a></code>
* <code>.trailers() -&gt; <a href="#listtrailer-type">List&lt;Trailer&gt;</a></code>
* <code>.change_id() -&gt; <a href="#changeid-type">ChangeId</a></code>
* <code>.commit_id() -&gt; <a href="#commitid-type">CommitId</a></code>
* <code>.parents() -&gt; <a href="#list-type">List</a>&lt;<a href="#commit-type">Commit</a>&gt;</code>
* <code>.author() -&gt; <a href="#signature-type">Signature</a></code>
* <code>.committer() -&gt; <a href="#signature-type">Signature</a></code>
* <code>.signature() -&gt; <a href="#option-type">Option</a>&lt;<a href="#cryptographicsignature-type">CryptographicSignature</a>&gt;</code>: Cryptographic signature if the
  commit was signed.
* <code>.mine() -&gt; <a href="#boolean-type">Boolean</a></code>: Commits where the author's email matches the email of
  the current user.
* <code>.working_copies() -&gt; <a href="#list-type">List</a>&lt;WorkspaceRef&gt;</code>: For multi-workspace repositories, returns a list of workspace references for each workspace whose working-copy commit matches the current commit.
* <code>.current_working_copy() -&gt; <a href="#boolean-type">Boolean</a></code>: True for the working-copy commit of the
  current workspace.
* <code>.bookmarks() -&gt; <a href="#list-type">List</a>&lt;<a href="#commitref-type">CommitRef</a>&gt;</code>: Local and remote bookmarks pointing to the
  commit. A tracking remote bookmark will be included only if its target is
  different from the local one.
* <code>.local_bookmarks() -&gt; <a href="#list-type">List</a>&lt;<a href="#commitref-type">CommitRef</a>&gt;</code>: All local bookmarks pointing to the
  commit.
* <code>.remote_bookmarks() -&gt; <a href="#list-type">List</a>&lt;<a href="#commitref-type">CommitRef</a>&gt;</code>: All remote bookmarks pointing to the
  commit.
* <code>.tags() -&gt; <a href="#list-type">List</a>&lt;<a href="#commitref-type">CommitRef</a>&gt;</code>
* <code>.git_refs() -&gt; <a href="#list-type">List</a>&lt;<a href="#commitref-type">CommitRef</a>&gt;</code>
* <code>.git_head() -&gt; <a href="#boolean-type">Boolean</a></code>: True for the Git `HEAD` commit.
* <code>.divergent() -&gt; <a href="#boolean-type">Boolean</a></code>: True if the commit's change id corresponds to multiple
  visible commits.
* <code>.hidden() -&gt; <a href="#boolean-type">Boolean</a></code>: True if the commit is not visible (a.k.a. abandoned).
* <code>.immutable() -&gt; <a href="#boolean-type">Boolean</a></code>: True if the commit is included in [the set of
  immutable commits](config.md#set-of-immutable-commits).
* <code>.contained_in(revset: <a href="#string-type">String</a>) -&gt; <a href="#boolean-type">Boolean</a></code>: True if the commit is included in [the provided revset](revsets.md).
* <code>.conflict() -&gt; <a href="#boolean-type">Boolean</a></code>: True if the commit contains merge conflicts.
* <code>.empty() -&gt; <a href="#boolean-type">Boolean</a></code>: True if the commit modifies no files.
* <code>.diff([files: <a href="#string-type">String</a>]) -&gt; <a href="#treediff-type">TreeDiff</a></code>: Changes from the parents within [the
  `files` expression](filesets.md). All files are compared by default, but it is
  likely to change in future version to respect the command line path arguments.
* <code>.root() -&gt; <a href="#boolean-type">Boolean</a></code>: True if the commit is the root commit.

### `ChangeId` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: yes, <code><a href="#template-type">Template</a></code>: yes_

The following methods are defined.

* <code>.normal_hex() -&gt; <a href="#string-type">String</a></code>: Normal hex representation (0-9a-f) instead of the
  canonical "reversed" (z-k) representation.
* <code>.short([len: <a href="#integer-type">Integer</a>]) -&gt; <a href="#string-type">String</a></code>
* <code>.shortest([min_len: <a href="#integer-type">Integer</a>]) -&gt; <a href="#shortestidprefix-type">ShortestIdPrefix</a></code>: Shortest unique prefix.

### `CommitId` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: yes, <code><a href="#template-type">Template</a></code>: yes_

The following methods are defined.

* <code>.short([len: <a href="#integer-type">Integer</a>]) -&gt; <a href="#string-type">String</a></code>
* <code>.shortest([min_len: <a href="#integer-type">Integer</a>]) -&gt; <a href="#shortestidprefix-type">ShortestIdPrefix</a></code>: Shortest unique prefix.

### `CommitRef` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: yes, <code><a href="#template-type">Template</a></code>: yes_

The following methods are defined.

* <code>.name() -&gt; <a href="#refsymbol-type">RefSymbol</a></code>: Local bookmark or tag name.
* <code>.remote() -&gt; <a href="#option-type">Option</a>&lt;<a href="#refsymbol-type">RefSymbol</a>&gt;</code>: Remote name if this is a remote ref.
* <code>.present() -&gt; <a href="#boolean-type">Boolean</a></code>: True if the ref points to any commit.
* <code>.conflict() -&gt; <a href="#boolean-type">Boolean</a></code>: True if [the bookmark or tag is
  conflicted](bookmarks.md#conflicts).
* <code>.normal_target() -&gt; <a href="#option-type">Option</a>&lt;<a href="#commit-type">Commit</a>&gt;</code>: Target commit if the ref is not
  conflicted and points to a commit.
* <code>.removed_targets() -&gt; <a href="#list-type">List</a>&lt;<a href="#commit-type">Commit</a>&gt;</code>: Old target commits if conflicted.
* <code>.added_targets() -&gt; <a href="#list-type">List</a>&lt;<a href="#commit-type">Commit</a>&gt;</code>: New target commits. The list usually
  contains one "normal" target.
* <code>.tracked() -&gt; <a href="#boolean-type">Boolean</a></code>: True if the ref is tracked by a local ref. The local
  ref might have been deleted (but not pushed yet.)
* <code>.tracking_present() -&gt; <a href="#boolean-type">Boolean</a></code>: True if the ref is tracked by a local ref,
    and if the local ref points to any commit.
* <code>.tracking_ahead_count() -&gt; <a href="#sizehint-type">SizeHint</a></code>: Number of commits ahead of the tracking
  local ref.
* <code>.tracking_behind_count() -&gt; <a href="#sizehint-type">SizeHint</a></code>: Number of commits behind of the
  tracking local ref.

### `ConfigValue` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: no, <code><a href="#template-type">Template</a></code>: yes_

This type can be printed in TOML syntax. The following methods are defined.

* <code>.as_boolean() -&gt; <a href="#boolean-type">Boolean</a></code>: Extract boolean.
* <code>.as_integer() -&gt; <a href="#integer-type">Integer</a></code>: Extract integer.
* <code>.as_string() -&gt; <a href="#string-type">String</a></code>: Extract string. This does not convert non-string
  value (e.g. integer) to string.
* <code>.as_string_list() -&gt; <a href="#list-type">List</a>&lt;<a href="#string-type">String</a>&gt;</code>: Extract list of strings.

### `CryptographicSignature` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: no, <code><a href="#template-type">Template</a></code>: no_

The following methods are defined.

* <code>.status() -&gt; <a href="#string-type">String</a></code>: The signature's status (`"good"`, `"bad"`, `"unknown"`, `"invalid"`).
* <code>.key() -&gt; <a href="#string-type">String</a></code>: The signature's key id representation (for GPG, this is the key fingerprint).
* <code>.display() -&gt; <a href="#string-type">String</a></code>: The signature's display string (for GPG this is the formatted primary user ID).

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

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: no, <code><a href="#template-type">Template</a></code>: yes_

This type can be printed as a histogram of the changes. The following methods
are defined.

* <code>.total_added() -&gt; <a href="#integer-type">Integer</a></code>: Total number of insertions.
* <code>.total_removed() -&gt; <a href="#integer-type">Integer</a></code>: Total number of deletions.

### `Email` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: yes, <code><a href="#serialize-type">Serialize</a></code>: yes, <code><a href="#template-type">Template</a></code>: yes_

The email field of a signature may or may not look like an email address. It may
be empty, may not contain the symbol `@`, and could in principle contain
multiple `@`s.

The following methods are defined.

* <code>.local() -&gt; <a href="#string-type">String</a></code>: the part of the email before the first `@`, usually the
  username.
* <code>.domain() -&gt; <a href="#string-type">String</a></code>: the part of the email after the first `@` or the empty
  string.

### `Integer` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: yes, <code><a href="#template-type">Template</a></code>: yes_

No methods are defined.

### `List` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: yes, <code><a href="#serialize-type">Serialize</a></code>: maybe, <code><a href="#template-type">Template</a></code>: maybe_

A list can be implicitly converted to <code><a href="#boolean-type">Boolean</a></code>. The following methods are
defined.

* <code>.len() -&gt; <a href="#integer-type">Integer</a></code>: Number of elements in the list.
* <code>.join(separator: <a href="#template-type">Template</a>) -&gt; <a href="#template-type">Template</a></code>: Concatenate elements with
  the given `separator`.
* <code>.filter(|item| expression) -&gt; <a href="#list-type">List</a></code>: Filter list elements by predicate
  `expression`. Example: `description.lines().filter(|s| s.contains("#"))`
* <code>.map(|item| expression) -&gt; <a href="#listtemplate-type">ListTemplate</a></code>: Apply template `expression`
  to each element. Example: `parents.map(|c| c.commit_id().short())`

### `List<Trailer>` type

The following methods are defined. See also the <code><a href="#list-type">List</a></code> type.

* <code>.contains_key(key: <a href="#stringify-type">Stringify</a>) -&gt; <a href="#boolean-type">Boolean</a></code>: True if the commit description
  contains at least one trailer with the key `key`.

### `ListTemplate` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: no, <code><a href="#template-type">Template</a></code>: yes_

The following methods are defined. See also the <code><a href="#list-type">List</a></code> type.

* <code>.join(separator: <a href="#template-type">Template</a>) -&gt; <a href="#template-type">Template</a></code>

### `Operation` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: yes, <code><a href="#template-type">Template</a></code>: no_

This type cannot be printed. The following methods are defined.

* <code>.current_operation() -&gt; <a href="#boolean-type">Boolean</a></code>
* <code>.description() -&gt; <a href="#string-type">String</a></code>
* <code>.id() -&gt; <a href="#operationid-type">OperationId</a></code>
* <code>.tags() -&gt; <a href="#string-type">String</a></code>
* <code>.time() -&gt; <a href="#timestamprange-type">TimestampRange</a></code>
* <code>.user() -&gt; <a href="#string-type">String</a></code>
* <code>.snapshot() -&gt; <a href="#boolean-type">Boolean</a></code>: True if the operation is a snapshot operation.
* <code>.root() -&gt; <a href="#boolean-type">Boolean</a></code>: True if the operation is the root operation.

### `OperationId` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: yes, <code><a href="#template-type">Template</a></code>: yes_

The following methods are defined.

* <code>.short([len: <a href="#integer-type">Integer</a>]) -&gt; <a href="#string-type">String</a></code>

### `Option` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: yes, <code><a href="#serialize-type">Serialize</a></code>: maybe, <code><a href="#template-type">Template</a></code>: maybe_

An option can be implicitly converted to <code><a href="#boolean-type">Boolean</a></code> denoting whether the
contained value is set. If set, all methods of the contained value can be
invoked. If not set, an error will be reported inline on method call.

On comparison between two optional values or optional and non-optional values,
unset value is not an error. Unset value is considered less than any set values.

### `RefSymbol` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: yes, <code><a href="#template-type">Template</a></code>: yes_

A <code><a href="#string-type">String</a></code> type, but is formatted as revset symbol by quoting
and escaping if necessary. Unlike strings, this cannot be implicitly converted
to <code><a href="#boolean-type">Boolean</a></code>.

### `RepoPath` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: yes, <code><a href="#template-type">Template</a></code>: yes_

A slash-separated path relative to the repository root. The following methods
are defined.

* <code>.display() -&gt; <a href="#string-type">String</a></code>: Format path for display. The formatted path uses
  platform-native separator, and is relative to the current working directory.
* <code>.parent() -&gt; <a href="#option-type">Option</a>&lt;<a href="#repopath-type">RepoPath</a>&gt;</code>: Parent directory path.

### `Serialize` type

An expression that can be serialized in machine-readable format such as JSON.

!!! note

    Field names and value types in the serialized output are usually stable
    across jj versions, but the backward compatibility isn't guaranteed. If the
    underlying data model is updated, the serialized output may change.

### `ShortestIdPrefix` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: yes, <code><a href="#template-type">Template</a></code>: yes_

The following methods are defined.

* <code>.prefix() -&gt; <a href="#string-type">String</a></code>
* <code>.rest() -&gt; <a href="#string-type">String</a></code>
* <code>.upper() -&gt; <a href="#shortestidprefix-type">ShortestIdPrefix</a></code>
* <code>.lower() -&gt; <a href="#shortestidprefix-type">ShortestIdPrefix</a></code>

### `Signature` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: yes, <code><a href="#template-type">Template</a></code>: yes_

The following methods are defined.

* <code>.name() -&gt; <a href="#string-type">String</a></code>
* <code>.email() -&gt; <a href="#email-type">Email</a></code>
* <code>.timestamp() -&gt; <a href="#timestamp-type">Timestamp</a></code>

### `SizeHint` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: yes, <code><a href="#template-type">Template</a></code>: no_

This type cannot be printed. The following methods are defined.

* <code>.lower() -&gt; <a href="#integer-type">Integer</a></code>: Lower bound.
* <code>.upper() -&gt; <a href="#option-type">Option</a>&lt;<a href="#integer-type">Integer</a>&gt;</code>: Upper bound if known.
* <code>.exact() -&gt; <a href="#option-type">Option</a>&lt;<a href="#integer-type">Integer</a>&gt;</code>: Exact value if upper bound is known and it
  equals to the lower bound.
* <code>.zero() -&gt; <a href="#boolean-type">Boolean</a></code>: True if upper bound is known and is `0`. Equivalent to
  `.upper() == 0`.

### `String` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: yes, <code><a href="#serialize-type">Serialize</a></code>: yes, <code><a href="#template-type">Template</a></code>: yes_

A string can be implicitly converted to <code><a href="#boolean-type">Boolean</a></code>. The following methods are
defined.

* <code>.len() -&gt; <a href="#integer-type">Integer</a></code>: Length in UTF-8 bytes.
* <code>.contains(needle: <a href="#stringify-type">Stringify</a>) -&gt; <a href="#boolean-type">Boolean</a></code>
* <code>.first_line() -&gt; <a href="#string-type">String</a></code>
* <code>.lines() -&gt; <a href="#list-type">List</a>&lt;<a href="#string-type">String</a>&gt;</code>: Split into lines excluding newline characters.
* <code>.upper() -&gt; <a href="#string-type">String</a></code>
* <code>.lower() -&gt; <a href="#string-type">String</a></code>
* <code>.starts_with(needle: <a href="#stringify-type">Stringify</a>) -&gt; <a href="#boolean-type">Boolean</a></code>
* <code>.ends_with(needle: <a href="#stringify-type">Stringify</a>) -&gt; <a href="#boolean-type">Boolean</a></code>
* <code>.remove_prefix(needle: <a href="#stringify-type">Stringify</a>) -&gt; <a href="#string-type">String</a></code>: Removes the passed prefix, if
  present.
* <code>.remove_suffix(needle: <a href="#stringify-type">Stringify</a>) -&gt; <a href="#string-type">String</a></code>: Removes the passed suffix, if
  present.
* <code>.trim() -&gt; <a href="#string-type">String</a></code>: Removes leading and trailing whitespace
* <code>.trim_start() -&gt; <a href="#string-type">String</a></code>: Removes leading whitespace
* <code>.trim_end() -&gt; <a href="#string-type">String</a></code>: Removes trailing whitespace
* <code>.substr(start: <a href="#integer-type">Integer</a>, end: <a href="#integer-type">Integer</a>) -&gt; <a href="#string-type">String</a></code>: Extract substring. The
  `start`/`end` indices should be specified in UTF-8 bytes. Negative values
  count from the end of the string.
* <code>.escape_json() -&gt; <a href="#string-type">String</a></code>: Serializes the string in JSON format. This
  function is useful for making machine-readable templates. For example, you
  can use it in a template like `'{ "foo": ' ++ foo.escape_json() ++ ' }'` to
  return a JSON/JSONL.

#### String literals

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

### `Stringify` type

An expression that can be converted to a <code><a href="#string-type">String</a></code>.

Any types that can be converted to <code><a href="#template-type">Template</a></code> can also be <code><a href="#stringify-type">Stringify</a></code>. Unlike
<code><a href="#template-type">Template</a></code>, color labels are stripped.

### `Template` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: no, <code><a href="#template-type">Template</a></code>: yes_

Most types can be implicitly converted to <code><a href="#template-type">Template</a></code>. No methods are defined.

### `Timestamp` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: yes, <code><a href="#template-type">Template</a></code>: yes_

The following methods are defined.

* <code>.ago() -&gt; <a href="#string-type">String</a></code>: Format as relative timestamp.
* <code>.format(format: <a href="#string-type">String</a>) -&gt; <a href="#string-type">String</a></code>: Format with [the specified strftime-like
  format string](https://docs.rs/chrono/latest/chrono/format/strftime/).
* <code>.utc() -&gt; <a href="#timestamp-type">Timestamp</a></code>: Convert timestamp into UTC timezone.
* <code>.local() -&gt; <a href="#timestamp-type">Timestamp</a></code>: Convert timestamp into local timezone.
* <code>.after(date: <a href="#string-type">String</a>) -&gt; <a href="#boolean-type">Boolean</a></code>: True if the timestamp is exactly at or after the given date.
* <code>.before(date: <a href="#string-type">String</a>) -&gt; <a href="#boolean-type">Boolean</a></code>: True if the timestamp is before, but not including, the given date.

### `TimestampRange` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: yes, <code><a href="#template-type">Template</a></code>: yes_

The following methods are defined.

* <code>.start() -&gt; <a href="#timestamp-type">Timestamp</a></code>
* <code>.end() -&gt; <a href="#timestamp-type">Timestamp</a></code>
* <code>.duration() -&gt; <a href="#string-type">String</a></code>

### `Trailer` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: no, <code><a href="#template-type">Template</a></code>: yes_

The following methods are defined.

* <code>.key() -&gt; <a href="#string-type">String</a></code>
* <code>.value() -&gt; <a href="#string-type">String</a></code>

### `TreeDiff` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: no, <code><a href="#template-type">Template</a></code>: no_

This type cannot be printed. The following methods are defined.

* <code>.files() -&gt; <a href="#list-type">List</a>&lt;<a href="#treediffentry-type">TreeDiffEntry</a>&gt;</code>: Changed files.
* <code>.color_words([context: <a href="#integer-type">Integer</a>]) -&gt; <a href="#template-type">Template</a></code>: Format as a word-level diff
  with changes indicated only by color.
* <code>.git([context: <a href="#integer-type">Integer</a>]) -&gt; <a href="#template-type">Template</a></code>: Format as a Git diff.
* <code>.stat([width: <a href="#integer-type">Integer</a>]) -&gt; <a href="#diffstats-type">DiffStats</a></code>: Calculate stats of changed lines.
* <code>.summary() -&gt; <a href="#template-type">Template</a></code>: Format as a list of status code and path pairs.

### `TreeDiffEntry` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: no, <code><a href="#template-type">Template</a></code>: no_

This type cannot be printed. The following methods are defined.

* <code>.path() -&gt; <a href="#repopath-type">RepoPath</a></code>: Path to the entry. If the entry is a copy/rename, this
  points to the target (or right) entry.
* <code>.status() -&gt; <a href="#string-type">String</a></code>: One of `"modified"`, `"added"`, `"removed"`,
  `"copied"`, or `"renamed"`.
* <code>.source() -&gt; <a href="#treeentry-type">TreeEntry</a></code>: The source (or left) entry.
* <code>.target() -&gt; <a href="#treeentry-type">TreeEntry</a></code>: The target (or right) entry.

### `TreeEntry` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: no, <code><a href="#template-type">Template</a></code>: no_

This type cannot be printed. The following methods are defined.

* <code>.path() -&gt; <a href="#repopath-type">RepoPath</a></code>: Path to the entry.
* <code>.conflict() -&gt; <a href="#boolean-type">Boolean</a></code>: True if the entry is a merge conflict.
* <code>.file_type() -&gt; <a href="#string-type">String</a></code>: One of `"file"`, `"symlink"`, `"tree"`,
  `"git-submodule"`, or `"conflict"`.
* <code>.executable() -&gt; <a href="#boolean-type">Boolean</a></code>: True if the entry is an executable file.

### `WorkspaceRef` type

_Conversion: <code><a href="#boolean-type">Boolean</a></code>: no, <code><a href="#serialize-type">Serialize</a></code>: yes, <code><a href="#template-type">Template</a></code>: yes_

The following methods are defined.

* <code>.name() -&gt; <a href="#refsymbol-type">RefSymbol</a></code>: Returns the workspace name as a symbol.
* <code>.target() -&gt; <a href="#commit-type">Commit</a></code>: Returns the working-copy commit of this workspace.

## Color labels

Template fragments are usually labeled with the command name, the context (or
the top-level object), and the method names. You can [customize the output
colors][config-colors] by using these labels.

For example, the following template is labeled as `op_log operation id short`:

```sh
jj op log -T 'self.id().short()'
```

In addition to that, you can insert arbitrary labels by `label(label, content)`
function.

To inspect how output fragments are labeled, use `--color=debug` option.

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
