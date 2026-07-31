//! Deterministic path filters over git history.
//!
//! A *view* is a child repository that is nothing but a pure function of a
//! parent monorepo's history. [`derive`] computes it: given a parent commit and
//! a path, it produces the commit of the history restricted to that path.
//! [`unfilter`] is the inverse, lifting a commit of the view back into the
//! parent.
//!
//! The property the rest of the design rests on is *round trip hash identity*.
//! Take an upstream repository, inject all of its history into a parent repo by
//! moving every tree under `vendor/upstream/` and copying commit metadata
//! verbatim, then [`derive`] the parent with the filter `vendor/upstream`. The
//! commits that come back out carry upstream's original hashes, byte for byte.
//! That is what makes the view share real ancestry with upstream, so merge
//! bases are correct and syncing is an ordinary fetch and rebase rather than a
//! translation layer.
//!
//! Identity is not unconditional, and [`Filter::elide_empty`] is where it
//! breaks. See that method for the exact trade.
//!
//! Only prefix filters are supported.

#![deny(missing_docs)]

use std::collections::HashMap;

use bstr::BString;
use bstr::ByteSlice as _;
use gix::ObjectId;
use gix::hash::oid;
use gix::objs::Write as _;

mod raw;

pub mod fixture;

/// Anything that can go wrong deriving or unfiltering a view.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The filter path was not usable as a prefix.
    #[error("filter path {path:?} must be relative, with no empty or dot components")]
    BadFilterPath {
        /// The rejected path.
        path: String,
    },
    /// An object could not be read from the repository.
    #[error("could not read object {id}")]
    Find {
        /// The object that could not be read.
        id: ObjectId,
        /// The underlying object database error.
        #[source]
        source: Box<gix::object::find::existing::Error>,
    },
    /// An object could not be written.
    #[error("could not write object")]
    Write(#[from] gix::objs::write::Error),
    /// A commit object did not begin with a `tree` line followed by its
    /// `parent` lines.
    #[error("commit object is malformed")]
    MalformedCommit,
    /// An object was not the kind its referrer claimed.
    #[error("expected object {id} to be a {expected}")]
    WrongKind {
        /// The object in question.
        id: ObjectId,
        /// The kind that was expected.
        expected: gix::objs::Kind,
    },
    /// A commit of the view could not be lifted back into the parent, because
    /// one of its parents has no known position there.
    #[error("commit {id} of the view has no counterpart in the parent repository")]
    Ungrafted {
        /// The view commit with no counterpart.
        id: ObjectId,
    },
}

/// A path prefix to restrict history to.
///
/// Two filters differing only in [`elide_empty`](Self::elide_empty) are
/// distinct filters and get distinct cache entries, because they produce
/// different commits.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Filter {
    components: Vec<BString>,
    elide_empty: bool,
}

impl Filter {
    /// A filter keeping only what lives under `path`.
    ///
    /// Empty commits are elided by default, matching what a josh style filter
    /// does. See [`elide_empty`](Self::elide_empty) for when you do not want
    /// that.
    pub fn prefix(path: &str) -> Result<Self, Error> {
        let components: Vec<BString> = path
            .trim_matches('/')
            .split('/')
            .map(|part| BString::from(part.as_bytes()))
            .collect();
        // An empty component comes from a doubled slash, and a dot component
        // would make it ambiguous which tree entry the filter names.
        let usable = components
            .iter()
            .all(|part| !matches!(part.as_slice(), b"" | b"." | b".."));
        if !usable {
            return Err(Error::BadFilterPath {
                path: path.to_owned(),
            });
        }
        Ok(Self {
            components,
            elide_empty: true,
        })
    }

    /// Whether a commit that leaves the filtered tree unchanged is dropped in
    /// favour of its parent.
    ///
    /// Elision is what makes a view read like a repository of its own: a
    /// monorepo commit that only touched some other directory should not show
    /// up as an empty commit in the view. It is also the one thing that breaks
    /// round trip hash identity, and it breaks it transitively.
    ///
    /// Upstream histories contain commits whose tree equals their parent's:
    /// deliberate empty commits, and merges that resolve entirely to one side.
    /// Under a full prefix injection the filtered tree of such a commit equals
    /// its parent's, so elision drops it, so it has no counterpart whose hash
    /// could match. Every descendant is then built on a different parent list
    /// and its hash moves too, so one elided commit near the root invalidates
    /// everything after it.
    ///
    /// Set this to `false` for a view that must stay hash compatible with the
    /// history it was injected from. Leave it `true` for a view carved out of
    /// history that only ever existed in the monorepo.
    #[must_use]
    pub fn elide_empty(mut self, elide: bool) -> Self {
        self.elide_empty = elide;
        self
    }

