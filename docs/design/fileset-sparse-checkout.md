# Fileset-based Sparse Checkouts via Ordered Structured Patterns

Author: [Priyanka Mandloi](mailto:mandloip@google.com)

**Summary:** This document proposes a redesign of the `jj sparse` command and its storage format to support flexible matching rules (globs and exclusions) using the fileset engine. Instead of storing complex fileset expression strings directly in the repository state, we propose storing an **ordered list of structured pattern rules** (includes/excludes). This design preserves the semantics of sequential CLI operations, enables simple configuration editing, and provides robust backward compatibility.

## Objective

The current `jj sparse` implementation only supports prefix-matching paths. We want to extend this to support:

1.  **Exclusions:** Excluding specific subdirectories or files from a materialized directory.
2.  **Glob Matching:** Including or excluding files based on wildcards (e.g., `*.rs`).

In doing so, we must satisfy these constraints:

*   **Correct Semantics:** Sequential CLI commands must behave chronologically (e.g., adding a subpath after removing its parent must work).
*   **Simple UX:** Editing the sparse configuration via `jj sparse edit` must remain simple and readable, avoiding complex nested formulas.
*   **Backward Compatibility:** Storing sparse patterns must be resilient to future changes in the fileset DSL syntax.

### Non-Goals
*   Allowing users to input arbitrary, complex fileset expressions (using operators like `&`, `|`, `~` or functions like `all()`) directly on the CLI or in the editor (e.g., `jj sparse set --add 'A ~ B'` is **out of scope**).
*   Implementing client-side path remapping.

## Current State

Currently sparse patterns are stored as a flat and unordered list of path prefix strings. A path is in the working copy if and only if it falls under at least one listed prefix.

The following limitations of the current design motivate this proposal:

*   **No Exclusions:** There is no way to say "everything except build/". Users who want that must enumerate every sibling directory by hand and keep the list updated as the tree changes.
*   **No Globs:** Patterns are literal directory prefixes only; there's no way to select, say, all `*.proto` files across the tree.
*   **CWD Inconsistency:** Sparse commands resolve bare paths relative to the repository root, while every other command that accepts paths in `jj` (`diff`, `restore`, `commit`, etc.) resolves them relative to the current working directory. This is a persistent source of surprise for anyone running sparse commands from a subdirectory.

## Proposed State

We propose storing sparse patterns as an ordered list of structured rules (includes/excludes of basic file patterns). During file matching, this list is converted into an in-memory `FilesetExpression`.

### 1. Data Model (Protobuf)

We replace the `repeated string prefixes` in the `SparsePatterns` protobuf definition with a list of structured `SparsePattern` messages.

```proto
message SparsePattern {
  enum MatchType {
    ROOT = 0;             // Matches root:"path" (prefix match)
    ROOT_FILE = 1;        // Matches root-file:"path" (exact match)
    ROOT_GLOB = 2;        // Matches root-glob:"pattern" (glob match)
    ROOT_PREFIX_GLOB = 3; // Matches root-prefix-glob:"pattern" (glob prefix match)
  }
  bool include = 1;       // true for + (add), false for - (remove)
  MatchType match_type = 2;
  string pattern = 3;
}

message SparsePatterns {
  // Deprecate older string-based prefixes (tag 1) to avoid wire-format conflicts
  reserved 1;
  reserved "prefixes";
  repeated SparsePattern rules = 2;
}
```

These structured patterns align directly with `FilePattern` variants in `lib/src/fileset.rs`:
*   `ROOT` -> `FilePattern::PrefixPath`
*   `ROOT_FILE` -> `FilePattern::FilePath`
*   `ROOT_GLOB` -> `FilePattern::FileGlob`
*   `ROOT_PREFIX_GLOB` -> `FilePattern::PrefixGlob`

#### Default and Empty States
*   **Default State (Full Checkout):** A fresh repository has a single default rule: `+ root:""` (or `jj sparse reset`), which compiles to matching the entire tree (`all()`).
*   **Empty State (Zero Files):** If the rule list is cleared (`[]` via `jj sparse set --clear`), it compiles to `none()`, meaning no files are materialized.

