//! Deterministic path filters over git history.
//!
//! A *view* is a child repository that is nothing but a pure function of a
//! parent monorepo's history. [`derive()`] computes it: given a parent commit
//! and a path, it produces the commit of the history restricted to that path.
//! [`unfilter`] is the inverse, lifting a commit of the view back into the
//! parent.
//!
//! The property the rest of the design rests on is *round trip hash identity*.
//! Take an upstream repository, inject all of its history into a parent repo by
//! moving every tree under `vendor/upstream/` and copying commit metadata
//! verbatim, then [`derive()`] the parent with the filter `vendor/upstream`.
//! The commits that come back out carry upstream's original hashes, byte for
//! byte. That is what makes the view share real ancestry with upstream, so
//! merge bases are correct and syncing is an ordinary fetch and rebase rather
//! than a translation layer.
//!
//! Identity survives elision, but only because the elision rule asks whether a
//! commit was empty *before* filtering rather than only after. See [`Elide`]
//! for why that one condition is what makes a view both clean and hash
//! compatible.
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
    /// A filter spec was missing a field, had an unknown field, or named a
    /// value this version does not know.
    #[error("filter spec {spec:?} is not one this version can read")]
    BadSpec {
        /// The rejected spec.
        spec: String,
    },
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
/// Two filters differing only in their [`Elide`] policy or their trivial merge
/// policy are distinct filters and get distinct cache entries, because they
/// produce different commits.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Filter {
    components: Vec<BString>,
    elide: Elide,
    keep_trivial_merges: bool,
    semantics: Semantics,
}

/// The version of the hash affecting rules a filter follows.
///
/// Every decision that can change an output commit hash is pinned by this
/// number, and the number is written into the repository next to the filter so
/// a view records what produced it. Without that, a change to any rule silently
/// produces a different history from the same input, and the only symptom is
/// that hashes stop matching a view somebody already has.
///
/// This is not hypothetical. josh has broken its own output hashes at least
/// twice and its compatibility flags are the fossils: `gpgsig="norm-lf"` exists
/// only to reproduce histories from a josh that normalized CRLF inside
/// `gpgsig`, and `history="keep-trivial-merges"` was the default before it
/// became opt in. rust-lang generated commits with a josh that stripped
/// signatures and had to force push the entire history of rustc-dev-guide to
/// recover, and their README now pins an exact josh tag with the reason written
/// down.
///
/// Adding a rule, or changing one, means adding a variant here. Old variants
/// keep their old behavior forever; that is the entire point of them.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum Semantics {
    /// The initial rules.
    ///
    /// 1. Only the `tree` and `parent` lines of a commit object are rewritten.
    ///    Every other byte, including `author`, `committer`, `encoding`,
    ///    `gpgsig`, `mergetag`, any unknown extra header, their relative order,
    ///    and the message, is copied through untouched.
    /// 2. A path absent from a commit's tree filters to the empty tree, so a
    ///    commit that deletes the filtered directory appears in the view as a
    ///    commit that empties it.
    /// 3. Derived parents keep duplicates the source commit already had, and
    ///    drop only duplicates the filter itself introduced by collapsing two
    ///    distinct parents onto one view commit.
    /// 4. Elision follows the filter's [`Elide`] policy.
    /// 5. Trivial merges follow the filter's trivial merge policy, and a merge
    ///    that was already trivial before filtering is never dropped.
    /// 6. Tree entries are ordered by git's rule, with a directory name
    ///    compared as though it ended in a slash.
    #[default]
    V1,
}

impl Semantics {
    /// The number as it appears in a filter spec.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::V1 => "1",
        }
    }
}

