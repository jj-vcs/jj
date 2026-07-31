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
use std::fmt::Write as _;

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
    // The DEFAULT filter, elision included. Identity does not require turning
    // elision off; it requires elision to ask whether the commit was empty
    // before filtering, which `Elide::Unchanged` does.
    let filter = Filter::prefix(PREFIX).expect("a valid prefix");
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

/// The check on the check, part one. A comparison loop that matched nothing
/// prints the same "0 mismatches" as one that passed, so the round trip
/// assertion is worthless until it has been watched to fail for a known reason.
///
/// Corrupting the tip gives the clean single-mismatch case: the tip has no
/// descendants, so exactly one commit may be reported and it must be that one.
#[test]
fn corrupting_the_tip_is_detected_as_exactly_one_mismatch() {
    let moved = corrupt_and_collect_moved("duplicate-parents");
    assert_eq!(
        moved,
        vec!["duplicate-parents"],
        "corrupting the tip must be reported as exactly that one commit moving; anything else \
         means the comparison is not comparing"
    );
}

/// The check on the check, part two. Corrupting a commit in the middle must
/// report the victim *and* every descendant, because a moved hash changes its
/// children's parent lines. Requiring only the victim here would be requiring
/// the wrong answer; what matters is that the victim is named and comes first.
#[test]
fn corrupting_a_middle_commit_is_detected_on_it_and_its_descendants() {
    let moved = corrupt_and_collect_moved("modes-symlink-gitlink");
    assert_eq!(
        moved,
        vec![
            "modes-symlink-gitlink",
            "tree-order-siblings",
            "left",
            "right",
            "third",
            "merge-with-mergetag",
            "octopus",
            "duplicate-parents",
        ],
        "the corrupted commit and exactly its descendants must be reported, in order"
    );
}

/// Injects the fixture, corrupts the message of the commit labeled `label`,
/// repoints its descendants at the corrupted commit, and returns the labels the
/// round trip check reports as moved, in topological order.
fn corrupt_and_collect_moved(label: &str) -> Vec<&'static str> {
    let filter = Filter::prefix(PREFIX).expect("a valid prefix");
    let world = inject(&filter);

    let (_, victim) = *world
        .upstream
        .commits
        .iter()
        .find(|(candidate, _)| *candidate == label)
        .expect("the fixture has this commit");
    let injected_victim = *world.map.get(&victim).expect("it was injected");

    let corrupted = corrupt_message(&world.repo, &injected_victim);
    let mut map = world.map.clone();
    let mut rewritten: HashMap<ObjectId, ObjectId> = HashMap::new();
    rewritten.insert(injected_victim, corrupted);
    // The victim's own entry has to move too, not just its children's parent
    // lines, or the derivation walks the pristine commit and the corruption is
    // reported against its descendants instead of itself.
    map.insert(victim, corrupted);
    for (_, upstream) in &world.upstream.commits {
        let injected = *map.get(upstream).expect("it was injected");
        let Some(fresh) = repoint_parents(&world.repo, &injected, &rewritten) else {
            continue;
        };
        rewritten.insert(injected, fresh);
        map.insert(*upstream, fresh);
    }

    let mut cache = Cache::new();
    let mut moved: Vec<&'static str> = Vec::new();
    for (candidate, upstream) in &world.upstream.commits {
        let injected = map.get(upstream).expect("it was injected");
        let derived =
            jj_views::derive(&world.repo, injected, &filter, &mut cache).expect("a derivation");
        if derived != Some(*upstream) {
            moved.push(candidate);
        }
    }
    moved
}

/// Rewrites a commit with one byte of its message changed, keeping everything
/// else including its tree and parents.
fn corrupt_message(repo: &gix::Repository, commit: &ObjectId) -> ObjectId {
    let raw = repo.find_object(*commit).expect("a commit").detach().data;
    let mut corrupted = raw.clone();
    // The message starts after the blank line that ends the header block.
    let at = corrupted
        .windows(2)
        .position(|pair| pair == b"\n\n")
        .expect("a commit has a header block")
        + 2;
    let byte = corrupted.get_mut(at).expect("the message is not empty");
    *byte = if *byte == b'X' { b'Y' } else { b'X' };
    let id = gix::objs::Write::write_buf(&repo.objects, gix::objs::Kind::Commit, &corrupted)
        .expect("a commit");
    assert_ne!(id, *commit, "the corruption must change the hash");
    id
}

