# Style guide

## Panics

Panics are not allowed, especially in code that may run on a server. Calling
`.unwrap()` is okay if it's guaranteed to be safe by previous checks or
documented invariants. For example, if a function is documented as requiring a
non-empty slice as input, it's fine to call `slice[0]` and panic.

## Markdown

Try to wrap at 80 columns. We don't have a formatter yet.

## Prefer lower-level tests to end-to-end tests

When possible, prefer lower-level tests that don't use the `jj` binary.
End-to-end tests are much slower than similar tests that create a repo using
`jj-lib` (roughly 100x slower). It's also often easier to test edge cases in
lower-level tests.

It can still be useful to add a test case or two to check that the lower-level
functionality is correctly hooked up in the CLI. For example, the end-to-end
tests for `jj log` don't need to test that all kinds of revsets are evaluated
correctly (we have tests in `jj-lib` for that), but they should check that the
`-r` flag is respected.

Use end-to-end tests for testing the CLI commands themselves.

## Documentation comments

### General

In general, try to follow [`rust-lang/rfcs` 1574]. The important points are:

- Use documentation comments (`///`) on types, functions, and fields.

- Use Markdown where appropriate, such as for symbols, code blocks, headings,
  etc.

- The first line of a doc comment should be a single-line short sentence that
  summarizes the element being described. There is no hard rule for "short", but
  try to make it both clear and succinct.
  - Since doc comments are interpreted as Markdown, there must be a blank
    comment line between this first line and the rest of the comment:

    ```rust
    // CORRECT:

    /// Foos the given `bar` and `baz`.
    ///
    /// More description about this very important function.
    fn foo(bar: &Bar, baz: &Baz) { ... }
    ```

    ```rust
    // INCORRECT: This entire comment will be interpreted as the "first line".

    /// Foos the given `bar` and `baz`.
    /// More description about this very important function.
    fn foo(bar: &Bar, baz: &Baz) { ... }
    ```

  - For functions, this summary line should be written in third-person singular
    present indicative form ("Returns ..." instead of "Return ..."). You can
    imagine an implicit "This function..." or the function's name before the
    summary line.

- The rest of the comment after the first line should generally be written in
  complete sentences. But use your own judgment too; for example a sentence that
  says "Returns blah blah." would be fine.

[`rust-lang/rfcs` 1574]:
  https://github.com/rust-lang/rfcs/blob/master/text/1574-more-api-documentation-conventions.md

### Subcommand comments

Specifically for `jj` subcommands:

- Doc comments for subcommands should describe its corresponding `*Args` struct.
  For example, documentation for `jj new` goes on `struct NewArgs`.

- As above, write an appropriate short first line for the command, but do not
  include a trailing period / full stop. See `jj -h` for examples (`--help` will
  show the full doc comment for each command, while the short form `-h` will
  only show the first line).
  - Use imperative mood, as if you are telling the command what to do (giving it
    a command, if you will).

  - If the subcommand is actually a "collection" of subcommands (such as
    `jj config` or `jj bookmark`), then feel free to use your own judgment. Some
    existing conventions have been "Commands for \<doing something\>" or "Manage
    \<something\>".

  ```
  CORRECT:
  $ jj new -h
  Create a new, empty change and (by default) edit it in the working copy

  $ jj bookmark create -h
  Create a new bookmark
  ```

  ```
  INCORRECT:
  $ jj new -h
  Creates a new, empty change. The default behavior is to immediately edit this new change, but pass `--no-edit` to avoid this.

  $ jj bookmark create -h
  Creates new bookmarks with the given names and assigns them to the given target revision, which defaults to `@`.
  ```

- The rest of the comment should be written in complete sentences.

- If the command requires complex explanation, such as `jj rebase`, prefer to
  explain it in the command's doc comment rather than in some other online
  documentation page so that all the relevant info can still be accessed via the
  CLI.

- Default aliases (i.e., those defined in [`misc.toml`]) are not automatically
  shown by `clap`, since it doesn't know about them. Mention these at the end of
  the first line for the command in question in the format
  `[default alias: <alias>]`. For example, the first line of `jj describe` is:

  ```
  Update the change description or other metadata [default alias: desc]
  ```

- Very loose guidance, but if you are adding a new command, prefer single
  "action" words (e.g., verbs) over compound words or portmanteaus. It can be a
  good idea to add a new subcommand instead. Please ask for feedback on names in
  the Discord if you are struggling to come up with something or are unsure
  about a name.

- If you are unsure how your comment will look after being processed, run the
  `test_generate_markdown_docs_in_docs_dir` test to generate
  `cli-reference@.md.snap`.

[`misc.toml`]: ../cli/src/config/misc.toml

### Command argument / option comments

Specifically for `jj` command arguments / options:

