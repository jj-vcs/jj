# Filesets

Jujutsu supports a functional language for selecting a set of files.
Expressions in this language are called "filesets" (the idea comes from
[Mercurial](https://repo.mercurial-scm.org/hg/help/filesets)). The language
consists of file patterns, operators, and functions.

## Quoting file names

Many `jj` commands accept fileset expressions as positional arguments. File
names passed to these commands [must be quoted][string-literals] if they contain
whitespace or meta characters. However, as a special case, quotes can be omitted
if the expression has no operators nor function calls. For example:

* `jj diff 'Foo Bar'` (shell quotes are required, but inner quotes are optional)
* `jj diff '~"Foo Bar"'` (both shell and inner quotes are required)
* `jj diff '"Foo(1)"'` (both shell and inner quotes are required)

Glob characters aren't considered meta characters, but shell quotes are still
required:

* `jj diff '~glob:**/*.rs'`

[string-literals]: templates.md#string-literals

## File patterns

The following patterns are supported. In all cases, we do not mention any shell
quoting that might be necessary, and the quotes around `"path"` are optional if
the path [has no special characters](#quoting-file-names).

* `"path"` or `cwd:"path"`: Matches cwd-relative path prefix (file or files
  under directory recursively.)
* `file:"path"` or `cwd-file:"path"`: Matches cwd-relative file (or exact) path.
* `glob:"pattern"` or `cwd-glob:"pattern"`: Matches file paths with cwd-relative
  Unix-style shell [wildcard `pattern`][glob]. For example, `glob:"*.c"` will
  match all `.c` files in the current working directory non-recursively.
* `prefix-glob:"pattern"` or `cwd-prefix-glob:"pattern"`: Like `glob:`, but also
  matches path prefix (file or files under directory recursively.) For example,
  `prefix-glob:"*.d"` is equivalent to `glob:"*.d" | glob:"*.d/**"`.
* `root:"path"`: Matches workspace-relative path prefix (file or files under
  directory recursively.)
* `root-file:"path"`: Matches workspace-relative file (or exact) path.
* `root-glob:"pattern"`: Matches file paths with workspace-relative Unix-style
  shell [wildcard `pattern`][glob].
* `root-prefix-glob:"pattern"`: Like `root-glob:`, but also matches path prefix
  (file or files under directory recursively.)

Glob patterns support case-insensitive matching by appending `-i` to the pattern
name. For example, `glob-i:"*.TXT"` will match both `file.txt` and `FILE.TXT`.

[glob]: https://docs.rs/globset/latest/globset/#syntax

## Operators

The following operators are supported. `x` and `y` below can be any fileset
expressions.

* `~x`: Matches everything but `x`.
* `x & y`: Matches both `x` and `y`.
* `x ~ y`: Matches `x` but not `y`.
* `x | y`: Matches either `x` or `y` (or both).

(listed in order of binding strengths)

You can use parentheses to control evaluation order, such as `(x & y) | z` or
`x & (y | z)`.

## Functions

You can also specify patterns by using functions.

* `all()`: Matches everything.
* `none()`: Matches nothing.

## Examples

Show diff excluding `Cargo.lock`.

```shell
jj diff '~Cargo.lock'
```

List files in `src` excluding Rust sources.

```shell
jj file list 'src ~ glob:"**/*.rs"'
```

Split a revision in two, putting `foo` into the second commit.

```shell
jj split '~foo'
```
