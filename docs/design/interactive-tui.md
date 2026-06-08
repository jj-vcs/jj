<!-- markdownlint-configure-file
{"line-length": {"line_length": 100}, "table-column-style": {"style": "aligned"}}
-->

# Interactive `jj tui` Design

Author: [Josh McKinney](mailto:joshka@users.noreply.github.com)
Prepared with the assistance of AI tooling. I provided substantial direction, repeated review, and
human-in-the-loop editing throughout.

**Summary:** This document proposes `jj tui`: a native built-in terminal UI for moving through jj's
graph, inspecting commits and operations, refreshing in place, and triggering common actions without
constantly bouncing between separate commands. If done well, it would give jj a compelling built-in
interactive workflow, make existing TUI-style commands feel more coherent, and provide a strong
Rust-native foundation for richer views over time.

## Context and Scope

Jujutsu already has strong non-interactive commands such as `jj log`, `jj show`, `jj diff`, `jj op
log`, `jj edit`, and `jj rebase`. It also already has built-in precedents for in-terminal
full-screen workflows such as `cli/src/commands/arrange.rs`. In addition, the jj ecosystem already
has a number of community tools, many of which are not Rust-based.

What is currently missing is an interactive view that makes common inspection and graph-manipulation
tasks cheap to perform without repeatedly leaving the current context, rerunning commands, and
reconstructing the user's place in the graph. A common example is keeping jj open in one terminal
tab while an editor or agent is active in another: today, a user often ends up quitting `jj log` and
rerunning it just to refresh their place after changes land elsewhere.

This proposal is not meant to replace the community ecosystem. The intent is to provide a built-in
interface that tracks the CLI closely, reuses existing jj concepts and configuration, and formalizes
built-in TUI behavior so that interactive jj commands feel like parts of the same family.

## Motivation

This proposal is motivated by a few concrete needs:

* staying in context while work happens elsewhere, such as keeping jj open in one terminal tab while
  an editor or agent is active in another, then refreshing in place instead of quitting and
  rerunning `jj log`
* providing a built-in interactive workflow that stays close to jj's actual concepts, commands, and
  configuration rather than feeling like a separate tool layered on top
* establishing a strong Rust-native implementation path for a non-pane-oriented jj TUI, since the
  closest existing community tools in that space are written in OCaml and Go
* making built-in full-screen commands feel more like parts of one coherent family over time
* creating a real first-party consumer for any future `lib-jj`-style public library surface by
  forcing it to support an actual interactive client

## Disambiguation

This document uses `jj tui` to refer to the proposed built-in Rust-native TUI for jj itself.

The existing OCaml community tool is referred to here as [jj_tui] to avoid confusion with `jj tui`.

### Intended Scope

The intended scope of this design is:

* A new `jj tui` entry point with a log-first experience.
* A small set of navigable views, each centered on a meaningful jj object.
* A named action model that can be configured independently of the default keymap.
* A confirmation policy that is prompt-based and configurable.
* Reuse of existing jj configuration for templates and other display defaults.
* A more consistent structure for built-in TUI views across jj commands.

This design does not attempt to make every existing CLI command a first-class TUI view on day one,
nor to replace external tools.

## Goals and Non-Goals

### Goals

* Make `jj log`-style graph inspection interactive and full-screen.
* Let users inspect the selected item with `show`, `diff`, and file-oriented previews without
  leaving the main view.
* Make common commands on the selected commit or operation cheap to trigger, including `new`,
  `edit`, `describe`, `git fetch`, `git push`, `undo`, `revert`, and `rebase`.
* Keep the TUI aligned with jj's revset model so users can narrow views based on bookmarks, branches
  of work, authors, mutability, and other jj-native categories.
* Support a single-view interaction model, with presentation styles such as inline expansion and
  optional split preview layouts.
* Keep the action system configurable so different keybinding schemes can be supported over time.
* Reuse existing jj template configuration instead of inventing a parallel template system.

### Non-Goals

* Replacing the normal CLI for all workflows.
* Designing all possible views up front.
* Requiring Vim-style modes or a leader-key-based workflow.
* Perfectly matching the semantics of every command in the first iteration.

## Prior Work

There are several relevant influences:

* Terminal editors such as Vim and Neovim provide strong expectations for motion, view switching,
  and discoverable prefix-driven keymaps.
* LazyVim and related Neovim distributions show the value of named actions and transient key-hint
  popups.
