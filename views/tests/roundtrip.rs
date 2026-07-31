//! The round trip hash identity check, as a test.
//!
//! Round trip identity is the claim the derived-views design rests on: inject
//! an upstream history under a prefix, filter the prefix back out, and get
//! upstream's own commit hashes back. If it holds, a view shares real ancestry
//! with upstream, so merge bases are correct and syncing is a plain fetch.
//! These tests pin both the claim and the two ways it is known to fail.
//!
//! The history comes from `jj_views::fixture`, which builds it from raw
//! objects. Nothing here touches the network and the object ids are the same on
//! every run.

use std::collections::HashMap;

use gix::ObjectId;
use jj_views::Cache;
use jj_views::Filter;
use jj_views::fixture;

const PREFIX: &str = "vendor/upstream";

struct Injected {
    repo: gix::Repository,
    upstream: fixture::Upstream,
    /// Upstream commit to the parent-repo commit it was injected as.
    map: HashMap<ObjectId, ObjectId>,
    /// Kept so the repository outlives the test.
    _dir: tempfile::TempDir,
}

/// Builds the fixture history, then injects all of it under [`PREFIX`] inside a
/// monorepo that already has files of its own.
fn inject(filter: &Filter) -> Injected {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let repo = gix::init_bare(dir.path()).expect("an empty repository");
    let upstream = fixture::write_upstream(&repo).expect("the fixture history");

    let base = write_base(&repo);
    let mut cache = Cache::new();
    let mut map: HashMap<ObjectId, ObjectId> = HashMap::new();
    for (_, commit) in &upstream.commits {
        let raw = repo
            .find_object(*commit)
            .expect("a fixture commit")
            .detach()
            .data;
        let first_parent = gix::objs::CommitRef::from_bytes(&raw, repo.object_hash())
            .expect("a well formed commit")
            .parents()
            .next();
        let onto = first_parent
            .and_then(|parent| map.get(&parent).copied())
            .unwrap_or(base);
        let injected = jj_views::unfilter(&repo, commit, &onto, filter, &mut cache)
            .expect("prefix injection cannot conflict");
        map.insert(*commit, injected);
    }
    Injected {
        repo,
        upstream,
        map,
        _dir: dir,
    }
}

/// A monorepo commit for the vendored history to sit inside, so the injected
/// trees are not merely the prefix.
fn write_base(repo: &gix::Repository) -> ObjectId {
    let blob = repo.write_blob(b"the monorepo\n").expect("a blob").detach();
    let tree = repo
        .write_object(&gix::objs::Tree {
            entries: vec![gix::objs::tree::Entry {
                mode: gix::objs::tree::EntryKind::Blob.into(),
                filename: "MONOREPO".into(),
                oid: blob,
            }],
        })
        .expect("a tree")
        .detach();
    let raw = format!(
        "tree {tree}\nauthor Monorepo <mono@example.invalid> 1000000000 +0000\ncommitter Monorepo \
         <mono@example.invalid> 1000000000 +0000\n\nthe monorepo\n"
    );
    gix::objs::Write::write_buf(&repo.objects, gix::objs::Kind::Commit, raw.as_bytes())
        .expect("a commit")
}