    /// The path this filter keeps, as `a/b/c`.
    #[must_use]
    pub fn path(&self) -> BString {
        let mut out = BString::default();
        for (at, component) in self.components.iter().enumerate() {
            if at > 0 {
                out.push(b'/');
            }
            out.extend_from_slice(component);
        }
        out
    }
}

/// Memoized results, safe to reuse across calls and across filters.
///
/// The cache is keyed on `(tree, filter)` rather than on commits, because that
/// is where the sharing is: a monorepo commit that did not touch the filtered
/// path has the same subtree as its parent, and in a real history the large
/// majority of commits are in that position. Reusing a cache is what keeps an
/// incremental derivation proportional to what changed rather than to the size
/// of history.
#[derive(Default)]
pub struct Cache {
    per_filter: HashMap<Filter, FilterCache>,
}

#[derive(Default)]
struct FilterCache {
    /// Subtree lookups, keyed on the tree and how many path components have
    /// been consumed so far, `None` when the path is absent below it.
    ///
    /// Keying on the root tree alone gives almost no sharing, which is worth
    /// spelling out because it is the opposite of what one expects: a monorepo
    /// commit that touched some other directory has a *different* root tree and
    /// the *same* filtered subtree, and that is the common case. The sharing is
    /// one level down, so every level is memoized and a commit that left the
    /// filtered path alone is answered from the first shared tree on the way
    /// in.
    trees: HashMap<(ObjectId, usize), Option<ObjectId>>,
    /// Parent commit to view commit, `None` when nothing of the path exists in
    /// its ancestry.
    commits: HashMap<ObjectId, Option<ObjectId>>,
    /// View commit to its own tree, so elision does not have to re-read the
    /// parent's commit object once per commit.
    view_trees: HashMap<ObjectId, ObjectId>,
    /// View commit back to the parent commit it came from, for [`unfilter`].
    grafts: HashMap<ObjectId, ObjectId>,
    /// Tree objects read while descending, so the cost of a derivation can be
    /// measured rather than assumed.
    tree_reads: usize,
}

impl Cache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many `(tree, filter)` pairs are memoized, over all filters.
    #[must_use]
    pub fn tree_entries(&self) -> usize {
        self.per_filter.values().map(|per| per.trees.len()).sum()
    }

    /// How many commits are memoized, over all filters.
    #[must_use]
    pub fn commit_entries(&self) -> usize {
        self.per_filter.values().map(|per| per.commits.len()).sum()
    }

    /// How many tree objects have been read, over all filters.
    ///
    /// This is the number the cache exists to hold down. A commit that did not
    /// touch the filtered path should cost one read, not one per path
    /// component, because every tree on the way in except the changed root
    /// is shared with its parent commit.
    #[must_use]
    pub fn tree_reads(&self) -> usize {
        self.per_filter.values().map(|per| per.tree_reads).sum()
    }

    /// The view commit `commit` maps to, if this cache has already derived it.
    ///
    /// `Some(None)` means the commit was derived and has no counterpart,
    /// because nothing of the filtered path exists in its ancestry.
    #[must_use]
    pub fn derived(&self, commit: &oid, filter: &Filter) -> Option<Option<ObjectId>> {
        self.per_filter.get(filter)?.commits.get(commit).copied()
    }
}

/// Derives the view commit for `commit` under `filter`.
///
/// Returns `None` when no commit reachable from `commit` contains the filtered
/// path at all, since there is then no history to show.
///
/// This is a pure function of `(commit, filter)` and the contents of `repo`.
/// It writes the objects it produces into `repo`, which for a derived view is
/// the same store the parent lives in.
pub fn derive(
    repo: &gix::Repository,
    commit: &oid,
    filter: &Filter,
    cache: &mut Cache,
) -> Result<Option<ObjectId>, Error> {
    let per_filter = cache.per_filter.entry(filter.clone()).or_default();
    Deriver {
        repo,
        filter,
        cache: per_filter,
    }
    .run(commit)
}