* Git TUIs such as lazygit show demand for interactive navigation and command execution over
  version-control objects.
* Existing community tools around jj demonstrate that there is already demand for several different
  interaction styles:
  * [jj-fzf] is centered around the `jj log` graph and terminal previews, including diff, evolog,
    and op-log workflows.
  * [lazyjj] is explicitly lazygit-inspired and represents a more pane-heavy terminal approach.
  * [jj_tui] is described in the community tools page as an unopinionated TUI built in OCaml.
  * [jjui] is a Go-based terminal UI with revset editing, preview support, rebase, bookmarks, and
    op-log workflows. Of the currently visible community tools, this appears closest in spirit to
    the built-in experience proposed here, with a major difference being that this proposal is for a
    Rust-native implementation integrated directly into jj itself.
  * Other tools in the ecosystem span additional implementation languages and environments,
    including editor plugins and browser-based tools, which reinforces that the built-in TUI does
    not need to be the only interface style.

Taken together, that prior work shows both that the demand is real and that there is still clear
room for a built-in Rust-native implementation in jj itself.

* Existing built-in jj terminal UIs such as `arrange` demonstrate that a `ratatui`-based full-screen
  interface fits the codebase and provide patterns worth aligning.

The proposal here intentionally borrows interaction patterns from terminal tools, but keeps the
underlying model centered on jj concepts like commits, operations, bookmarks, and revsets. It also
aims to regularize built-in TUI presentation so that interactive jj commands feel more consistent.
Implementing this as a native Rust TUI is also a meaningful advantage in its own right: it keeps the
built-in experience close to jj's existing command and library layers, reduces impedance between the
UI and the core implementation, and makes it easier to share behavior, configuration, and
maintenance practices with the rest of the codebase. That matters in part because the most relevant
existing non-pane community tools in this space are written in OCaml and Go rather than Rust. It
also creates a strong first-party consumer for any future `lib-jj`-style public library surface: a
tool like this helps make that library shape real by forcing it to serve an actual interactive
client with concrete needs.

## Design Principles

### Views, not modes

The interface should primarily be understood as a set of views sharing a common navigation model.
The default behavior should not require editor-style modal thinking. A user should mostly
experience:

* "I am looking at the commit log."
* "I am looking at the operation log."
* "I have expanded the selected item."

This is different from requiring users to reason about distinct editing modes.

### Stable compact default

The initial view should be compact. Moving up and down should only change the selection. Details
should be expanded explicitly via commands like `show`, `diff`, or `expand`.

This keeps the default presentation stable and fast to scan while still allowing richer preview
behavior to be enabled via configuration later.

### Named actions before fixed keybindings

The TUI should define actions such as `move-down`, `show`, `git-fetch`, and `rebase` independently
of any particular keymap. Default bindings can be opinionated, but the architecture should allow
reconfiguration without changing command logic.

This matters because terminal-native users do not all want the same thing from "Vim-like" bindings.
Some prefer a stronger leader-key or grouped-action approach. Others prefer fewer prefixes and more
direct keys. Opinionated defaults are still useful, but they should be treated as one supported
mindset for operating the UI, not as the only valid interpretation of terminal-style interaction.

### Prompt-based confirmation

Destructive or high-impact commands should use a prompt-and-confirm flow rather than an uppercase
convention. Confirmation policy should be configurable.

### Reuse existing jj configuration

Templates and related display defaults should follow existing jj config such as `templates.log`,
`templates.log_node`, and other established settings wherever possible. The TUI may add its own
config, but it should not fork the conceptual model of jj configuration.

## Top-Level User Experience

The initial command surface is:

```text
jj tui
jj tui log
jj tui log -r 'mine() & mutable()'
jj tui op-log
```

`jj tui` should open the commit log by default.

Where a view already has a natural CLI filtering model, the TUI entry point should support it. In
particular, the log view should support `-r` / `--revision` startup filtering in a way that matches
`jj log`.

The initial visual experience is a borderless full-screen interface with:

* a title line for context such as current view, revset, template, and repo status
* a main scrollable viewport
* a status/prompt line for hints, confirmations, and transient messages

The default commit-log layout should be compact and centered on a single active view. The selected
item can be previewed either inline or in a split layout, depending on user preference and
configuration.

## Proposed Views

### Initial views

The following views are the strongest candidates for early implementation:

* `log`
* `op-log`

