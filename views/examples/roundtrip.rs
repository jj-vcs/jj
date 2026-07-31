//! Round trip hash identity, measured against a real repository.
//!
//! Injects every commit reachable from a source repository's refs under a
//! prefix, derives the prefix back out, and reports how many of the original
//! commit hashes came back. Every mismatch is classified, so the report says
//! which byte moved rather than only that something did.
//!
//! Objects are read from the source through a git alternate and written into a
//! fresh repository, so the source is never touched and no objects are copied.
//!
//! ```text
//! cargo run --release --example roundtrip -- <source.git> <work-dir> vendor/linux [--elide=RULE]
//! ```
//!
//! `--elide=` takes `nothing`, `unchanged` (the default, and josh's) or
//! `including-already-empty`, the rule that looks right and silently breaks
//! hash identity.
//!
//! `--survey-only` skips the injection and reports just which commit shapes the
//! source contains. That works on a treeless clone (`--filter=tree:0`), so the
//! question "does this repository contain commits a naive rewrite corrupts" can
//! be answered for a repository far too large to inject.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::Path;
use std::path::PathBuf;

use gix::ObjectId;
use jj_views::Cache;
use jj_views::Elide;
use jj_views::Filter;
use jj_views::fixture;

type Failure = Box<dyn std::error::Error + Send + Sync>;

fn main() -> Result<(), Failure> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [source, work, path, rest @ ..] = args.as_slice() else {
        return Err(
            "usage: roundtrip <source.git> <work-dir> <prefix> [--elide=RULE] [--survey-only]"
                .into(),
        );
    };
    let rule = rest
        .iter()
        .filter_map(|arg| arg.strip_prefix("--elide="))
        .next_back()
        .unwrap_or("unchanged");
    let elide = match rule {
        "nothing" => Elide::Nothing,
        "unchanged" => Elide::Unchanged,
        "including-already-empty" => Elide::UnchangedIncludingAlreadyEmpty,
        other => return Err(format!("unknown elide rule {other:?}").into()),
    };
    let survey_only = rest.iter().any(|arg| arg == "--survey-only");

    let source = gix::open(source)?;
    let filter = Filter::prefix(path)?.elide(elide);

    // Surveying reads only, so it neither needs nor may create the work
    // directory; doing so would clobber a concurrent injection using it.
    if survey_only {
        let order = topological_order(&source)?;
        println!("commits reachable from refs: {}", order.len());
        return survey(&source, &order);
    }

    let parent = open_parent(Path::new(work), source.git_dir(), rule)?;
    println!(
        "source {:?}\nparent {:?}\nfilter {} (elide={rule})",
        source.git_dir(),
        parent.git_dir(),
        filter.path()
    );

    let order = topological_order(&source)?;
    println!("upstream commits reachable from refs: {}", order.len());

    // A monorepo the vendored history lands inside, so the injected trees are
    // not just the prefix and the experiment is not accidentally easier than
    // the real case.
    let base = write_base(&parent)?;

    let mut inject = Cache::new();
    let mut injected: HashMap<ObjectId, ObjectId> = HashMap::new();
    let started = std::time::Instant::now();
    for upstream in &order {
        let raw = parent.find_object(*upstream)?.detach().data;
        let first_parent = gix::objs::CommitRef::from_bytes(&raw, parent.object_hash())?
            .parents()
            .next();
        let onto = first_parent
            .and_then(|first| injected.get(&first).copied())
            .unwrap_or(base);
        let id = jj_views::unfilter(&parent, upstream, &onto, &filter, &mut inject)?;
        injected.insert(*upstream, id);
    }
    println!(
        "injected {} commits under {} in {:.1}s",
        injected.len(),
        filter.path(),
        started.elapsed().as_secs_f64()
    );

    let mut derive_cache = Cache::new();
    let started = std::time::Instant::now();
    for upstream in &order {
        let Some(id) = injected.get(upstream) else {
            continue;
        };
        jj_views::derive(&parent, id, &filter, &mut derive_cache)?;
    }
    println!(
        "derived in {:.1}s, cache holds {} trees and {} commits",
        started.elapsed().as_secs_f64(),
        derive_cache.tree_entries(),
        derive_cache.commit_entries()
    );

    report(&parent, &order, &injected, &filter, &derive_cache)
}