- As above, write an appropriate short first line for the arg, but do not
  include a trailing period / full stop. See `jj <command> -h` for examples
  (`--help` will show the full doc comment for each arg, while the short form
  `-h` will only show the first line).
  - Since arguments can be things or actions or whatever, there is no hard rule
    for the form of the first line. If it is a noun, such as `-R`/`--repository`
    or `--config`, a good first line could be the answer to the question "What
    is `<arg>`?" If it is a verb, such as `--quiet`, try to stick with
    imperative mood. Use your own judgment for what might be clearest for users.

  - If the argument can be repeated, append "(can be repeated)" to the end of
    the first line. Even if the argument name is plural, prefer to explicitly
    mention that the argument itself can be repeated to preemptively address any
    confusion; for example, `--revisions <REVSETS>` could mean that `<REVSETS>`
    can evaluate to multiple revisions without allowing `--revisions` to appear
    multiple times.

  - If the argument can be repeated, prefer to speak about the "things" in the
    plural, even if the argument name is singular.

  ```
  CORRECT / PREFERRED:
  --ignore-working-copy   Don't snapshot the working copy, and don't update it
  --quiet                 Silence non-primary command output
  --no-pager              Disable the pager
  --config <NAME=VALUE>   Additional configuration options (can be repeated)
  ```

  ```
  INCORRECT / LESS PREFERRED:
  --ignore-working-copy   No snapshotting (so working copy is ignored).
  --quiet                 Provide this option to skip non-primary command output.
  --no-pager              Disables the pager.
  --config <NAME=VALUE>   Override a configuration option (can be repeated).
  ```

- Short options and visible aliases are automatically shown by `clap`, so no
  need to mention them explicitly (for simple cases). However, it may be good to
  mention conflicting arguments / options or how options interact with each
  other if applicable.

- If an argument has a default value, mention what it is, probably in the full
  description (i.e., not the first line).
  - In cases where this default comes from a configurable setting, such as
    `revsets.log` for `--revisions` in `jj log`, mention the setting. Prefer not
    to also spell out the default value of the setting, since that introduces
    toil and potential inconsistency if the default ever changes in the future.

- If you are unsure how your comment will look after being processed, run the
  `test_generate_markdown_docs_in_docs_dir` test to generate
  `cli-reference@.md.snap`.

### Fileset / revset / templating language functions

(Note: In the templating language, this guidance also applies to all type
methods.)

#### Function signatures

- Function signatures are written similar to Rust:

  ```
  function_name(arg1: Type1, arg2: Type2) -> ReturnType
  ```

  (Note that types are only applicable to the templating language and can be
  omitted from the fileset and revset language documentation.)

- For optional parameters, put square brackets around the `arg: Type` part;
  importantly, do not put the brackets around any surrounding commas.

  ```
  CORRECT:
  parents(x, [depth])
  if(condition: Boolean, then: Template, [else: Template]) -> Template
  ```

  ```
  INCORRECT:
  parents(x[, depth])
  if(condition: Boolean, then: Template[, else]) -> Template
  ```

- For named parameters (rare), put the parameter name in the format `[name=]`
  before the `arg: Type` part, with no spaces. Note that this should also be
  included in the square brackets indicating that the parameter is optional.
  `name` and `arg` may be the same, but they don't have to be.

  (Note that currently, named parameters must be optional, since they are
  intended as a way for users to skip previous optional parameters and provide a
  later one instead. However, this may change in the future, especially if named
  parameters make their way to the templating language.)

  ```
  CORRECT:
  remote_bookmarks([name_pattern], [[remote=]remote_pattern])
  ```

  ```
  INCORRECT:
  remote_bookmarks([name_pattern], [remote] = [remote_pattern])
  remote_bookmarks([name_pattern], [remote=remote_pattern])
  ```

- For variadic parameters, add an ellipsis (`...`) after the `arg: Type` part.

  ```
  CORRECT:
  coalesce(revsets...)
  concat(content: Template...) -> Template
  ```

  ```
  INCORRECT:
  coalesce(...revsets)
  concat(*content: Template) -> Template
  ```

#### Descriptions

- For the first sentence...
  - In the fileset language, "functions" act as "filters for files". Follow the
    general Rust style of writing the first sentence in third-person singular
    present indicative form.

  - In the revset language, "functions" evaluate to "a set of revisions".
    Putting a "Returns" prefix for every function is unnecessary; instead, write
    the first sentence as if there is an implicit "The return value is..." or
    "The return values are..." in front of the first sentence.

  - In the templating language, "functions" will "accept inputs, do things, and
    return an output". Follow the general Rust style of writing the first
    sentence in third-person singular present indicative form.

  ```
  CORRECT:
  all(): Matches all files.

  root(): The virtual commit that is the oldest ancestor of all other commits.
  none(): No commits.
  none(): Nothing.

  json(value: Serialize) -> String: Serializes `value` in JSON format.
  ```

  ```
  INCORRECT:
  all(): All files.

  root(): Returns the virtual commit that is the oldest ancestor of all other commits.
  none(): Returns nothing.

  json(value: Serialize) -> String: Serialize `value` in JSON format.
  json(value: Serialize) -> String: The `value` in JSON format.
  ```

- All following sentences should be written as complete sentences.

- We tend to write the entire description as a single paragraph following the
  function signature, but add line breaks for any examples of usages.

- Use `backticks` to reference arguments of the function instead of simply
  saying "the content" or "the width".

- If a parameter's purpose is obvious, it does not need to be explicitly
  mentioned in the description, but try to make sure it is clear what all
  parameters do.

- Mention default values for optional parameters, or what happens if not given.

- Call out edge cases if applicable.

- For type methods in the templating language, try to document every method,
  even if obvious. This will give you the chance to mention any edge cases,
  default behaviors, or differences from other methods.