These correspond directly to high-value existing workflows and provide a meaningful object to
navigate.

### Likely follow-on views

These views appear useful, but are lower priority than the initial log work:

* `bookmarks`
* `status`
* `evolog`

These should be added incrementally once the main log interaction model is proven.

### Full intended view surface

The long-term design should make room for a broader set of built-in views, even if many of them
arrive after the initial release. A reasonable full-version inventory would include:

* `log`
* `op-log`
* `bookmarks`
* `status`
* `evolog`
* `files`
* `tags`
* `workspaces`
* `sparse`
* `git-remotes`

Not all of these need to be equally rich or equally prominent. The important point is that they are
part of the expected design space for a complete built-in TUI, even if the initial version only
ships a subset.

### Preview types

Preview types are not separate top-level views. They are alternate representations of the selected
item within a view:

* `show`
* `diff`
* `files`

For example, the commit log can render the selected commit in show, diff, or files mode without
changing the surrounding view.

## Layout Model

The interface should be designed around views rather than panes. A view is the primary thing the
user is looking at, such as the commit log or operation log. Panes are only presentation tools. This
distinction matters because the TUI is intentionally not trying to become a pane-heavy dashboard
with multiple regions competing for attention.

The default experience should minimize pane management. Users should not need to decide which pane
is focused, rebalance screen regions, or constantly scan multiple simultaneous information sources
in order to perform common tasks. The main value of `jj tui` is that it keeps the user anchored in
one active view while still making adjacent information and actions easy to reach.

The TUI should support at least the following layout styles:

* `inline-expand`
* `split`

### Inline expand

The selected item expands beneath its compact row. This makes the experience feel like an
interactive `jj log` with details attached to the current selection. It preserves the sense that the
user is still in one view, simply at a deeper level of detail.

### Split

The viewport is divided into a list area and a preview area. This is useful for users who prefer a
stable log region and a persistent preview.

The split geometry does not need to be fixed. Depending on window dimensions, it is reasonable for
the preview to appear side by side in wider terminals and top/bottom in narrower or taller ones.

### Default

The default should be compact log with manual preview activation. The choice of inline versus split
presentation should be a layout preference, not a semantic difference in how previewing works.

Possible future behavior such as "follow selection" should be a preference, not the mandatory
default.

This default is intentional. It keeps the experience close to the CLI's log-first workflow, avoids
forcing users into pane management by default, and prevents the screen from feeling like a dense
dashboard of unrelated information. Users who prefer split presentation should be able to choose it
without being treated as second-class.

## Interaction Model

### Navigation

Navigation should feel natural to terminal-native users. Reasonable defaults include:

* `j` / `k` for next/previous item
* `gg` / `G` for top/bottom
* `Ctrl-d` / `Ctrl-u` for paging
* `h`, `Left`, or `Esc` for collapsing expanded detail
* `l`, `Right`, or `Enter` for expanding the selected item

These bindings are familiar to many users of terminal tools while remaining simple enough to explain
to others.

### Display commands

The most important display commands should be cheap to trigger:

* `s`: show preview
* `d`: diff preview
* `f`: files preview
* `refresh`: reload the current view in place after external changes or background work

These are useful enough, and frequent enough, to justify direct bindings.

### Action families

Jujutsu has too many meaningful commands to map everything onto single keys cleanly. Grouped actions
are therefore justified even without adopting a strict leader-key culture.

Likely families include:

* `g`: git actions such as fetch and push
* `r`: rewrite/history actions such as rebase, abandon, revert, and undo
* `b`: bookmark actions such as set, move, rename, track, and untrack
* `v`: view actions such as switching between log and op-log
* `t`: template and display actions

The exact defaults can evolve, but the architecture should support grouped actions and transient
hint popups.

### Command line escape hatch

The TUI should also have a command line for explicit execution:

```text
:git-fetch
:git-push
:view-op-log
:revset 'bookmarks()'
:rebase
```

This is both a power-user feature and a compatibility path for actions that do not yet have
ergonomic bindings.

Revset filtering should be supported through this command-line model even if a more dedicated in-app
filtering interaction is deferred.

Text search should operate within the currently visible contents of a view, while revsets determine
which items belong in that view in the first place.

## Named Action Model

Actions should be addressable by kebab-case identifiers. For example:

