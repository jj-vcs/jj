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

[string-literals]: templates.md#stringliteral-type

## File patterns

The following patterns are supported. In all cases, we do not mention any shell
quoting that might be necessary, and the quotes around `"path"` are optional if
the path [has no special characters](#quoting-file-names).

By default, `"path"` is parsed as a `prefix-glob:` pattern, which matches
cwd-relative path prefix.

* `cwd:"path"`: Matches cwd-relative path prefix (file or files under directory
  recursively.)
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

Operators are listed in order of binding power from strongest to weakest, e.g.
`x | y & z` is interpreted as `x | (y & z)` since `&` has stronger binding power
than `|`. Infix operators of the same binding power are parsed from left to
right, e.g. `x ~ y & z` is interpreted as `(x ~ y) & z` rather than `x ~ (y &
z)`.

As seen above, parentheses can be used to control evaluation order, e.g. `(x &
y) | z` or `x & (y | z)`.

1. * `f(x)`: Function call.

2. * `p:x`: File pattern or pattern alias named `p`.

3. * `~x`: Matches everything but `x`.

4. * `x & y`: Matches both `x` and `y`.
   * `x ~ y`: Matches `x` but not `y`.

5. * `x | y`: Matches either `x` or `y` (or both).

## Functions

You can also specify patterns by using functions.

* `all()`: Matches everything.
* `none()`: Matches nothing.

## Aliases

New symbols, functions, and `<name>:<value>` patterns can be defined in the
config file, by using any combination of the predefined symbols/functions and
other aliases.

Alias functions can be overloaded by the number of parameters. However, builtin
function will be shadowed by name, and can't co-exist with aliases.

For example:

```toml
[fileset-aliases]
'LOCK' = '**/Cargo.lock | **/package-lock.json | **/uv.lock'
'not:x' = '~x'
```

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