/// Compares every derived commit against the upstream commit it came from and
/// prints the tally plus a cause for each mismatch.
fn report(
    parent: &gix::Repository,
    order: &[ObjectId],
    injected: &HashMap<ObjectId, ObjectId>,
    filter: &Filter,
    cache: &Cache,
) -> Result<(), Failure> {
    let mut matched = 0_usize;
    let mut absent = 0_usize;
    let mut elided = 0_usize;
    let mut inherited = 0_usize;
    let mut collapsed = 0_usize;
    // Cause to (count, one example), so a systematic failure reports once.
    let mut primary: HashMap<String, (usize, ObjectId)> = HashMap::new();
    let mut naive_would_break: HashMap<String, usize> = HashMap::new();

    for upstream in order {
        let raw = parent.find_object(*upstream)?.detach().data;
        if !fixture::naive_rebuild_matches(&raw, parent.object_hash()) {
            *naive_would_break
                .entry(header_summary(&raw, parent.object_hash()))
                .or_default() += 1;
        }

        let Some(id) = injected.get(upstream) else {
            continue;
        };
        let derived = cache.derived(id, filter).flatten();
        let Some(derived) = derived else {
            absent += 1;
            continue;
        };
        if derived == *upstream {
            matched += 1;
            continue;
        }
        // Elision maps a commit onto one of its own parents, so it has no
        // distinct counterpart. That is not the same failure as a commit that
        // was rebuilt wrongly, and counting them together hides which is which.
        let parents: Vec<ObjectId> = gix::objs::CommitRef::from_bytes(&raw, parent.object_hash())?
            .parents()
            .collect();
        let onto_a_parent = parents.iter().any(|source| {
            injected
                .get(source)
                .and_then(|injected| cache.derived(injected, filter).flatten())
                == Some(derived)
        });
        if onto_a_parent {
            elided += 1;
            continue;
        }
        let actual = parent.find_object(derived)?.detach().data;
        // Separate a commit whose own bytes did not survive from one that only
        // inherited a parent whose hash had already moved, and from a merge
        // whose parent list got shorter because elision collapsed two sides
        // onto one commit.
        let expected_parents = count_parent_lines(&raw);
        if same_but_for_parents(&raw, &actual) {
            inherited += 1;
        } else if expected_parents != count_parent_lines(&actual) {
            collapsed += 1;
        } else {
            let cause = fixture::describe_difference(&raw, &actual);
            let entry = primary.entry(cause).or_insert((0, *upstream));
            entry.0 += 1;
        }
    }

    let total = order.len();
    println!("\n=== round trip against {total} upstream commits");
    println!("  matched byte for byte:            {matched}");
    println!("  no counterpart at all:            {absent}");
    println!("  elided onto one of their parents: {elided}");
    println!("  moved, inherited a parent:        {inherited}");
    println!("  moved, merge arity collapsed:     {collapsed}");
    let primary_total: usize = primary.values().map(|(count, _)| *count).sum();
    println!("  moved in their own bytes:         {primary_total}");
    for (cause, (count, example)) in &primary {
        println!("    {count} commits, e.g. {example}: {cause}");
    }

    print_naive_tally(&naive_would_break, total);
    Ok(())
}

/// Reports which commit shapes a repository contains, reading commits only.
fn survey(repo: &gix::Repository, order: &[ObjectId]) -> Result<(), Failure> {
    let mut naive_would_break: HashMap<String, usize> = HashMap::new();
    let mut shapes: HashMap<String, usize> = HashMap::new();
    let mut offsets: HashMap<String, usize> = HashMap::new();
    for id in order {
        let raw = repo.find_object(*id)?.detach().data;
        if !fixture::naive_rebuild_matches(&raw, repo.object_hash()) {
            *naive_would_break
                .entry(header_summary(&raw, repo.object_hash()))
                .or_default() += 1;
        }
        let parsed = gix::objs::CommitRef::from_bytes(&raw, repo.object_hash())?;
        let parents: Vec<ObjectId> = parsed.parents().collect();
        let mut unique = parents.clone();
        unique.sort();
        unique.dedup();
        if unique.len() < parents.len() {
            *shapes.entry("duplicate parents".to_owned()).or_default() += 1;
        }
        if parents.len() > 2 {
            *shapes.entry("octopus merge".to_owned()).or_default() += 1;
        }
        for (name, _) in &parsed.extra_headers {
            // Named rather than bucketed. A header nobody expected is exactly
            // the thing a survey is for, and "some other header" hides it.
            *shapes.entry(format!("{name} header")).or_default() += 1;
        }
        if parsed.encoding.is_some() {
            *shapes.entry("encoding header".to_owned()).or_default() += 1;
        }
        if let Some(offset) = parsed.author.rsplit(|byte| *byte == b' ').next()
            && offset.len() != 5
        {
            *offsets
                .entry(String::from_utf8_lossy(offset).into_owned())
                .or_default() += 1;
        }
    }

    println!("\n=== shapes among {} commits", order.len());
    let mut counted: Vec<(&String, &usize)> = shapes.iter().collect();
    counted.sort_by(|left, right| right.1.cmp(left.1));
    for (shape, count) in counted {
        println!("  {count:>8} {shape}");
    }
    println!("\n=== author timezone offsets that are not [+-]NNNN");
    if offsets.is_empty() {
        println!("  none");
    }
    let mut counted: Vec<(&String, &usize)> = offsets.iter().collect();
    counted.sort_by(|left, right| right.1.cmp(left.1));
    for (offset, count) in counted.iter().take(20) {
        println!("  {count:>8} {offset:?}");
    }
    print_naive_tally(&naive_would_break, order.len());
    Ok(())
}