* `move-down`
* `move-up`
* `page-down`
* `expand`
* `collapse`
* `show`
* `diff`
* `files`
* `refresh`
* `view-log`
* `view-op-log`
* `git-fetch`
* `git-push`
* `new`
* `edit`
* `describe`
* `rebase`
* `abandon`
* `revert`
* `undo`
* `bookmark-set`
* `bookmark-move`
* `bookmark-track`
* `template-log`
* `template-preview`

The TUI implementation should dispatch on these actions. Keymaps and menus should resolve to action
identifiers rather than calling command logic directly.

Action identifiers should also support aliases. This makes it possible to keep canonical action
names close to jj's existing CLI vocabulary while still supporting alternate names that reflect
grouping, view context, or prior naming choices.

For example:

* `view-op-log` could have an alias such as `op-log`
* `view-log` could have an alias such as `log`

Aliases should resolve to the same underlying action. They exist for ergonomics, compatibility, and
configurability, not to define different semantics.

## Confirmation Policy

Prompt-based confirmation should be used for actions that are destructive or otherwise high-impact.

Examples:

* abandon selected commit
* revert selected commit
* undo selected or most recent operation
* possibly push, depending on user preference

The confirmation behavior should be configurable. A plausible policy space is:

* `none`
* `destructive`
* `destructive-and-network`
* `all-mutations`

This is preferable to encoding confirmation semantics into uppercase bindings.

For `op-log` in particular, destructive actions such as `undo` should present a focused confirmation
popup or overlay that shows what is about to happen. For example, `undo` should identify the
selected operation, the target operation or state being restored, and the major categories of state
that will be affected. This should make the action legible before the user confirms it.

## Templates and Display Configuration

The TUI should follow existing jj configuration wherever possible.

In particular:

* compact log rows should derive from existing log template defaults
* node rendering should respect existing graph and node configuration
* preview rendering should prefer existing show/log-related templates where practical

Not all preview modes need to be template-driven. `show` should reuse jj templates where practical,
while views such as `diff` may remain primarily structural.

The TUI may need additional configuration for:

* layout selection
* preview follow behavior
* keymap bindings
* confirmation policy

However, these should extend the existing configuration model rather than replace it.

## Revset Filtering

Revsets are one of jj's defining strengths, so the TUI should make room for them rather than
replacing them with a weaker ad hoc filtering model.

At minimum, the log view should support revset filtering at startup:

```text
jj tui log -r 'mine() & mutable()'
jj tui log -r 'bookmarks() | @'
```

Longer term, the TUI should also support changing the active revset from within the interface. This
could happen through the command line, a dedicated prompt, or both.

The current revset should be visible in the title line or status line so the user always knows what
slice of history they are inspecting.

Text search and revset filtering should be treated as complementary features, not substitutes. Text
search is useful for local discovery within the current view, while revsets define which commits or
operations belong in the view in the first place.

## Initial Command Coverage

The first implementation should concentrate on a useful subset of jj commands:

### Commit-log-centered actions

* inspect with show, diff, and file previews
* `jj new`
* `jj edit`
* `jj describe`
* `jj git fetch`
* `jj git push`
* `jj undo`
* `jj revert`
* `jj rebase`

### Operation-log-centered actions

* `jj op show`
* `jj op diff`
* `jj undo`
* `jj op revert`
* `jj op restore`

Undo from `op-log` should not be a blind action. The TUI should show what is being undone and what
state would be restored before confirmation.

This is enough to create real value without claiming to replace every command. Supporting startup
revset filtering for the log view is strongly preferred even if in-app revset editing is deferred.

### Missing and future command coverage

The initial command coverage above is intentionally incomplete. Long term, the goal should be that
all jj commands are represented somehow in the built-in TUI, even if some of them are only exposed
through a command line, menu entry, or a thin dedicated view.

One useful way to think about this is:

* the initial version should cover the most valuable daily workflows
* the full version should eventually provide some path to essentially all jj commands
* that path may be a first-class view, an item action, a prompt, a dialog, or a command-line entry

A later revision of this design should enumerate the full current jj command surface explicitly to
make long-term TUI coverage easier to track.

The following command families are either not covered yet or are only partially covered in this
initial design:

* history rewriting and graph editing beyond `rebase`/`revert`/`undo`, such as `absorb`, `split`,
  `squash`, `simplify-parents`, `duplicate`, `parallelize`, `interdiff`, and `diffedit`