/// What to do with a commit whose filtered tree is unchanged from its parent's.
///
/// This is hash affecting, so it is part of the filter's identity and belongs
/// in a recorded semantics version rather than in a caller's discretion.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Elide {
    /// Keep every commit. The view has one commit per parent commit.
    Nothing,
    /// Drop it, unless the commit was already empty *before* filtering.
    ///
    /// The exception is the whole trick, and it is what lets a view be both
    /// clean and hash compatible. A monorepo commit that touched only some
    /// other directory has a tree that differs from its parent's, so it is
    /// dropped and does not clutter the view. An upstream commit that was
    /// deliberately empty has a tree identical to its parent's before
    /// filtering as well as after, so it survives, and the hashes of it and
    /// everything after it are preserved.
    ///
    /// This matches josh's default, implemented in `select_parent_commits` in
    /// `josh-core/src/history.rs` as `if affects_filtered || all_diffs_empty`.
    Unchanged,
    /// Drop it whether or not it was already empty before filtering.
    ///
    /// Do not use this on a history whose hashes must match an upstream. It is
    /// here because it is the rule one writes by accident, it looks correct,
    /// and the damage is invisible until something compares hashes: on
    /// git.git's 85050 commits it drops 7 deliberately empty commits and,
    /// because a moved hash changes every descendant's parent line, moves
    /// 78500 more.
    UnchangedIncludingAlreadyEmpty,
}

impl Filter {
    /// A filter keeping only what lives under `path`.
    ///
    /// Defaults to [`Elide::Unchanged`] and to dropping trivial merges, which
    /// is what josh does by default and what preserves hash identity.
    pub fn prefix(path: &str) -> Result<Self, Error> {
        let components: Vec<BString> = path
            .trim_matches('/')
            .split('/')
            .map(|part| BString::from(part.as_bytes()))
            .collect();
        // An empty component comes from a doubled slash, and a dot component
        // would make it ambiguous which tree entry the filter names. A `;` or a
        // control byte is refused so that a filter spec, which separates its
        // fields with `;`, can be parsed back without an escaping scheme. git
        // permits both in a path; no vendoring mount point needs them.
        let usable = components.iter().all(|part| {
            !matches!(part.as_slice(), b"" | b"." | b"..")
                && !part
                    .iter()
                    .any(|byte| *byte == b';' || byte.is_ascii_control())
        });
        if !usable {
            return Err(Error::BadFilterPath {
                path: path.to_owned(),
            });
        }
        Ok(Self {
            components,
            elide: Elide::Unchanged,
            keep_trivial_merges: false,
            semantics: Semantics::default(),
        })
    }

    /// Sets what happens to a commit whose filtered tree is unchanged.
    #[must_use]
    pub fn elide(mut self, elide: Elide) -> Self {
        self.elide = elide;
        self
    }

    /// Whether a merge whose filtered tree equals its first filtered parent's
    /// is kept.
    ///
    /// Dropping them, the default, is what keeps a view from filling up with
    /// merges that say nothing about the filtered path; rust-lang called the
    /// merge flood the main problem they hit with josh, over 10000 merges for
    /// one initial sync. Keeping them preserves the branch structure at the
    /// cost of degenerate merges whose parents collapse onto one chain.
    ///
    /// A merge that was *already* trivial before filtering is never dropped,
    /// for the same reason an already empty commit is not: dropping it would
    /// move its hash and every descendant's.
    #[must_use]
    pub fn keep_trivial_merges(mut self, keep: bool) -> Self {
        self.keep_trivial_merges = keep;
        self
    }

    /// Sets the semantics version.
    ///
    /// Only needed to reproduce a view built by an older version of this crate.
    #[must_use]
    pub fn semantics(mut self, semantics: Semantics) -> Self {
        self.semantics = semantics;
        self
    }

    /// The canonical spec string, which is what gets written into a repository
    /// beside the view so it records the rules that produced it.
    ///
    /// Round trips through [`parse`](Self::parse).
    #[must_use]
    pub fn spec(&self) -> String {
        let elide = match self.elide {
            Elide::Nothing => "nothing",
            Elide::Unchanged => "unchanged",
            Elide::UnchangedIncludingAlreadyEmpty => "including-already-empty",
        };
        let merges = if self.keep_trivial_merges {
            "keep"
        } else {
            "drop"
        };
        format!(
            "semantics={};prefix={};elide={elide};trivial-merges={merges}",
            self.semantics.as_str(),
            self.path()
        )
    }