/// Lifts `commit`, a commit of a view, back into the parent repository on top
/// of `onto`.
///
/// The result has `onto`'s tree with the filtered path replaced by `commit`'s
/// tree, and keeps `commit`'s metadata verbatim. Its parents are the parent
/// repo counterparts of `commit`'s parents where `cache` knows them, which is
/// the case when `commit` came out of [`derive`] or an earlier `unfilter` with
/// the same cache; a root or an unknown single parent lands on `onto`.
///
/// `cache` is updated so a later `unfilter` of a descendant finds this commit.
/// Applying this over an entire history, parents first with `onto` set to the
/// previous result, is exactly the prefix injection that round trip identity is
/// about.
///
/// # Errors
///
/// Prefix filters cannot conflict on content, since the result is a pure tree
/// overlay. They can conflict on topology: a merge in the view whose sides were
/// grafted into unrelated places in the parent has no single answer, and a side
/// with no counterpart at all surfaces as [`Error::Ungrafted`].
pub fn unfilter(
    repo: &gix::Repository,
    commit: &oid,
    onto: &oid,
    filter: &Filter,
    cache: &mut Cache,
) -> Result<ObjectId, Error> {
    let per_filter = cache.per_filter.entry(filter.clone()).or_default();
    let raw = read_commit(repo, commit)?;
    let parsed = gix::objs::CommitRef::from_bytes(&raw, repo.object_hash())
        .map_err(|_| Error::MalformedCommit)?;

    let mut parents = Vec::with_capacity(parsed.parents.len());
    for parent in parsed.parents() {
        match per_filter.grafts.get(&parent) {
            Some(grafted) => parents.push(*grafted),
            // A single unknown parent means the view commit sits on history
            // this cache did not inject, so `onto` is the only position it can
            // take. A merge has no such fallback.
            None if parsed.parents.len() == 1 => parents.push(onto.to_owned()),
            None => return Err(Error::Ungrafted { id: parent }),
        }
    }

    let onto_tree = commit_tree(repo, onto)?;
    let tree = graft_tree(repo, &onto_tree, &filter.components, &parsed.tree())?;
    let bytes = raw::replace_ids(&raw, &tree, &parents)?;
    let id = repo.objects.write_buf(gix::objs::Kind::Commit, &bytes)?;
    per_filter.grafts.insert(commit.to_owned(), id);
    per_filter
        .view_trees
        .insert(commit.to_owned(), parsed.tree());
    // Deliberately *not* recording `id -> commit` in `commits`. That the
    // injected commit derives back to `commit` is the claim this crate exists
    // to make good on, not an assumption it may seed itself with; caching it
    // here would turn any round trip check into a lookup of its own input.
    Ok(id)
}

struct Deriver<'a> {
    repo: &'a gix::Repository,
    filter: &'a Filter,
    cache: &'a mut FilterCache,
}

/// One step of the explicit traversal stack.
enum Step {
    /// Resolve the commit's parents first.
    Enter(ObjectId),
    /// Parents are resolved, so map this commit.
    Map(ObjectId),
}