### 2. Path Rewriting & CWD Handling

Stored patterns are always repository-relative. If a user enters paths from a subdirectory, `jj` calculates the offset from the repository root to the CWD (`CWD_OFFSET`) and rewrites the patterns to their `root-` equivalents before saving them.

| User Input (in `src/` directory) | Rewritten & Stored Pattern | Match Type |
| :--- | :--- | :--- |
| `--add glob:"*.rs"` | `root-glob:"src/*.rs"` | `ROOT_GLOB` |
| `--add prefix-glob:"*.d"` | `root-prefix-glob:"src/*.d"` | `ROOT_PREFIX_GLOB` |
| `--add file:main.rs` | `root-file:"src/main.rs"` | `ROOT_FILE` |
| `--add mydir` | `root:"src/mydir"` | `ROOT` |

### 3. CLI & Editor UX

#### CLI Commands & Flags
*   `jj sparse set <patterns>...`: Positional arguments overwrite the current configuration (clears the list and sets the specified patterns with `include: true`).
    *   *Example:* `jj sparse set src lib` -> `[+ root:"src", + root:"lib"]`
*   `--add <pattern>`: Appends a new `include: true` rule to the end of the existing list.
*   `--remove <pattern>`: Appends a new `include: false` rule to the end of the existing list.
*   `--clear`: Empties the rule list (resulting in `[]` / `none()`, zero files checked out).
*   `jj sparse reset`: Resets the configuration back to the default full checkout (`[+ root:""]`).

*CLI argument exclusivity:* Positional arguments (`<patterns>...`) and modify flags (`--add`, `--remove`, `--clear`) are mutually exclusive. Combining positional arguments with modify flags in a single command will result in a CLI error.

#### Displaying Patterns (`jj sparse list`)
`jj sparse list` will print the ordered list of rules in a clean, human-readable format:
```txt
+ root-glob:"src/*.rs"
- root:"src/temp"
+ root:"src/temp/keep"
```

#### Editing Configuration (`jj sparse edit`)
Running `jj sparse edit` will open the user's editor with the current configuration.

*   **Display format:** Stored patterns are always displayed in their canonical, repository-absolute form (`root:`, `root-glob:`, `root-file:`) so that the configuration is deterministic and unambiguous regardless of the working directory or workspace from which it is edited.
*   **CWD-relative inputs:** Users may add or modify lines using CWD-relative syntax (e.g. `+ glob:"*.py"` or `+ "subdir"`). On save, `jj` validates and canonicalizes these into repository-absolute `root-` patterns based on the CWD from which `jj sparse edit` was invoked.
*   **Editor guidance:** A commented header at the top of the buffer clarifies the syntax and CWD conversion rules.

##### Example CWD-Relative Walkthrough:
1.  **Current Working Directory:** `/repo/lib/`
2.  **User runs:** `jj sparse edit`
3.  **Editor opens showing the canonical repository-absolute state:**
    ```txt
    # Sparse patterns are repository-relative.
    # Lines starting with '+' include paths, '-' exclude paths.
    # CWD-relative patterns (glob:, file:, bare paths) will be converted to root-relative on save.
    + root:"src"
    ```
4.  **User adds a CWD-relative line:**
    ```txt
    + glob:"*.py"
    ```
5.  **User saves and exits.**
6.  **`jj` parses and canonicalizes:**
    *   `root:"src"` is already absolute -> preserved as `root:"src"`.
    *   `glob:"*.py"` is CWD-relative -> rewritten to `root-glob:"lib/*.py"` using the CWD offset (`lib/`).
7.  **Stored state:** `[+ root:"src", + root-glob:"lib/*.py"]`

During `jj sparse edit`, if the user enters an invalid path, invalid format, or tries to use unsupported fileset operators (e.g., `&` or `~` directly), **the command will error out and abort**, leaving the previous configuration unchanged and restoring the old working copy state.

### 4. Compilation to Fileset AST

When evaluating working copy paths, the stored list of rules is converted into an in-memory `FilesetExpression`, which serves as the internal representation for file matching.

