# Forge Integration and Code Review Management

Author: [Shish](mailto:shish@shishnet.org)

## Summary

This design document proposes a collection of features which *can* be
implemented separately, but feel too correlated to be discussed without
at least highlighting how much they overlap and work together, and how
much code their implementations would share:

- Adding `<forge> upload` commands for other forges (github, forgejo,
  gitlab, ...) similar to `jj gerrit upload`
- Having the `upload` commands automatically manage change metadata such as
  stacking (in gerrit the stack is implicit because it's based on the git
  branch, but other forges like github require manually creating PRs and
  then calling a separate API to link them together)
- Adding a `<forge> download` command for each forge, so that one developer
  can upload a change to the forge and another can download it, and they
  end up with identical local state
- Integrating code review status into `jj log` output - for each change in
  the tree, check if there is a Code Review associated with it - if so,
  display the status (Accepted, Pending, Needs Changes, etc) as well as
  the status of CI/CD pipelines, and a count of unresolved comments, with
  each of those things being a clickable link to the relevant forge page
- Adding a `<forge> list` command to get an overview of all my open code
  reviews, whether or not there's a local change associated with it
- Adding a `<forge> rebase` command which will rebase local changes onto
  their merge targets, eg if I have two outstanding pull requests, one set
  to be merged into `dev@origin` and one set to be merged into
  `stable@origin`, they can be automatically rebased onto those separate
  branches without needing to remember which is which, or accidentally typo
  and rebase onto the wrong target
- Adding a `<forge> abandon` command which will abandon a code review
- Adding a new `jj cr` command which acts as an abstraction layer across
  forge backends, supporting the common features of all of them - for
  example `jj cr upload` will call `jj gerrit upload` under the hood when
  my remote is pointing at a gerrit server, or call `jj github upload` under
  the hood when my remote is pointing at a github server

Most of these could be implemented totally independently, and they don't
depend on each other (except for `cr` being dependent on having some
backends to dispatch to), but I think they become a lot more useful and
powerful when they all work together.

## State of the Feature as of `jj-cr` Proof-of-Concept