/// Rewrites `commit` with any parent found in `rewritten` replaced, or returns
/// `None` when none of its parents moved.
fn repoint_parents(
    repo: &gix::Repository,
    commit: &ObjectId,
    rewritten: &HashMap<ObjectId, ObjectId>,
) -> Option<ObjectId> {
    let raw = repo.find_object(*commit).expect("a commit").detach().data;
    let parsed =
        gix::objs::CommitRef::from_bytes(&raw, repo.object_hash()).expect("a well formed commit");
    let parents: Vec<ObjectId> = parsed.parents().collect();
    if !parents.iter().any(|parent| rewritten.contains_key(parent)) {
        return None;
    }
    let mut out = Vec::new();
    for line in raw.split_inclusive(|byte| *byte == b'\n') {
        if line.starts_with(b"parent ") {
            continue;
        }
        if line.starts_with(b"author ") {
            for parent in &parents {
                let mapped = rewritten.get(parent).unwrap_or(parent);
                out.extend_from_slice(format!("parent {mapped}\n").as_bytes());
            }
        }
        out.extend_from_slice(line);
    }
    Some(
        gix::objs::Write::write_buf(&repo.objects, gix::objs::Kind::Commit, &out)
            .expect("a commit"),
    )
}

/// The one condition that makes elision compatible with hash identity, pinned
/// from both sides.
///
/// `Elide::Unchanged` spares a commit that was already empty before filtering,
/// so every hash survives. `Elide::UnchangedIncludingAlreadyEmpty` is the rule
/// one writes by accident: it looks the same, it drops the deliberately empty
/// commit, and because a moved hash changes every descendant's parent line it
/// takes the rest of history with it.
#[test]
fn eliding_an_already_empty_commit_is_what_breaks_identity() {
    let safe = Filter::prefix(PREFIX).expect("a valid prefix");
    let unsafe_rule = Filter::prefix(PREFIX)
        .expect("a valid prefix")
        .elide(jj_views::Elide::UnchangedIncludingAlreadyEmpty);

    assert!(
        moved_labels(&safe).is_empty(),
        "the default rule must move nothing: {:?}",
        moved_labels(&safe)
    );
    assert_eq!(
        moved_labels(&unsafe_rule),
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
        "eliding the already empty commit must take it and every descendant"
    );
}

/// Injects the fixture under `filter` and returns the labels whose derived
/// commit is not the upstream commit it came from.
fn moved_labels(filter: &Filter) -> Vec<&'static str> {
    let world = inject(filter);
    let mut cache = Cache::new();
    let mut moved: Vec<&'static str> = Vec::new();
    for (label, commit) in &world.upstream.commits {
        let injected = world.map.get(commit).expect("every commit was injected");
        let derived =
            jj_views::derive(&world.repo, injected, filter, &mut cache).expect("a derivation");
        if derived != Some(*commit) {
            moved.push(label);
        }
    }
    moved
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