    /// Reads a filter back from its [`spec`](Self::spec).
    ///
    /// Every field is required and unknown fields are refused, so a spec
    /// written by a newer version fails loudly here rather than being read
    /// as though the missing rules did not exist.
    pub fn parse(spec: &str) -> Result<Self, Error> {
        let bad = || Error::BadSpec {
            spec: spec.to_owned(),
        };
        let mut semantics = None;
        let mut prefix = None;
        let mut elide = None;
        let mut merges = None;
        for field in spec.split(';') {
            let (key, value) = field.split_once('=').ok_or_else(bad)?;
            match key {
                "semantics" => {
                    semantics = Some(match value {
                        "1" => Semantics::V1,
                        _ => return Err(bad()),
                    });
                }
                "prefix" => prefix = Some(value),
                "elide" => {
                    elide = Some(match value {
                        "nothing" => Elide::Nothing,
                        "unchanged" => Elide::Unchanged,
                        "including-already-empty" => Elide::UnchangedIncludingAlreadyEmpty,
                        _ => return Err(bad()),
                    });
                }
                "trivial-merges" => {
                    merges = Some(match value {
                        "keep" => true,
                        "drop" => false,
                        _ => return Err(bad()),
                    });
                }
                _ => return Err(bad()),
            }
        }
        Ok(Self::prefix(prefix.ok_or_else(bad)?)?
            .semantics(semantics.ok_or_else(bad)?)
            .elide(elide.ok_or_else(bad)?)
            .keep_trivial_merges(merges.ok_or_else(bad)?))
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
    /// Parent-repo commit to its own unfiltered tree. Needed because the
    /// elision rule turns on whether a commit was empty *before* filtering, so
    /// the unfiltered trees of its parents have to be on hand.
    source_trees: HashMap<ObjectId, ObjectId>,
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
    Derivation {
        repo,
        filter,
        cache: per_filter,
    }
    .run(commit)
}

/// Lifts `commit`, a commit of a view, back into the parent repository on top
/// of `onto`.
///
/// The result keeps `commit`'s metadata verbatim. Its parents are the parent
/// repo counterparts of `commit`'s parents where `cache` knows them, which is
/// the case when `commit` came out of [`derive()`] or an earlier `unfilter`
/// with the same cache; a root or an unknown single parent lands on `onto`.
///
/// Its tree is the FIRST PARENT's tree with the filtered path replaced, falling
/// back to `onto`'s tree only when no parent has a counterpart. So `onto`
/// positions a commit whose ancestry is unknown here; it does not pull in
/// monorepo changes that the commit's own parent does not have. Putting a
/// lifted patch on top of a monorepo that has moved is two operations, this one
/// and then a merge, and this one will not silently do the second.
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