/// The claim, in the configuration that is meant to satisfy it: every upstream
/// commit comes back with its own hash, byte for byte.
#[test]
fn every_injected_commit_derives_back_to_its_own_hash() {
    let filter = Filter::prefix(PREFIX)
        .expect("a valid prefix")
        .elide_empty(false);
    let world = inject(&filter);

    let mut cache = Cache::new();
    let mut failures: Vec<String> = Vec::new();
    for (label, commit) in &world.upstream.commits {
        let injected = world.map.get(commit).expect("every commit was injected");
        let derived = jj_views::derive(&world.repo, injected, &filter, &mut cache)
            .expect("deriving a prefix cannot fail on a well formed history");
        match derived {
            Some(derived) if derived == *commit => {}
            Some(derived) => {
                let expected = world
                    .repo
                    .find_object(*commit)
                    .expect("upstream")
                    .detach()
                    .data;
                let actual = world
                    .repo
                    .find_object(derived)
                    .expect("derived")
                    .detach()
                    .data;
                failures.push(format!(
                    "{label}: expected {commit}, got {derived}: {}",
                    fixture::describe_difference(&expected, &actual)
                ));
            }
            None => failures.push(format!("{label}: {commit} was dropped entirely")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} commits did not round trip:\n{}",
        failures.len(),
        world.upstream.commits.len(),
        failures.join("\n")
    );
}

/// Elision is the documented exception, and it is transitive. The empty commit
/// loses its counterpart, and everything after it is rebuilt on a different
/// parent list, so its hash moves too.
#[test]
fn elision_costs_the_empty_commit_and_every_descendant() {
    let filter = Filter::prefix(PREFIX).expect("a valid prefix");
    let world = inject(&filter);

    let mut cache = Cache::new();
    let mut matched: Vec<&str> = Vec::new();
    let mut moved: Vec<&str> = Vec::new();
    let mut dropped: Vec<&str> = Vec::new();
    for (label, commit) in &world.upstream.commits {
        let injected = world.map.get(commit).expect("every commit was injected");
        let derived = jj_views::derive(&world.repo, injected, &filter, &mut cache)
            .expect("deriving a prefix cannot fail on a well formed history");
        match derived {
            Some(derived) if derived == *commit => matched.push(label),
            Some(_) => moved.push(label),
            None => dropped.push(label),
        }
    }

    // Everything up to and including the commit before the empty one keeps its
    // hash, because nothing before it was elided.
    assert_eq!(
        matched,
        vec![
            "root",
            "non-ascii",
            "encoding-latin1",
            "gpgsig",
            "gpgsig-before-encoding",
            "negative-zero-offset",
            "odd-timezone-offset",
        ],
        "commits before the first elision must be unaffected"
    );
    // The empty commit derives to its own parent, so it is not dropped from the
    // view, it just stops being a distinct commit.
    assert!(
        dropped.is_empty(),
        "nothing should lose its counterpart outright: {dropped:?}"
    );
    assert_eq!(
        moved,
        vec![
            "empty",
            "modes-symlink-gitlink",
            "tree-order-siblings",
            "left",
            "right",
            "third",
            "merge-with-mergetag",
            "octopus",
            "duplicate-parents",
        ],
        "the elided commit and every descendant must be reported as moved"
    );
}

/// Derivation is a pure function, so a second run over the same input has to
/// produce the same objects, whether or not it reuses a cache.
#[test]
fn derivation_is_deterministic_and_cache_independent() {
    let filter = Filter::prefix(PREFIX).expect("a valid prefix");
    let world = inject(&filter);
    let head = world
        .map
        .get(&world.upstream.head)
        .expect("the head was injected");

    let mut warm = Cache::new();
    let first = jj_views::derive(&world.repo, head, &filter, &mut warm).expect("a derivation");
    let second = jj_views::derive(&world.repo, head, &filter, &mut warm).expect("a derivation");
    let mut cold = Cache::new();
    let third = jj_views::derive(&world.repo, head, &filter, &mut cold).expect("a derivation");

    assert_eq!(first, second, "a warm cache must return the same commit");
    assert_eq!(first, third, "a cold cache must return the same commit");
    assert!(
        warm.tree_entries() > 0,
        "the tree cache should have been populated"
    );
}

/// The direction elision is actually for: monorepo commits that leave the
/// vendored path alone. Each one should vanish from the view, and each one
/// should cost a single tree read rather than one per path component, since
/// every tree on the way in except the changed root is shared with its parent.
#[test]
fn monorepo_commits_that_miss_the_path_elide_and_cost_one_read_each() {
    const CHAIN: usize = 20;

    let filter = Filter::prefix(PREFIX).expect("a valid prefix");
    let world = inject(&filter);
    let mut head = *world
        .map
        .get(&world.upstream.head)
        .expect("the head was injected");

    let mut cache = Cache::new();
    let view_before =
        jj_views::derive(&world.repo, &head, &filter, &mut cache).expect("a derivation");
    let trees_before = cache.tree_reads();
    let commits_before = cache.commit_entries();

    // A chain that only ever rewrites a monorepo file, so the subtree under
    // PREFIX is untouched and its tree object is shared by every commit.
    for step in 0..CHAIN {
        head = append_monorepo_commit(&world.repo, &head, step);
    }
    let view_after =
        jj_views::derive(&world.repo, &head, &filter, &mut cache).expect("a derivation");

    assert_eq!(
        view_after, view_before,
        "commits that miss the filtered path must leave the view unchanged"
    );
    let commits_added = cache.commit_entries() - commits_before;
    assert_eq!(
        commits_added, CHAIN,
        "every new commit should have been mapped"
    );
    let reads_added = cache.tree_reads() - trees_before;
    // One read per commit for its own root tree, which is the one tree that did
    // change. Anything more means a deeper level was re-read per commit, which
    // is what per-level memoization exists to prevent, and what would make
    // incremental derivation cost the depth of the path on every commit.
    assert_eq!(
        reads_added, CHAIN,
        "expected {CHAIN} tree reads for {CHAIN} commits that changed only the root, got \
         {reads_added}"
    );
}

/// Adds a commit on top of `parent` that rewrites one monorepo file and nothing
/// under [`PREFIX`].
fn append_monorepo_commit(repo: &gix::Repository, parent: &ObjectId, step: usize) -> ObjectId {
    let parent_raw = repo.find_object(*parent).expect("a commit").detach().data;
    let parent_tree = gix::objs::CommitRef::from_bytes(&parent_raw, repo.object_hash())
        .expect("a well formed commit")
        .tree();
    let blob = repo
        .write_blob(format!("monorepo revision {step}\n").as_bytes())
        .expect("a blob")
        .detach();

    let base = repo.find_object(parent_tree).expect("a tree").detach().data;
    let decoded =
        gix::objs::TreeRef::from_bytes(&base, repo.object_hash()).expect("a well formed tree");
    let mut entries: Vec<gix::objs::tree::Entry> = decoded
        .entries
        .iter()
        .map(|entry| gix::objs::tree::Entry {
            mode: entry.mode,
            filename: entry.filename.to_owned(),
            oid: entry.oid.to_owned(),
        })
        .collect();
    for entry in &mut entries {
        if entry.filename == "MONOREPO" {
            entry.oid = blob;
        }
    }
    entries.sort();
    let tree = repo
        .write_object(&gix::objs::Tree { entries })
        .expect("a tree")
        .detach();

    let raw = format!(
        "tree {tree}\nparent {parent}\nauthor Monorepo <mono@example.invalid> 100000000{step} \
         +0000\ncommitter Monorepo <mono@example.invalid> 100000000{step} +0000\n\nmonorepo \
         change {step}\n"
    );
    gix::objs::Write::write_buf(&repo.objects, gix::objs::Kind::Commit, raw.as_bytes())
        .expect("a commit")
}

/// A path absent from the whole history has no view, rather than an empty one.
#[test]
fn a_path_that_never_existed_has_no_view() {
    let filter = Filter::prefix(PREFIX).expect("a valid prefix");
    let world = inject(&filter);
    let head = world
        .map
        .get(&world.upstream.head)
        .expect("the head was injected");

    let absent = Filter::prefix("no/such/path").expect("a valid prefix");
    let mut cache = Cache::new();
    let derived = jj_views::derive(&world.repo, head, &absent, &mut cache).expect("a derivation");
    assert_eq!(derived, None, "an absent path must derive to nothing");
}

/// The corner cases the fixture exists for, spelled out. Each of these commits
/// is one a parse-and-reserialize rewrite corrupts, which is why the real
/// rewrite works on raw bytes and why the round trip test above passes.
#[test]
fn the_fixture_contains_commits_a_naive_rewrite_would_corrupt() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let repo = gix::init_bare(dir.path()).expect("an empty repository");
    let upstream = fixture::write_upstream(&repo).expect("the fixture history");

    let mut corrupted: Vec<&str> = Vec::new();
    for (label, commit) in &upstream.commits {
        let raw = repo
            .find_object(*commit)
            .expect("a fixture commit")
            .detach()
            .data;
        if !fixture::naive_rebuild_matches(&raw, repo.object_hash()) {
            corrupted.push(label);
        }
    }
    // Measured, not predicted, and both results were surprises. Header
    // reordering was the expected failure and is not one: gix leaves `encoding`
    // in the extra header list whenever it is not the first header, so `gpgsig`
    // before `encoding` survives. What does not survive is either timezone
    // offset, including `-0000`, which `git fsck` accepts without complaint. So
    // the hazard is not confined to malformed history: an ordinary repository is
    // enough to break a rewrite that reserializes signature lines, which is why
    // the rewrite in `raw` copies them through as bytes.
    assert_eq!(
        corrupted,
        vec!["negative-zero-offset", "odd-timezone-offset"],
        "the fixture must keep exercising timezone offsets a normalizing serializer cannot \
         reproduce, since that is what the byte level rewrite defends against"
    );
}

/// Prefix filters are the only kind, and a path that is not one is refused
/// rather than silently normalized into something else.
#[test]
fn unusable_filter_paths_are_refused() {
    for path in ["a//b", "a/./b", "../escape", "."] {
        assert!(
            Filter::prefix(path).is_err(),
            "{path:?} should not be accepted as a prefix"
        );
    }
    assert_eq!(
        Filter::prefix("/vendor/linux/")
            .expect("leading and trailing slashes are trimmed")
            .path(),
        "vendor/linux",
        "a josh style spelling should normalize to the same filter"
    );
}