The feature currently exists as a proof-of-concept on
[Shish's `jj-cr` branch](https://github.com/shish/jj/tree/jj-cr). Core
functionality is implemented for multiple backends:

- Gerrit: Full support (upload, list, log, download, rebase, abandon)
- GitHub: Partial support (list, log, download, abandon; upload and rebase
  have some limitations due to github itself having poor support for stacking)
- Phabricator: Partial support (uses phabricator's own `arc` CLI under the
  hood for some operations because the phabricator API is very hairy in some
  places - native support is *possible*, but assuming the user has `arc`
  installed has been good-enough for this proof-of-concept; list, log,
  abandon work natively)
- GitLab, Forgejo: Very experimental - I've mostly just been testing these
  APIs to ensure that there's nothing there which is fundamentally
  incompatible with my current architecture

Abstraction across all of the above: Works pretty well. Backend can be
auto-detected based on forge URL, or specified with the `cr.forge` config
option.

In all of the above cases (except for phabricator/`arc`), the clients for the
forge APIs are simple wrappers built on the `reqwest` HTTP client, built into
the `jj` binary to avoid any dependency on external tools. None of the above
features require particularly advanced API usage, so keeping the the
dependency list minimal seemed like a reasonable approach. (Considering supply
chain risks, different forge client crates bloating the build with multiple
different HTTP client libraries, etc)

The proof-of-concept prioritizes exploring the design space and seeing what's
possible, hoping to inspire future work by showing people the value of good
forge integrations. That said, I've been using the github, gerrit, and
phabricator backends on a daily basis for a couple of months now, and haven't
needed to fall back to `git` / `gh` / `arc`.

## Prior work

### Individual-Forge Support within JJ

- `jj` already has native support for `gerrit upload` in the main branch
- There are some existing proposals for support for individual forges
  - GitHub: https://github.com/jj-vcs/jj/pull/9498
  - Also GitHub: https://github.com/jj-vcs/jj/pull/7483
  - GitHub again: https://github.com/jj-vcs/jj/issues/4555

### Individual-Forge Support alongside JJ

- https://github.com/mrjones2014/jj-gh - support for many github features
- https://github.com/jennings/jj-spr - specifically supports github PRs

### Other Version Control System Clients

- A lot of these features are inspired by Meta's Sapling (`sl`), in particular
  Sapling's tight integration with Phabricator (internal to Meta) and modest
  integration with GitHub (open source)
- I have no experience with Google's internal tooling, but from presentations
  given at conferences, I get the impression that they have a similar kind of
  user interface (IIRC there was mention of Google's internal `jj` build
  having a `jj piper` subcommand?)

### Other Forge-Specific Clients

- GitHub CLI (`gh`): Single-forge; PR-based model doesn't map well to
  multi-commit stacks; their recently released stacking beta is IMHO weirdly
  over-complicated and under-powered. No smartlog style features. Is
  "integrated with jj" in so far as they both use git under the hood, but no
  more than that.
- Arcanist (`arc`): Single-forge (phabricator); lots of powerful
  forge-specific features, but minimal integration with the source-control
  system (ie. it works by uploading the text of a .diff file into mysql, as
  opposed to `git push`-ing a commit into a repository, so many
  commit-oriented features are lacking)


## Goals and non-goals

### Goals

- Consistency: Provide a set of commands to support common workflows using
  the same terminology across multiple forges
- Unity: Provide a `jj cr` interface that allows developers to type the same
  commands and get the same results regardless of backend
- Don't *prevent* forge-specific features: Allow backends to expose their own
  specialized commands and flags (eg, `jj gerrit upload --topic <TOPIC>`)
- Stack support: Enable managing stacks of commits, not just single changes
  (Initially focussing on the "1 change : 1 code review" model)
- Ergonomics: Eliminate the need to switch between `gh`, `git-review`, `arc`,
  etc, and learn each tool's quirks
- Extensibility: Make it straightforward to add support for new forges
- Internal integration: Seamlessly work with Jujutsu's templates, config, and
  operation log
- External integration: When referring to a review, a CI/CD run, or other
  external entity, provide a link to open it directly.

### Non-goals

- Complete API coverage: The focus is on the most common workflows used across
  multiple forges; less-commonly-used features which only exist in one forge
  can be left to that forge's native tooling
- Replacing forge UIs: The goal is to reduce CLI switching and streamline the
  most common workflows, not eliminate web browsing entirely
- Perfect stacking support: Some forges don't support stacking, others support
  it in a weird and limited way - working with what they offer seems more
  reasonable than attempting to build our own system on top
- Support for "N changes : 1 review"-style stacking: this *could* be done, and
  nothing in the current design would prevent having support for both models,
  but I personally feel that the "1 change : 1 review" model is significantly
  more intuitive as well as being easier to implement
- Backward compatibility during PoC: Breaking changes to commands and config
  are acceptable while exploring the design
- Give JJ a plugin system: In my ideal world it would be possible to
  add support for new backends without needing to modify the core `jj`
  codebase - however as things are right now, JJ doesn't have such a system,
  so for now we build directly into the main binary.

## Overview

### Design Philosophy

The design uses a lowest-common-denominator abstraction on top of
forge-specific implementations:

* Users normally interact with forge-agnostic `jj cr` commands that
  work on all platforms and cover common use cases
* Under the hood, these dispatch to backend-specific implementations
* If a backend-specific feature is needed, users can use the `jj <forge>`
  tools such as the already-existing feature-filled `jj gerrit upload`
* For displaying the status of reviews, a unified `CodeReview` data structure
  and template functions allow integrating the code review status with other
  commands like `jj log`

### User-Facing Workflows

#### Basic Feature Development

This same set of commands works the same way across all supported forges:

```shell
# Start new work from trunk
jj new 'trunk()'
# ... make changes ...
jj commit
# ... make more changes ...
jj commit

# Upload stack of commits for review
jj cr upload

# Check review status with annotated log
jj cr log

# Update based on feedback
jj edit <change-id>
# ... fix issues ...
jj cr upload -m "addressed feedback"
```

### Command Breakdown

* `jj cr upload` - upload a stack of commits to be reviewed, basically
  the same as `jj gerrit upload` does today, but with support for more
  backends
  * `-r, --revision <REF>`
  * `--draft`
  * `-m, --message <MESSAGE>`
* `jj cr download <IDENTIFIER>`
  * Download a given change by its code review identifier (eg gerrit
    change ID, GitHub pull request number)
* `jj cr list` - show a table of open reviews
  * `--json` - (mostly used as part of integration testing)
* `jj cr log`
  * Should take all the same flags as `jj log`, but additionally
    call out to the backend to fetch review status
* `jj cr abandon <IDENTIFIER>`
  * Abandon / close a given review
* `jj cr rebase` - rebase the current branch on its target
  * `-a, --all-crs` - rebase all branches which have an associated CR,
    skip branches without one
  * `-A, --all-branches` - rebase all branches, use the default merge
    target for a branch that doesn't have an associated CR

And then each of the backends will also implement each of these commands
(`jj github upload`, `jj forgejo abandon`, etc)

### Log Formatting

* Adds a new `review()` function to the `Commit` struct, returning
  `Option<CodeReview>`
* Adds a `format_commit_review(commit)` function
* Calls `format_commit_review` from the places where commits are
  normally rendered, eg `format_short_commit_header`

### Detailed Design

#### Backend Code Layout

Each forge backend implements a consistent but not necessarily identical
interface. Backends live in parallel module trees. The `cr` commands
dispatch to the appropriate backend based on the user's configuration.

```
cli/src/commands/
├── cr/                   # Generic dispatcher and utilities
│   ├── mod.rs            # Command routing
│   ├── detect.rs         # Backend auto-detection
│   ├── util.rs           # Shared helpers (branch names, commit mapping)
│   ├── netrc.rs          # Credential loading
│   └── rebase.rs         # Common rebase logic
├── github/
│   ├── mod.rs            # Command handlers
│   ├── client.rs         # HTTP API wrapper
│   ├── upload.rs
│   ├── list.rs
│   ├── log.rs
│   ├── download.rs
│   └── abandon.rs
├── gerrit/
│   ├── mod.rs
│   ├── client.rs
│   └── ...
├── phabricator/
│   └── ...
├── gitlab/
│   └── ...
└── forgejo/
    └── ...
```

#### Top-Down vs Bottom-Up

The current proof-of-concept branch starts by defining some common data
structures used by all backends, and then builds the backends on top.

If the individual commands are the highest priority (`jj github upload`,
`jj forgejo upload`, etc) while people don't think that the abstraction
(`jj cr upload`) is a good idea, then this could be flipped around -
building individual commands that don't share any code to start with,
reviewing and merging them independently, and then factoring out common
parts later on.

#### Integration Testing

As this proposal is all about integration with external systems, I think it's
important to consider how these would be tested.

In the proof-of-concept branch, a `compose.yml` file is included which will
use Docker to spin up an ephemeral instance of each supported forge (except
GitHub, which is closed source), with that instance pre-configured with an
admin account and API key.

The tests for each of the features then have a `TestRepo` struct, eg
`GerritTestRepo`. `GerritTestRepo::new()` will use the pre-populated admin
credentials and the API to create a fresh test repository; tests can then
clone / push / pull / call the API to turn a branch into a pull request /
etc as needed; then when this struct is dropped, the test repository is
deleted.

#### Core Data Model (`core/src/review.rs`)

```rust
pub struct CodeReview {
    /// Forge-specific identifier for the review.
    pub id: String,
    /// Version of the change, eg Gerrit patchset
    pub version: Option<i64>,
    /// The title of the review.
    pub title: String,
    /// The URL of the review.
    pub url: Url,
    /// Forge-specific state name, eg "closed", "abandoned", "trashed"
    pub state_name: String,
    /// The generic state of the review, eg Closed
    pub state: CodeReviewState,
    /// A list of CI/CD checks triggered by the CR
    pub checks: Vec<Check>,
    /// The number of unresolved comments on the review.
    pub unresolved_comments: i64,
}

pub struct Check {
    /// Name of the check, eg "lint", "test", "typecheck"
    pub name: String,
    /// The URL of the check, if available.
    pub url: Option<Url>,
    /// The state of the check, eg Pass, InProgress, Fail
    pub state: CheckState,
}
```

#### Template Integration

The `CodeReview` type is added to the template system via a custom function

1. `jj cr log` fetches CR status for all displayed commits
2. Builds a `HashMap<CommitId, CodeReview>` from forge APIs
3. Passes this map to the template renderer as context
4. Custom template function `commit.review()` looks up commit in map and
   returns `Option<CodeReview>`
5. Template can format however user prefers

#### Credential Management

Uses the `~/.netrc` file which seems to be the closest thing to a standard
for unix CLI tools.

```
machine api.github.com login my-username password gho_xxxxx
machine gerrit.company.com login my-username password my-token-xxx
machine gitlab.company.com login my-username password glpat-xxxxx
```

Storing credentials inside JJ's own config files feels wrong to me, but it's
definitely a possibility.

## Alternatives considered

### 1. Dynamic Plugin System via Shared Libraries

This would be nice, but I feel like `jj` would need to have such a system
in place already - implementing a plugin system from scratch as *part* of
implementing forge integrations is a huge amount of scope-creep, and I
don't know enough about `jj`'s internals to even suggest how this might work.

### 2. JJ-Specific Wrapper Tools per Forge

`jj-gh`, `jj-gerrit`, `jj-phab`, etc as separate tools - we sort of have
this already (a few people, myself included, have written such things). The
integration with `jj` internals isn't so nice; the commands are inconsistent
across backends as each tool tends to use its forge's own conventions instead
of `jj`'s.

### 3. Minimal Abstraction over Forge-Native Tools

Thin wrappers around `gh`, `git-review`, `arc` etc without abstraction - the
integration with `jj` is moderate at best, often poor (because the tools are
designed to work with regular git workflows, and get confused by things like
branchless setups); UX across tools is inconsistent.