    // The base for the tree is the first parent's counterpart when there is one,
    // and only otherwise `onto`. Taking `onto`'s tree while parenting the result
    // on a different commit is not a graft, it is a fabrication: the result would
    // carry every change `onto` made outside the prefix while naming a parent that
    // does not contain them, so those changes would look like this commit's own
    // work and `onto` would be an ancestor of nothing. Lifting a patch and merging
    // the monorepo forward are two operations, and conflating them is how
    // reverse-applying a rewritten view ends up rewriting the monorepo.
    let base = parents.first().copied().unwrap_or_else(|| onto.to_owned());
    let base_tree = commit_tree(repo, &base)?;
    let tree = graft_tree(repo, &base_tree, &filter.components, &parsed.tree())?;
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

struct Derivation<'a> {
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

impl Derivation<'_> {
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
    ///
    /// The order of the decisions here follows `create_filtered_commit2` in
    /// josh's `josh-core/src/history.rs`, because the two rules that keep hash
    /// identity are both easy to get wrong in the same direction and both live
    /// in that order: a trivial merge is spared if it was already trivial, and
    /// an unchanged commit is spared if it was already empty.
    fn map(&mut self, id: &oid, raw: &[u8]) -> Result<(), Error> {
        let parsed = gix::objs::CommitRef::from_bytes(raw, self.repo.object_hash())
            .map_err(|_| Error::MalformedCommit)?;
        let source_tree = parsed.tree();
        let sources: Vec<ObjectId> = parsed.parents().collect();
        let subtree = self.derive_tree(&source_tree)?;
        self.cache.source_trees.insert(id.to_owned(), source_tree);

        // An absent path is the empty tree, not a special case. Treating it
        // uniformly is what makes a commit that *deletes* the filtered
        // directory show up in the view as a commit that empties it, rather
        // than vanishing.
        let filtered_tree =
            subtree.unwrap_or_else(|| ObjectId::empty_tree(self.repo.object_hash()));
        let parents = self.derived_parents(&sources);

        // Did this commit change the filtered path relative to any parent?
        let mut affects_filtered = false;
        for parent in &parents {
            if self.view_tree(parent)? != filtered_tree {
                affects_filtered = true;
                break;
            }
        }
        // Was the commit already empty before filtering? Vacuously true for a
        // root, matching josh, so a root is never dropped for being unchanged.
        let already_empty = sources
            .iter()
            .all(|parent| self.cache.source_trees.get(parent) == Some(&source_tree));

        let mapped = if let Some(collapsed) =
            self.trivial_merge_target(&sources, source_tree, filtered_tree, &parents)?
        {
            Some(collapsed)
        } else {
            let unchanged = !affects_filtered;
            let drop = match self.filter.elide {
                Elide::Nothing => false,
                Elide::Unchanged => unchanged && !already_empty,
                Elide::UnchangedIncludingAlreadyEmpty => unchanged,
            };
            match (drop, parents.first()) {
                // Dropped, and the parent takes its place in the view.
                (true, Some(first)) => Some(*first),
                // Dropped with nothing to fall back to, and nothing of the path
                // here, so there is no counterpart at all.
                (true, None) if subtree.is_none() => None,
                _ if subtree.is_none() && parents.is_empty() => None,
                _ => Some(self.build(id, raw, &filtered_tree, &parents)?),
            }
        };

        self.cache.commits.insert(id.to_owned(), mapped);
        Ok(())
    }

    /// The view commit a trivial merge collapses onto, if it collapses.
    ///
    /// A merge whose filtered tree equals its first filtered parent's says
    /// nothing about the filtered path. It is dropped unless it was already
    /// trivial before filtering, in which case dropping it would move its hash.
    fn trivial_merge_target(
        &mut self,
        sources: &[ObjectId],
        source_tree: ObjectId,
        filtered_tree: ObjectId,
        parents: &[ObjectId],
    ) -> Result<Option<ObjectId>, Error> {
        if self.filter.keep_trivial_merges || parents.len() < 2 {
            return Ok(None);
        }
        let Some(first) = parents.first() else {
            return Ok(None);
        };
        if self.view_tree(first)? != filtered_tree {
            return Ok(None);
        }
        let was_trivial = sources
            .first()
            .and_then(|parent| self.cache.source_trees.get(parent))
            == Some(&source_tree);
        Ok((!was_trivial).then_some(*first))
    }

    /// The view commits this commit's parents map to.
    ///
    /// Two *distinct* parents can map to the same view commit once elision
    /// collapses one onto the other, and a merge with twin parents is not a
    /// merge, so that duplicate is dropped. A commit that already listed the
    /// same parent twice is a different matter: git stores and preserves it, so
    /// collapsing it would move the commit's hash for no reason. The linux
    /// kernel has four such commits and the earliest, from 2005, has 1458483 of
    /// its 1464098 commits as descendants, so a blind dedupe moves 99.6% of
    /// that history. Only duplicates the filter introduced are removed.
    ///
    /// josh does not dedupe at all here, so it can emit a merge whose parents
    /// are the same commit twice. That difference is deliberate and is part of
    /// this filter's semantics.
    fn derived_parents(&self, sources: &[ObjectId]) -> Vec<ObjectId> {
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
        parents
    }

    /// The tree of a view commit, from the cache where possible.
    fn view_tree(&mut self, view: &oid) -> Result<ObjectId, Error> {
        if let Some(hit) = self.cache.view_trees.get(view) {
            return Ok(*hit);
        }
        let tree = commit_tree(self.repo, view)?;
        self.cache.view_trees.insert(view.to_owned(), tree);
        Ok(tree)
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