* commit-editing commands such as `commit`, `metaedit`, and `describe`
* navigation helpers such as `next`, `prev`, and `evolog`
* bookmark, tag, file, sparse, and workspace command families
* Git-oriented commands beyond fetch and push, such as clone, import/export, remote management, and
  colocation
* operation-log commands beyond the initial subset
* utility, debug, help, and version-oriented commands

Not every command needs a rich first-class view. Some commands may be best represented as:

* direct actions on the selected item
* entries in a command palette or command line
* thin focused views over a specific object type
* prompts or dialogs launched from another view

It is therefore acceptable for the initial version to omit views such as `bookmarks` or `status`, so
long as the design explicitly leaves room for them and does not assume the initial subset is the
final shape of the product. Appendix C sketches a broader long-term interaction inventory.

The important design constraint is that the TUI should not permanently exclude large areas of jj
functionality from eventual access.

## Detailed Design

### Command structure

The initial command structure should look like:

```text
jj tui
jj tui log
jj tui op-log
```

The default `jj tui` entry should behave like `jj tui log`.

### Shared TUI shell

The implementation should provide a reusable shell for:

* entering and leaving the alternate screen
* raw mode setup and teardown
* event dispatch
* rendering the title line, main area, and status line
* overlay rendering for prompts, menus, and action hints

This should be shared by all future TUI views, rather than keeping the logic embedded in a single
command like `arrange`. Over time, existing built-in TUI commands should be able to adopt the same
shell and conventions where that makes sense.

### View-specific state

Each view should manage:

* current selection
* scroll position
* active preview type
* preview state such as expanded/collapsed
* local search/filter state
* active revset, where applicable

The shell should manage:

* current view
* current prompt/menu state
* key sequence buffering for grouped actions
* transient messages

### Preview behavior

The selected item starts with no active preview. Preview activation is explicit. Once active, the
preview stays associated with the selected item.

This behavior should be agnostic to whether the preview is presented inline or in a split layout.
Inline and split are display choices layered on top of the same underlying preview state.

Optional future behavior can include:

* follow-selection expansion
* pinned preview
* fullscreen preview

These should be preferences layered on top of the same action model.

### Filtering behavior

Where a view supports revsets, the filtering behavior should stay close to the corresponding CLI
command. For example, commit-log filtering should follow the same semantics as `jj log -r <revset>`.

This keeps the TUI predictable for existing jj users and avoids inventing a parallel query language
for one of jj's central concepts.

### Command execution model

Many actions will map to existing jj commands. The TUI should prefer reusing existing command logic
instead of inventing separate implementations.

The preferred model is direct integration with existing command and library codepaths, not shelling
out through command-line parsing where avoidable. In particular, actions triggered from the TUI
should ideally call into the same jj command implementations or lower-level library operations that
the CLI uses, while supplying already-resolved context such as the selected commit, operation, or
revset.

This has several advantages:

* it avoids unnecessary shell parsing and quoting concerns
* it stays closer to the existing CLI semantics
* it makes error handling and refresh behavior easier to control
* it reduces the risk that the TUI and CLI drift apart semantically

The shell should therefore be able to suspend rendering, execute the chosen action in-process, and
then refresh the affected view state.

Fallback execution through a more generic command-dispatch path may still be useful for early
development or for actions not yet directly integrated, but it should be treated as a compatibility
mechanism rather than the long-term architecture.

### Discoverability

The TUI should support transient hints after grouped-action prefixes. This is similar to "which-key"
style discoverability, but should not require the user to adopt a leader-key workflow.

This helps both users who prefer short grouped bindings and users who are not already familiar with
terminal-editor conventions.

## Open Questions

### Navigation stack

The shared shell may benefit from an explicit navigation stack so users can move from one view or
focused context to another and then return naturally. Examples include:

* jumping from commit log to `op-log` and then back
* opening a focused bookmark or evolog context from the current selection
* temporarily entering a fullscreen or dedicated preview and then returning to the previous place

An explicit stack could make "back" behavior more principled, but it also adds state and complexity
to the shell. This should be investigated further as more views are added.

Even before a richer navigation stack exists, back and collapse behavior should return the user to
the immediately prior view or detail level in a predictable way.

### Breadcrumbs

Breadcrumbs may be useful for showing where the current view came from, or what focused context the
user is currently inside. For example, they might clarify that the user is looking at the evolog for
the commit currently selected in the main log.