/// The other half of the parent rule. When elision collapses one side of a
/// merge onto the other, the derived commit must lose the duplicate and stop
/// being a merge. This is the case that makes deduplication necessary at all,
/// and it is the same rule that must *not* fire on a source commit that already
/// listed the same parent twice.
#[test]
fn a_merge_whose_side_elides_stops_being_a_merge() {
    let filter = Filter::prefix(PREFIX).expect("a valid prefix");
    let world = inject(&filter);
    let head = *world
        .map
        .get(&world.upstream.head)
        .expect("the head was injected");

    // A side branch that only touches a monorepo file, so it elides.
    let side = append_monorepo_commit(&world.repo, &head, 0);
    // The merge has to change the filtered path or it elides too and there is
    // no derived merge left to inspect. Borrowing another injected commit's tree
    // is the cheapest way to get a different subtree under PREFIX.
    let elsewhere = world
        .upstream
        .commits
        .iter()
        .find(|(label, _)| *label == "left")
        .map(|(_, commit)| world.map.get(commit).expect("it was injected"))
        .expect("the fixture has a commit labeled left");
    let merge = append_commit(&world.repo, &[head, side], &tree_of(&world.repo, elsewhere));

    let mut cache = Cache::new();
    let base = jj_views::derive(&world.repo, &head, &filter, &mut cache)
        .expect("a derivation")
        .expect("the injected head has a view");
    let derived = jj_views::derive(&world.repo, &merge, &filter, &mut cache)
        .expect("a derivation")
        .expect("the merge has a view");

    assert_ne!(
        derived, base,
        "the merge changes the filtered path, so it must survive as its own commit"
    );
    let raw = world
        .repo
        .find_object(derived)
        .expect("the derived merge")
        .detach()
        .data;
    let parents: Vec<ObjectId> = gix::objs::CommitRef::from_bytes(&raw, world.repo.object_hash())
        .expect("a well formed commit")
        .parents()
        .collect();
    assert_eq!(
        parents,
        vec![base],
        "both sides derive to the same view commit, so the merge must keep one parent"
    );
}

/// The tree of the injected form of the fixture commit with this label.
fn injected_tree(world: &Injected, label: &str) -> ObjectId {
    let upstream = world
        .upstream
        .commits
        .iter()
        .find(|(candidate, _)| *candidate == label)
        .map(|(_, commit)| commit)
        .expect("the fixture has this commit");
    tree_of(
        &world.repo,
        world.map.get(upstream).expect("it was injected"),
    )
}

fn tree_of(repo: &gix::Repository, commit: &ObjectId) -> ObjectId {
    let raw = repo.find_object(*commit).expect("a commit").detach().data;
    gix::objs::CommitRef::from_bytes(&raw, repo.object_hash())
        .expect("a well formed commit")
        .tree()
}