impl Deriver<'_> {
    fn run(&mut self, head: &oid) -> Result<Option<ObjectId>, Error> {
        // An explicit stack rather than recursion: the histories this is for
        // run hundreds of thousands of commits deep on a single chain, which
        // overflows the default stack long before it runs out of memory.
        let mut stack = vec![Step::Enter(head.to_owned())];
        while let Some(step) = stack.pop() {
            let (id, expanded) = match step {
                Step::Enter(id) => (id, false),
                Step::Map(id) => (id, true),
            };
            if self.cache.commits.contains_key(&id) {
                continue;
            }
            let raw = read_commit(self.repo, &id)?;
            if expanded {
                self.map(&id, &raw)?;
                continue;
            }
            let parsed = gix::objs::CommitRef::from_bytes(&raw, self.repo.object_hash())
                .map_err(|_| Error::MalformedCommit)?;
            let pending: Vec<ObjectId> = parsed
                .parents()
                .filter(|parent| !self.cache.commits.contains_key(parent))
                .collect();
            if pending.is_empty() {
                self.map(&id, &raw)?;
            } else {
                stack.push(Step::Map(id));
                stack.extend(pending.into_iter().map(Step::Enter));
            }
        }
        Ok(self.cache.commits.get(head).copied().flatten())
    }

    /// Maps one commit whose parents are already mapped.
    fn map(&mut self, id: &oid, raw: &[u8]) -> Result<(), Error> {
        let parsed = gix::objs::CommitRef::from_bytes(raw, self.repo.object_hash())
            .map_err(|_| Error::MalformedCommit)?;
        let subtree = self.derive_tree(&parsed.tree())?;

        // Two *distinct* parents can map to the same view commit once elision
        // collapses one onto the other, and a merge with twin parents is not a
        // merge, so that duplicate is dropped. A commit that already listed the
        // same parent twice is a different matter: git stores and preserves it,
        // so collapsing it would move the commit's hash for no reason. Only
        // duplicates the filter introduced are removed.
        let sources: Vec<ObjectId> = parsed.parents().collect();
        let mut parents: Vec<ObjectId> = Vec::with_capacity(sources.len());
        for (at, source) in sources.iter().enumerate() {
            let Some(mapped) = self.cache.commits.get(source).copied().flatten() else {
                continue;
            };
            let introduced = sources.iter().take(at).any(|earlier| {
                earlier != source
                    && self.cache.commits.get(earlier).copied().flatten() == Some(mapped)
            });
            if !introduced {
                parents.push(mapped);
            }
        }

        let single = match parents.as_slice() {
            [only] => Some(*only),
            _ => None,
        };
        let mapped = match subtree {
            // Nothing of the filtered path here. A single line of history just
            // continues at the parent; a merge is kept, with an empty tree, so
            // the view does not silently lose a branch point.
            None if parents.len() < 2 => single,
            None => {
                let empty = ObjectId::empty_tree(self.repo.object_hash());
                Some(self.build(id, raw, &empty, &parents)?)
            }
            Some(subtree) => {
                let elide = self.filter.elide_empty
                    && single
                        .is_some_and(|parent| self.cache.view_trees.get(&parent) == Some(&subtree));
                if elide {
                    single
                } else {
                    Some(self.build(id, raw, &subtree, &parents)?)
                }
            }
        };

        self.cache.commits.insert(id.to_owned(), mapped);
        Ok(())
    }

    fn build(
        &mut self,
        source: &oid,
        raw: &[u8],
        tree: &oid,
        parents: &[ObjectId],
    ) -> Result<ObjectId, Error> {
        // Referred to by a commit, so the object has to exist or the view
        // fails `git fsck`. git itself is happy to write it lazily.
        if *tree == *ObjectId::empty_tree(self.repo.object_hash()) {
            self.repo.objects.write(&gix::objs::Tree::default())?;
        }
        let bytes = raw::replace_ids(raw, tree, parents)?;
        let id = self
            .repo
            .objects
            .write_buf(gix::objs::Kind::Commit, &bytes)?;
        self.cache.view_trees.insert(id, tree.to_owned());
        self.cache
            .grafts
            .entry(id)
            .or_insert_with(|| source.to_owned());
        Ok(id)
    }

    /// The subtree of `tree` at the filter's path, memoized at every level.
    fn derive_tree(&mut self, tree: &oid) -> Result<Option<ObjectId>, Error> {
        self.descend(tree.to_owned(), 0)
    }

    /// Recursion depth is the filter's path depth, a handful of components, not
    /// anything that grows with the size of history.
    fn descend(&mut self, tree: ObjectId, depth: usize) -> Result<Option<ObjectId>, Error> {
        let Some(component) = self.filter.components.get(depth) else {
            return Ok(Some(tree));
        };
        if let Some(hit) = self.cache.trees.get(&(tree, depth)) {
            return Ok(*hit);
        }
        self.cache.tree_reads += 1;
        // A blob where the filter expects a directory means the path is absent
        // at this revision, not that the history is broken.
        let found = match lookup(self.repo, &tree, component.as_bstr())? {
            Some((mode, child)) if mode.is_tree() => self.descend(child, depth + 1)?,
            _ => None,
        };
        self.cache.trees.insert((tree, depth), found);
        Ok(found)
    }
}