However, breadcrumbs also consume scarce space in a compact terminal application. It is not yet
clear whether they provide enough value beyond a well-designed title line and status line. This
should remain an open question until the shell has more than one or two mature views.

## Alternatives Considered

### Keep everything as separate CLI invocations

This remains valuable and should continue to work well, but it leaves too much friction in
multi-step graph inspection and rewrite workflows.

### Single-purpose `jj log --interactive`

This is attractive for minimizing new surface area, but it constrains future growth. A `jj tui`
namespace better reflects the likely expansion into multiple views and layouts.

### New external `ratatui`-based TUI

Another option would be to build a new external Rust TUI that talks to jj via CLI invocations and
parsed output. This would still benefit from a Rust-native terminal stack and could evolve
independently of jj's release cadence.

The drawback is that it puts a layer of parsing and interpretation between the UI and jj's actual
data and behavior. For a tool that needs to inspect commit graphs, revsets, bookmarks, operation
history, and Git-backed state in depth, direct access to jj's command and library layers is a
significant advantage. The built-in approach makes it easier to:

* reuse existing command and library code directly
* avoid parsing command output for jj and Git details
* surface richer or more precise state than a text-oriented integration layer
* keep CLI and TUI behavior aligned over time

For this proposal, those integration benefits outweigh the flexibility of keeping the TUI fully
external.

### Hard-code a single keymap

This would simplify implementation at first, but would make it hard to support different user
expectations later. In particular, it would force one view of how terminal- and Vim-shaped
interaction ought to work, even though users often disagree about the right balance between direct
keys and grouped commands. Defining named actions first is a better foundation.

### Permanent split-pane-only UI

This is workable, and it has real advantages. Split layouts can keep more information visible at
once, and they can fit naturally into existing constrained panes such as tmux windows or VS Code
terminals. Some users will reasonably prefer that style for exactly those reasons.

The pushback is not that split panes are bad. The concern is that making panes fundamental would
push the TUI toward focus management and multi-region scanning as primary interaction costs.
Supporting both inline and split layouts is worthwhile, and neither should be treated as
intrinsically more important. The underlying model should remain view-centric rather than
pane-centric.

There is already ecosystem space for pane-heavy interfaces such as [lazyjj], and that niche is
already served for users who prefer a more dashboard-like experience. `jj tui` therefore does not
need to optimize for permanent split-pane usage as its defining default.

## Future Possibilities

Once the initial command is proven useful, plausible extensions include:

* bookmarks view
* status view
* evolog view
* navigation stack support
* breadcrumbs or related navigation affordances
* interactive in-app revset editing and presets
* customizable layouts beyond inline and split
* persistent user-defined keymaps
* richer command palette support
* direct in-process execution of more jj commands
* additional previews such as conflict-specific or bookmark-specific details

## Appendix A: Config Sketch

The exact schema should evolve with implementation, but a plausible starting shape would look like:

```toml
[tui]
default-view = "log"
layout = "inline-expand"
confirm = "destructive"

[tui.preview]
follow-selection = false
default-mode = "show"

[tui.templates]
log = "builtin_log_compact"
preview = "builtin_show"

[tui.keymap]
j = "move-down"
k = "move-up"
gg = "move-top"
G = "move-bottom"
h = "collapse"
l = "expand"
enter = "expand"
esc = "collapse"
s = "show"
d = "diff"
f = "files"
gf = "git-fetch"
gp = "git-push"
rb = "rebase"
bd = "bookmark-delete"
```

This is only a sketch. The important design point is that the config should map keys to named
actions rather than to ad hoc command strings.

## Appendix B: Initial Action Map

The following table sketches a plausible default action map for the initial views.

| Key          | Context        | Action         | Likely behavior        |
| ------------ | -------------- | -------------- | ---------------------- |
| `j`          | any list view  | `move-down`    | move selection         |
| `k`          | any list view  | `move-up`      | move selection         |
| `gg`         | any list view  | `move-top`     | jump to first row      |
| `G`          | any list view  | `move-bottom`  | jump to last row       |
| `h`          | preview active | `collapse`     | hide preview           |
| `l`          | list view      | `expand`       | show preview           |
| `Enter`      | list view      | `expand`       | show preview           |
| `Esc`        | prompt/preview | `cancel`       | cancel prompt or hide  |
| `s`          | commit/op list | `show`         | show-style preview     |
| `d`          | commit/op list | `diff`         | diff-style preview     |
| `f`          | commit list    | `files`        | changed files summary  |
| `R`          | any view       | `refresh`      | reload current view    |
| `n`          | commit log     | `new`          | `jj new`               |
| `e`          | commit log     | `edit`         | `jj edit`              |
| `c`          | commit log     | `describe`     | `jj describe`          |
| `gf`         | repo views     | `git-fetch`    | `jj git fetch`         |
| `gp`         | repo views     | `git-push`     | `jj git push`          |
| `rb`         | commit log     | `rebase`       | target-picking rebase  |
| `u`          | op-log         | `undo`         | confirmed `jj undo`    |
| `v` then `o` | any            | `view-op-log`  | switch to op log       |
| `v` then `l` | any            | `view-log`     | switch to commit log   |
| `:`          | any            | `command-line` | open command prompt    |

