# Local per-revision metadata

Author: [Isaac Corbrey](isaac@isaaccorbrey.com)

## Summary

Introduce a formal mechanism for associating arbitrary metadata with individual
revisions. This metadata would live entirely locally, with no prescribed method
of pushing it to a remote. As it currently stands, this is a prerequisite of
implementing [topics].

[topics]: https://github.com/jj-vcs/jj/blob/push-yuslknovtlto/docs/design/topics.md

## Prior art

There are a variety of mechanisms other version control systems employ for
metadata (both native and third-party):

- TK Git Notes
- TK SVN revision properties/custom properties
- TK Mercurial tags/bookmarks/branches/extras
- TK Git-Theta

## Scope

- ✅ Attach arbitrary key-value stores to revisions.
- ✅ Avoid rewriting commits when key-value pairs are mutated.
- ✅ Introduce commands for reading/writing key-value pairs.
- ❌ Introduce global per-repo metadata (that's just `jj config set --repo`
  anyway)
- ❌ Syndication of metadata to remotes.
- ❌ Formalization of Topics.

## Detailed design

### Semantics

- Keys and values are both stringly typed for flexibility.
- For a given revision, its keys must all be unique.
- A metadata entry is attached to exactly one **revision ID**, which may be
  either a change ID or a commit hash.
- If attached to a change ID, **the metadata is visible from all commits with
  that change ID.** This naturally survives rebases, amends, duplication, and
  other history rewriting that preserves the change.
	- For **squashes**, the per-change metadata for all source revisions is merged
    into the target revision's metadata. The target revision's entries take
    precedence when they exist. In the event of a `A, B -> C` style squash,
    where `A` and `B` have a key `foo` with disparate values and `C` has no
    value for `foo`, no value is assigned.
	- For **splits**, the per-change metadata naively follows the change ID.
- If attached to a commit hash, **the metadata is only visible when viewing that
  specific commit**. It does not get copied to other commits upon squash, amend,
  duplication, or rebases.
- For revisions that have a key `foo` defined both at the change- and the
  commit-level, the change-level entry will be shadowed and the commit-level
  entry will be visible.

### Storage

TK

### User interface

Commands for interacting with metadata will live under the `jj metaedit` command:

- `jj metaedit --set KEY=VALUE -r REVISION`
- `jj metaedit --unset KEY -r REVISION`

`--set` and `--unset` will have no automatic knowledge of the working copy;
users **must** provide a revset that resolves to at least one revision.

## Future possibilities

- Making the values of metadata entries structured would allow us to more
  easily merge them in the event of squashes. We'd still run into the disparate
  value problem at times, but this would mean we could have the merge semantics
  prescribed by the type (e.g. two changes have topics `["one"]` and `["two"]`
  and get merged as `["one", "two"]`)
- Having either first-class conflicts for merges or allowing callers to
  optionally pipe conflicts to an application (a la `jj resolve --tool`) to
  dynamically choose which values to keep would be a UX improvement long-term.