/// Replaces the subtree at `components` inside `base` with `sub`, creating any
/// missing intermediate trees.
fn graft_tree(
    repo: &gix::Repository,
    base: &oid,
    components: &[BString],
    sub: &oid,
) -> Result<ObjectId, Error> {
    let Some((name, rest)) = components.split_first() else {
        return Ok(sub.to_owned());
    };

    let mut entries = decode_entries(repo, base)?;
    let existing = entries
        .iter()
        .position(|entry| entry.filename == name.as_bstr());
    let child_base = match existing.and_then(|at| entries.get(at)) {
        Some(entry) if entry.mode.is_tree() => entry.oid,
        // Absent, or shadowed by a blob of the same name. The graft replaces
        // it, since the filtered path is authoritative for its own subtree.
        _ => ObjectId::empty_tree(repo.object_hash()),
    };
    let child = graft_tree(repo, &child_base, rest, sub)?;

    let entry = gix::objs::tree::Entry {
        mode: gix::objs::tree::EntryKind::Tree.into(),
        filename: name.clone(),
        oid: child,
    };
    match existing.and_then(|at| entries.get_mut(at)) {
        Some(slot) => *slot = entry,
        None => entries.push(entry),
    }
    // git requires tree entries sorted with directory names compared as if
    // they ended in a slash; `gix_object::tree::Entry`'s ordering does that.
    entries.sort();
    Ok(repo.objects.write(&gix::objs::Tree { entries })?)
}

fn lookup(
    repo: &gix::Repository,
    tree: &oid,
    name: &bstr::BStr,
) -> Result<Option<(gix::objs::tree::EntryMode, ObjectId)>, Error> {
    if *tree == *ObjectId::empty_tree(repo.object_hash()) {
        return Ok(None);
    }
    let object = find(repo, tree)?;
    let decoded =
        gix::objs::TreeRef::from_bytes(&object.data, repo.object_hash()).map_err(|_| {
            Error::WrongKind {
                id: tree.to_owned(),
                expected: gix::objs::Kind::Tree,
            }
        })?;
    Ok(decoded
        .entries
        .iter()
        .find(|entry| entry.filename == name)
        .map(|entry| (entry.mode, entry.oid.to_owned())))
}

fn decode_entries(
    repo: &gix::Repository,
    tree: &oid,
) -> Result<Vec<gix::objs::tree::Entry>, Error> {
    if *tree == *ObjectId::empty_tree(repo.object_hash()) {
        return Ok(Vec::new());
    }
    let object = find(repo, tree)?;
    let decoded =
        gix::objs::TreeRef::from_bytes(&object.data, repo.object_hash()).map_err(|_| {
            Error::WrongKind {
                id: tree.to_owned(),
                expected: gix::objs::Kind::Tree,
            }
        })?;
    Ok(decoded
        .entries
        .iter()
        .map(|entry| gix::objs::tree::Entry {
            mode: entry.mode,
            filename: entry.filename.to_owned(),
            oid: entry.oid.to_owned(),
        })
        .collect())
}

fn find<'repo>(repo: &'repo gix::Repository, id: &oid) -> Result<gix::Object<'repo>, Error> {
    repo.find_object(id).map_err(|source| Error::Find {
        id: id.to_owned(),
        source: Box::new(source),
    })
}

fn read_commit(repo: &gix::Repository, id: &oid) -> Result<Vec<u8>, Error> {
    let object = find(repo, id)?;
    if object.kind != gix::objs::Kind::Commit {
        return Err(Error::WrongKind {
            id: id.to_owned(),
            expected: gix::objs::Kind::Commit,
        });
    }
    Ok(object.detach().data)
}

fn commit_tree(repo: &gix::Repository, id: &oid) -> Result<ObjectId, Error> {
    let raw = read_commit(repo, id)?;
    let parsed = gix::objs::CommitRef::from_bytes(&raw, repo.object_hash())
        .map_err(|_| Error::MalformedCommit)?;
    Ok(parsed.tree())
}