The expression is constructed starting from an empty set accumulator (`F_{-1} = none()`) and sequentially applying set operations in the order the rules were defined. This preserves the semantics of sequential CLI operations:

*   **Initial Base Accumulator:**
    *   `F_{-1} = none()`
*   **Recursive Step (for each rule R_k from k = 0 ... n):**
    *   `F_k = if R_k.include then (F_{k-1} | E_k) else (F_{k-1} ~ E_k)`

Every rule is processed identically with no special cases.

#### Examples:

**Example 1: Excluding from Full Checkout (e.g., "Everything except build")**
Stored rules:
1.  `+ root:""` (`E_0`)
2.  `- root:"build"` (`E_1`)

Evaluation:
*   `F_{-1} = none()`
*   `F_0 = none() | root:"" = all()`
*   `F_1 = all() ~ root:"build"`
*   **Compiled Expression:** `all() ~ root:"build"`

**Example 2: Sequential Inclusions and Exclusions on Nested Paths**
Stored rules:
1.  `+ root-glob:"src/*.rs"` (`E_0`)
2.  `- root:"src/temp"` (`E_1`)
3.  `+ root:"src/temp/keep"` (`E_2`)

Evaluation:
*   `F_{-1} = none()`
*   `F_0 = none() | root-glob:"src/*.rs" = root-glob:"src/*.rs"`
*   `F_1 = root-glob:"src/*.rs" ~ root:"src/temp"`
*   `F_2 = (root-glob:"src/*.rs" ~ root:"src/temp") | root:"src/temp/keep"`
*   **Compiled Expression:**
    ```txt
    ((root-glob:"src/*.rs") ~ root:"src/temp") | root:"src/temp/keep"
    ```


## Alternatives Considered

### 1. Accepting Fileset in CLI
*   **Design:** Allow users to pass arbitrary fileset expressions directly to the CLI (e.g., `jj sparse set "src | glob:*.rs"`).
*   **Why it falls short:** As subsequent add/remove operations are run, the generated string expression becomes increasingly complex. This makes editing the configuration difficult and not intuitive for the user, as they have to manage a single, complex fileset expression.

    *Example of complexity buildup:*
    If a user runs sequential commands attempting to use full fileset expressions:

        1. `jj sparse set --add "src"`
        2. `jj sparse set --add "lib & glob:*.rs"`
        3. `jj sparse set --remove "src & glob:test_*"`
        4. `jj sparse set --add "tests ~ glob:deprecated_*"`

    The resulting serialized expression would look like:
    ```txt
    (((src | (lib & glob:*.rs)) ~ (src & glob:test_*)) | (tests ~ glob:deprecated_*))
    ```

    If the user runs `jj sparse edit`, they must parse and edit this deeply nested AST formula directly. Simple updates (like adjusting a glob pattern or reordering an exclusion) become error-prone due to parenthesization and operator precedence rules (`&`, `|`, `~`).

### 2. Storing Raw Fileset Strings in Proto
*   **Design:** Store the sparse configuration as a single fileset expression string (e.g., `(src | glob:"*.rs") ~ temp`).
*   **Why it falls short:** Risk of backward compatibility issues. If fileset functions are renamed or syntax changes, old stored strings in the Op Store become unparsable.

### 3. Normalized `(positives) ~ (negatives)`
*   **Design:** Normalize the configuration into two separate lists: includes and excludes.
*   **Why it falls short:** It cannot maintain the chronological order in which CLI operations were performed. An exclusion would always win at the top level, making it impossible to re-include a subdirectory of an excluded directory (e.g., excluding `src/temp` but including `src/temp/tests`).

## Issues Addressed

This design addresses the following feature requests:

*   **[Issue #7815: Use filesets for sparse checkouts](https://github.com/jj-vcs/jj/issues/7815):** Proposes utilizing `jj`'s fileset engine to drive sparse checkouts.
*   **[Issue #1896: Allow exclusions in sparse checkouts](https://github.com/jj-vcs/jj/issues/1896):** Requests the ability to exclude specific paths (negations) from the sparse checkout.