fn print_naive_tally(tally: &HashMap<String, usize>, total: usize) {
    println!("\n=== commits a parse-and-reserialize rewrite would corrupt");
    let broken: usize = tally.values().sum();
    println!("  {broken} of {total}");
    let mut summaries: Vec<(&String, &usize)> = tally.iter().collect();
    summaries.sort_by(|left, right| right.1.cmp(left.1));
    for (summary, count) in summaries {
        println!("    {count} carrying {summary}");
    }
}

/// How many `parent` lines a commit object carries.
fn count_parent_lines(raw: &[u8]) -> usize {
    raw.split_inclusive(|byte| *byte == b'\n')
        .filter(|line| line.starts_with(b"parent "))
        .count()
}

/// True when two commit objects differ only in their `parent` lines.
fn same_but_for_parents(expected: &[u8], actual: &[u8]) -> bool {
    let strip = |raw: &[u8]| -> Vec<u8> {
        raw.split_inclusive(|byte| *byte == b'\n')
            .filter(|line| !line.starts_with(b"parent "))
            .flatten()
            .copied()
            .collect()
    };
    strip(expected) == strip(actual)
}

/// Which extra headers a commit carries, as a stable label for grouping.
fn header_summary(raw: &[u8], hash: gix::hash::Kind) -> String {
    let Ok(parsed) = gix::objs::CommitRef::from_bytes(raw, hash) else {
        return "a header block that will not parse".to_owned();
    };
    let mut names: Vec<String> = parsed
        .extra_headers
        .iter()
        .map(|(name, _)| name.to_string())
        .collect();
    if parsed.encoding.is_some() {
        names.push("encoding".to_owned());
    }
    if names.is_empty() {
        let mut out = String::from("no extra headers (author or committer line)");
        if let Some(author) = parsed.author.split(|byte| *byte == b'>').next_back() {
            let _ = write!(out, ", author suffix {:?}", String::from_utf8_lossy(author));
        }
        return out;
    }
    names.sort();
    names.join(" + ")
}

/// A fresh repository that reads the source's objects through an alternate and
/// writes only its own.
fn open_parent(work: &Path, source_objects: &Path, rule: &str) -> Result<gix::Repository, Failure> {
    std::fs::create_dir_all(work)?;
    let dir: PathBuf = work.join(format!("parent-elide-{rule}.git"));
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    gix::init_bare(&dir)?;
    let alternates = dir.join("objects").join("info").join("alternates");
    std::fs::write(
        &alternates,
        format!("{}\n", source_objects.join("objects").display()),
    )?;
    Ok(gix::open(&dir)?)
}

/// A monorepo commit for the vendored history to sit inside.
fn write_base(repo: &gix::Repository) -> Result<ObjectId, Failure> {
    let blob = repo.write_blob(b"the monorepo\n")?.detach();
    let tree = repo.write_object(&gix::objs::Tree {
        entries: vec![gix::objs::tree::Entry {
            mode: gix::objs::tree::EntryKind::Blob.into(),
            filename: "README".into(),
            oid: blob,
        }],
    })?;
    let raw = format!(
        "tree {}\nauthor Monorepo <mono@example.invalid> 1000000000 +0000\ncommitter Monorepo \
         <mono@example.invalid> 1000000000 +0000\n\nthe monorepo\n",
        tree.detach()
    );
    gix::objs::Write::write_buf(&repo.objects, gix::objs::Kind::Commit, raw.as_bytes())
}

/// Every commit reachable from any ref, parents before children.
fn topological_order(repo: &gix::Repository) -> Result<Vec<ObjectId>, Failure> {
    let mut tips: Vec<ObjectId> = Vec::new();
    for reference in repo.references()?.all()?.filter_map(Result::ok) {
        let mut reference = reference;
        if let Ok(id) = reference.peel_to_id()
            && let Ok(object) = id.object()
            && object.kind == gix::objs::Kind::Commit
        {
            tips.push(object.id);
        }
    }
    tips.sort();
    tips.dedup();

    let mut order: Vec<ObjectId> = Vec::new();
    let mut done: HashSet<ObjectId> = HashSet::new();
    let mut stack: Vec<(ObjectId, bool)> = tips.into_iter().map(|tip| (tip, false)).collect();
    while let Some((id, expanded)) = stack.pop() {
        if done.contains(&id) {
            continue;
        }
        if expanded {
            done.insert(id);
            order.push(id);
            continue;
        }
        let raw = repo.find_object(id)?.detach().data;
        let parents: Vec<ObjectId> = gix::objs::CommitRef::from_bytes(&raw, repo.object_hash())?
            .parents()
            .filter(|parent| !done.contains(parent))
            .collect();
        stack.push((id, true));
        stack.extend(parents.into_iter().map(|parent| (parent, false)));
    }
    Ok(order)
}