/// A commit over `parents` with the given `tree`.
fn append_commit(repo: &gix::Repository, parents: &[ObjectId], tree: &ObjectId) -> ObjectId {
    let mut raw = format!("tree {tree}\n");
    for parent in parents {
        writeln!(raw, "parent {parent}").expect("writing to a String cannot fail");
    }
    raw.push_str(
        "author Monorepo <mono@example.invalid> 1000000100 +0000\ncommitter Monorepo \
         <mono@example.invalid> 1000000100 +0000\n\na merge\n",
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

/// A version number nobody checks is decoration. This pins the exact object id
/// each policy derives to, so any change to a hash affecting rule fails here
/// and names the version that has to be bumped rather than silently producing a
/// different history from the same input.
///
/// The scenario has to contain something each policy treats differently, or the
/// test pins nothing about the policies. So it is the fixture injected, then a
/// monorepo commit that touches nothing under the prefix, then a merge whose
/// filtered tree equals its first parent's. The three specs must therefore give
/// three DIFFERENT hashes, and that is asserted too.
///
/// If you are here because this test failed: do not update the constants. Add a
/// `Semantics` variant, keep the old one behaving as it did, and add a line.
#[test]
fn semantics_v1_output_hashes_are_pinned() {
    // The fixture's own tip, pinned separately so that a change to the fixture
    // fails with its own message instead of looking like a rule change.
    const FIXTURE_HEAD: &str = "4da40074cd503a78762551c225efd95e714a86f7";

    let pinned = [
        (
            "semantics=1;prefix=vendor/upstream;elide=unchanged;trivial-merges=drop",
            "66fd0506342cb5453fa460f045ecff8f0f14698c",
        ),
        (
            "semantics=1;prefix=vendor/upstream;elide=nothing;trivial-merges=drop",
            "981a00d7560fa5df6bffe2d8dc560dfa65b4c508",
        ),
        (
            "semantics=1;prefix=vendor/upstream;elide=unchanged;trivial-merges=keep",
            "2bcfd8401aa24a06ecc342de6846161a8c1d3964",
        ),
    ];

    let mut actual: Vec<String> = Vec::new();
    for (spec, _) in &pinned {
        let filter = Filter::parse(spec).expect("a spec this version can read");
        assert_eq!(&filter.spec(), spec, "a spec must round trip unchanged");
        let world = inject(&filter);
        assert_eq!(
            format!("{}", world.upstream.head),
            FIXTURE_HEAD,
            "the fixture history changed; that is a separate matter from a rule change"
        );

        let head = *world
            .map
            .get(&world.upstream.head)
            .expect("the head was injected");
        // Two trees whose vendored subtrees differ, borrowed from injected
        // commits, so a commit carrying one is a commit the filter must keep.
        // They have to differ from EACH OTHER as well: if both sides of the merge
        // below filtered to the same subtree, the elide rule would drop that
        // merge on its own and mask what the trivial merge policy does.
        let vendored = injected_tree(&world, "left");
        let vendored_other = injected_tree(&world, "right");

        // Two monorepo-only commits, which only the elide policy has an opinion
        // about, with a commit that does touch the vendored path between them so
        // that dropping them changes what its view is built on.
        let quiet_one = append_monorepo_commit(&world.repo, &head, 0);
        let touches = append_commit(&world.repo, &[quiet_one], &vendored);
        let quiet_two = append_monorepo_commit(&world.repo, &touches, 1);
        // A second line that also touches the vendored path, so the merge below
        // keeps two distinct derived parents and the trivial merge rule is
        // reachable at all.
        let other_line = append_commit(&world.repo, &[head], &vendored_other);
        // A merge resolving entirely to its first side under the filter, but not
        // before it: its tree differs from that side's outside the prefix, so the
        // "was already trivial" guard does not spare it and the trivial merge
        // policy decides.
        let trivial_source = append_monorepo_commit(&world.repo, &quiet_two, 2);
        let trivial = append_commit(
            &world.repo,
            &[quiet_two, other_line],
            &tree_of(&world.repo, &trivial_source),
        );

        let mut cache = Cache::new();
        let derived = jj_views::derive(&world.repo, &trivial, &filter, &mut cache)
            .expect("a derivation")
            .expect("the tip has a view");
        actual.push(format!("{derived}"));
    }

    let mut distinct = actual.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        actual.len(),
        "the three policies must produce three different hashes or this test pins nothing about \
         them: {actual:?}"
    );

    let expected: Vec<String> = pinned.iter().map(|(_, id)| (*id).to_owned()).collect();
    assert_eq!(
        actual, expected,
        "semantics v1 output hashes changed; add a Semantics variant rather than editing these \
         constants"
    );
}

/// A spec that a newer version wrote must fail loudly rather than being read as
/// though its unknown rules did not apply.
#[test]
fn a_spec_this_version_cannot_read_is_refused() {
    for spec in [
        // A semantics version from the future.
        "semantics=2;prefix=vendor/upstream;elide=unchanged;trivial-merges=drop",
        // A rule this version does not have.
        "semantics=1;prefix=vendor/upstream;elide=unchanged;trivial-merges=drop;squash=yes",
        // A missing field, which would otherwise silently take a default.
        "semantics=1;prefix=vendor/upstream;elide=unchanged",
        // A value this version does not know.
        "semantics=1;prefix=vendor/upstream;elide=sometimes;trivial-merges=drop",
    ] {
        assert!(
            Filter::parse(spec).is_err(),
            "{spec:?} must be refused, not read with defaults filled in"
        );
    }
}

/// josh's `--check-roundtrip`, as a test: take a commit of the view, lift it
/// back into the monorepo, filter it out again, and require the same commit
/// back.
///
/// The round trip test above covers this for commits that came from an
/// injection, where `unfilter` is doing nothing but wrapping trees. This covers
/// the case the design actually needs and which nothing else here reaches: a
/// view commit that DIVERGED from what was injected, which is what a local
/// patch on the vendored copy is, grafted onto a monorepo head that has moved
/// on its own since.
#[test]
fn a_divergent_view_commit_survives_unfilter_then_derive() {
    let filter = Filter::prefix(PREFIX).expect("a valid prefix");
    let world = inject(&filter);
    let injected_head = *world
        .map
        .get(&world.upstream.head)
        .expect("the head was injected");

    // The monorepo moves on its own, so the graft is not onto the commit the
    // view was derived from. This is the part that makes it a graft rather than
    // a continuation.
    let monorepo_head = append_monorepo_commit(&world.repo, &injected_head, 7);

    let mut cache = Cache::new();
    let view_head = jj_views::derive(&world.repo, &injected_head, &filter, &mut cache)
        .expect("a derivation")
        .expect("the head has a view");

    // A local patch, authored against the view: a commit whose tree is a
    // different vendored subtree, parented on the view's head.
    let patch = append_commit(&world.repo, &[view_head], &vendored_subtree(&world, "left"));

    let lifted = jj_views::unfilter(&world.repo, &patch, &monorepo_head, &filter, &mut cache)
        .expect("a prefix graft cannot conflict on content");

    // The lifted commit must sit on the monorepo head, and must carry the
    // monorepo's own files as well as the patched subtree, or the graft dropped
    // something outside the prefix.
    let lifted_raw = world
        .repo
        .find_object(lifted)
        .expect("the lifted commit")
        .detach()
        .data;
    let lifted_parents: Vec<ObjectId> =
        gix::objs::CommitRef::from_bytes(&lifted_raw, world.repo.object_hash())
            .expect("a well formed commit")
            .parents()
            .collect();
    assert_eq!(
        lifted_parents,
        vec![injected_head],
        "the patch's parent is the view head, whose counterpart is known, so the lifted commit \
         belongs on that and not on a monorepo head it has never seen"
    );
    let lifted_tree = tree_of(&world.repo, &lifted);
    assert!(
        has_entry(&world.repo, &lifted_tree, "MONOREPO"),
        "the graft must keep the monorepo's own files, not just the filtered subtree"
    );
    // The important negative, and it has to compare the content OUTSIDE the
    // prefix to mean anything. Comparing whole trees passes either way, because
    // the patched subtree differs from the monorepo head's regardless. What
    // distinguishes a graft from a fabrication is whose `MONOREPO` blob comes
    // through: its own parent's, or the one the monorepo changed behind it.
    assert_eq!(
        entry_oid(&world.repo, &lifted_tree, "MONOREPO"),
        entry_oid(
            &world.repo,
            &tree_of(&world.repo, &injected_head),
            "MONOREPO"
        ),
        "the lifted commit must carry its own parent's content outside the prefix"
    );
    assert_ne!(
        entry_oid(&world.repo, &lifted_tree, "MONOREPO"),
        entry_oid(
            &world.repo,
            &tree_of(&world.repo, &monorepo_head),
            "MONOREPO"
        ),
        "lifting must not absorb what the monorepo did behind this commit's back; those changes \
         would look like its own work and the monorepo head would be an ancestor of nothing"
    );

    let round_tripped = jj_views::derive(&world.repo, &lifted, &filter, &mut cache)
        .expect("a derivation")
        .expect("the lifted commit has a view");
    assert_eq!(
        round_tripped, patch,
        "filtering the lifted commit must give back the view commit it came from"
    );
}

/// Whether a tree has a top level entry with this name.
fn has_entry(repo: &gix::Repository, tree: &ObjectId, name: &str) -> bool {
    entry_oid(repo, tree, name).is_some()
}

/// The object a tree's top level entry points at.
fn entry_oid(repo: &gix::Repository, tree: &ObjectId, name: &str) -> Option<ObjectId> {
    let raw = repo.find_object(*tree).expect("a tree").detach().data;
    gix::objs::TreeRef::from_bytes(&raw, repo.object_hash())
        .expect("a well formed tree")
        .entries
        .iter()
        .find(|entry| entry.filename == name)
        .map(|entry| entry.oid.to_owned())
}

/// The filtered subtree of the injected form of the fixture commit with this
/// label, which is a valid tree for a view commit to carry.
fn vendored_subtree(world: &Injected, label: &str) -> ObjectId {
    let upstream = world
        .upstream
        .commits
        .iter()
        .find(|(candidate, _)| *candidate == label)
        .map(|(_, commit)| commit)
        .expect("the fixture has this commit");
    tree_of(&world.repo, upstream)
}