This appendix is intentionally illustrative. It exists to make the design more concrete while still
leaving room for implementation feedback and user testing.

## Appendix C: Broader Interaction Map

The initial action map above focuses on the most important early workflows. Longer term, the
built-in TUI should provide some interaction path for essentially all jj commands. That does not
mean every command needs a dedicated shortcut. In many cases, the appropriate access path is a menu
entry, command palette item, command line, or object-specific dialog.

The table below sketches a fuller intended interaction surface.

| jj command family            | Intended TUI access                         | Initial? |
| ---------------------------- | ------------------------------------------- | -------- |
| `log`                        | `view-log`, startup default, command line   | yes      |
| `show`                       | direct preview key, menu, command line      | yes      |
| `diff`                       | direct preview key, menu, command line      | yes      |
| `evolog`                     | dedicated view/action, command line         | later    |
| `status`                     | dedicated view/action                       | later    |
| `next`, `prev`               | direct action, menu, command line           | later    |
| `new`                        | direct action, command line                 | yes      |
| `edit`                       | direct action, command line                 | yes      |
| `describe`                   | direct action, command line                 | yes      |
| `commit`, `metaedit`         | command line, dialog, focused action        | later    |
| `rebase`                     | direct action with target picker            | yes      |
| `abandon`, `revert`          | direct action with confirmation             | partial  |
| `undo`, `redo`, `restore`    | direct action or dialog, op-log integration | partial  |
| `split`, `squash`            | dialog/action from selected commit          | later    |
| `absorb`, `duplicate`        | command line, action menu                   | later    |
| `simplify-parents`           | command line, action menu                   | later    |
| `parallelize`                | command line, action menu                   | later    |
| `diffedit`, `interdiff`      | command line, action menu                   | later    |
| `resolve`                    | status/file view action, command line       | later    |
| `fix`, `sign`, `unsign`      | action menu, command line                   | later    |
| `bookmark ...`               | bookmark view, object actions, command line | later    |
| `tag ...`                    | tag view, object actions, command line      | later    |
| `file ...`                   | file/status views, object actions           | later    |
| `sparse ...`                 | sparse view, settings/action menu           | later    |
| `workspace ...`              | workspace view, command line                | later    |
| `git fetch`, `git push`      | direct action, menu, command line           | yes      |
| `git clone`, `git init`      | startup/onboarding flow, command line       | later    |
| `git import`, `git export`   | command line, Git action menu               | later    |
| `git remote ...`             | remote view, dialogs, command line          | later    |
| `git colocation ...`         | settings/maintenance UI, command line       | later    |
| `op log`                     | `view-op-log`, command line                 | yes      |
| `op show`, `op diff`         | direct preview/action in op-log             | yes      |
| `op revert`, `op restore`    | op-log action with confirmation             | partial  |
| `op abandon`, `op integrate` | op-log action, command line                 | later    |
| `config ...`                 | settings UI or command line                 | later    |
| `help`, `version`, `root`    | command palette, help/about UI              | later    |
| `util ...`, `debug ...`      | command line only or advanced palette       | later    |
| `gerrit ...`                 | connector-specific action/menu              | later    |
| `bench ...`, `bisect ...`    | command line or dedicated workflow          | later    |

This appendix is meant to make the long-term intent explicit: even when the first release is narrow,
the design should leave room for a coherent path to the broader jj command surface.

[jj_tui]: https://github.com/effusion/jj-tui
[jj-fzf]: https://github.com/timbertson/jj-fzf
[jjui]: https://github.com/idursun/jjui
[lazyjj]: https://github.com/Cretezy/lazyjj
