//! A deterministic upstream history built from raw objects, plus the byte level
//! diagnostics used to explain a hash that moved.
//!
//! This exists so the round trip check is a test rather than an errand. It
//! reaches no network, it produces the same object ids on every run and every
//! platform, and it contains the shapes that a naive rewrite gets wrong. The
//! commits are assembled byte by byte rather than through git plumbing, because
//! some of the interesting cases cannot be produced by the git CLI at all: an
//! `encoding` header placed after `gpgsig`, and a timezone offset outside the
//! range a normalizing serializer can reproduce.
//!
//! What it covers, one commit each unless noted: a root commit, a non-ASCII
//! author and message, an `encoding` header with a non-UTF-8 message body, a
//! `gpgsig` header, `gpgsig` *before* `encoding`, a negative zero timezone
//! offset, an offset no hour and minute pair can hold, an empty commit whose
//! tree equals its parent's, a mode change from `100644` to `100755`, a
//! symlink, a submodule gitlink pointing at an absent commit, two branches
//! joined by a merge carrying a `mergetag`, an octopus merge with three
//! parents, a commit listing the same parent twice, a message with no trailing
//! newline, and sibling names (`foo` as a directory next to `foo.txt`) whose
//! tree order differs between plain byte ordering and git's.

use std::collections::BTreeMap;

use bstr::BString;
use bstr::ByteSlice as _;
use gix::ObjectId;
use gix::objs::Write as _;

use crate::Error;

/// The upstream history written by [`write_upstream`].
pub struct Upstream {
    /// Every commit, in an order where parents precede children, paired with
    /// the label naming the corner case it carries.
    pub commits: Vec<(&'static str, ObjectId)>,
    /// The tip, the commit whose ancestry is the whole fixture.
    pub head: ObjectId,
}

/// One file in a snapshot.
#[derive(Clone, Copy)]
enum Content {
    /// A regular file, mode `100644`.
    File(&'static [u8]),
    /// An executable file, mode `100755`.
    Exec(&'static [u8]),
    /// A symlink, mode `120000`, whose blob holds the target path.
    Link(&'static str),
    /// A submodule, mode `160000`. The commit it names is deliberately absent
    /// from the repository, which is the normal state of a gitlink.
    Gitlink,
}

/// A gitlink target that exists in no repository, so the fixture also proves a
/// dangling `160000` entry survives the round trip.
const ABSENT_SUBMODULE: &str = "0123456789abcdef0123456789abcdef01234567";

/// A multi-line header value shaped like what `git commit -S` writes, one
/// element per line.
///
/// Held as lines rather than as one string with `\n` escapes because rustfmt's
/// `format_strings` reflows a long literal and has been observed splitting an
/// escape sequence across the break, which silently changes the value. Lines
/// also match the shape of the on disk format, which is per line folding.
///
/// The bytes are not a real signature and are not meant to verify. What matters
/// is that this is a header git stores verbatim and a rewrite must not touch.
const FAKE_SIGNATURE: &[&str] = &[
    "-----BEGIN PGP SIGNATURE-----",
    "",
    "iQEcBAABCgAGBQJmAAAAAAoJEAAAAAAAAAAAdGVzdA==",
    "=abcd",
    "-----END PGP SIGNATURE-----",
];

/// A `mergetag` value, the shape git writes when merging a signed tag. The
/// blank line is the separator inside the embedded tag object, and it folds to
/// a line holding a single space.
const FAKE_MERGETAG: &[&str] = &[
    "object 0000000000000000000000000000000000000000",
    "type commit",
    "tag v1.0",
    "tagger Tag Signer <tag@example.invalid> 1200000000 +0000",
    "",
    "release v1.0",
];

struct CommitSpec {
    tree: ObjectId,
    parents: Vec<ObjectId>,
    /// The raw `author` value, spelled exactly as it must appear on disk.
    author: &'static str,
    /// The raw `committer` value.
    committer: &'static str,
    /// Extra headers in the exact order they must be emitted, after
    /// `committer`, each value given as its lines and folded on write.
    headers: &'static [(&'static str, &'static [&'static str])],
    message: &'static [u8],
}

/// Writes the fixture history into `repo` and returns its commits.
///
/// `repo` may be bare. Nothing but objects is written, so no refs are created
/// and the caller decides what to point at [`Upstream::head`].
pub fn write_upstream(repo: &gix::Repository) -> Result<Upstream, Error> {
    let mut files: BTreeMap<&'static str, Content> = BTreeMap::new();
    let mut commits: Vec<(&'static str, ObjectId)> = Vec::new();

    // A plain root commit, so the interesting commits all have a parent.
    files.insert("README", Content::File(b"upstream\n"));
    files.insert("src/main.rs", Content::File(b"fn main() {}\n"));
    let root = write_commit(
        repo,
        &CommitSpec {
            tree: write_snapshot(repo, &files)?,
            parents: vec![],
            author: "Ada Lovelace <ada@example.invalid> 1100000000 +0000",
            committer: "Ada Lovelace <ada@example.invalid> 1100000000 +0000",
            headers: &[],
            message: b"root commit\n",
        },
    )?;
    commits.push(("root", root));

    // Non-ASCII in both the author name and the message, stored as UTF-8.
    files.insert(
        "docs/\u{e9}t\u{e9}.txt",
        Content::File("caf\u{e9}\n".as_bytes()),
    );
    let non_ascii = write_commit(
        repo,
        &CommitSpec {
            tree: write_snapshot(repo, &files)?,
            parents: vec![root],
            author: "Bj\u{f6}rn \u{5f20}\u{4f1f} <bjorn@example.invalid> 1100000100 +0200",
            committer: "Bj\u{f6}rn \u{5f20}\u{4f1f} <bjorn@example.invalid> 1100000100 +0200",
            headers: &[],
            message: "add caf\u{e9} \u{2014} \u{4e2d}\u{6587}\n".as_bytes(),
        },
    )?;
    commits.push(("non-ascii", non_ascii));

    // An `encoding` header, with a message body that is Latin-1 and therefore
    // not valid UTF-8. A rewrite that goes through `String` loses these bytes.
    files.insert("CHANGELOG", Content::File(b"1\n"));
    let encoded = write_commit(
        repo,
        &CommitSpec {
            tree: write_snapshot(repo, &files)?,
            parents: vec![non_ascii],
            author: "Ren\u{e9} <rene@example.invalid> 1100000200 +0100",
            committer: "Ren\u{e9} <rene@example.invalid> 1100000200 +0100",
            headers: &[("encoding", &["ISO-8859-1"])],
            // 0xE9 is `e` acute in Latin-1 and an invalid UTF-8 sequence.
            message: b"caf\xe9 in latin-1\n",
        },
    )?;
    commits.push(("encoding-latin1", encoded));

    // A signature header on its own.
    files.insert("src/lib.rs", Content::File(b"pub fn f() {}\n"));
    let signed = write_commit(
        repo,
        &CommitSpec {
            tree: write_snapshot(repo, &files)?,
            parents: vec![encoded],
            author: "Ada Lovelace <ada@example.invalid> 1100000300 +0000",
            committer: "Ada Lovelace <ada@example.invalid> 1100000300 +0000",
            headers: &[("gpgsig", FAKE_SIGNATURE)],
            message: b"signed commit\n",
        },
    )?;
    commits.push(("gpgsig", signed));

    // `gpgsig` before `encoding`. A representation with a dedicated `encoding`
    // field emits it directly after `committer`, so this order can only survive
    // if the parser leaves `encoding` in the general extra header list when it
    // is not the first header. gix does exactly that, so this case does round
    // trip; it is kept because a regression would be silent and would move the
    // hash of every commit after it.
    files.insert("CHANGELOG", Content::File(b"1\n2\n"));
    let signed_then_encoding = write_commit(
        repo,
        &CommitSpec {
            tree: write_snapshot(repo, &files)?,
            parents: vec![signed],
            author: "Ada Lovelace <ada@example.invalid> 1100000400 +0000",
            committer: "Ada Lovelace <ada@example.invalid> 1100000400 +0000",
            headers: &[("gpgsig", FAKE_SIGNATURE), ("encoding", &["ISO-8859-1"])],
            message: b"signed, then encoded\n",
        },
    )?;
    commits.push(("gpgsig-before-encoding", signed_then_encoding));

    // A negative zero timezone offset. git accepts it and `git fsck` is happy
    // with it, so this shape is present in ordinary history; it is nonetheless
    // distinct from `+0000` on disk and identical to it after parsing.
    files.insert("src/main.rs", Content::File(b"fn main() { f() }\n"));
    let negative_zero = write_commit(
        repo,
        &CommitSpec {
            tree: write_snapshot(repo, &files)?,
            parents: vec![signed_then_encoding],
            author: "Neg Zero <neg@example.invalid> 1100000500 -0000",
            committer: "Neg Zero <neg@example.invalid> 1100000500 -0000",
            headers: &[],
            message: b"a negative zero offset\n",
        },
    )?;
    commits.push(("negative-zero-offset", negative_zero));

    // A timezone offset outside what a `(hours, minutes)` pair can hold: five
    // hours eighteen minutes and zero seconds, spelled with six digits. `git
    // fsck` reports `badTimezone` for this and that is the point. The check
    // exists because such commits are in the wild, importers produced them, and
    // a filter that must preserve hashes has to carry them through unaltered
    // rather than quietly repairing them.
    files.insert("NOTES", Content::File(b"imported\n"));
    let odd_offset = write_commit(
        repo,
        &CommitSpec {
            tree: write_snapshot(repo, &files)?,
            parents: vec![negative_zero],
            author: "Odd Zone <odd@example.invalid> 1100000600 +051800",
            committer: "Odd Zone <odd@example.invalid> 1100000600 +051800",
            headers: &[],
            message: b"an offset no hour and minute pair can hold\n",
        },
    )?;
    commits.push(("odd-timezone-offset", odd_offset));

    // An empty commit: same tree as its parent. This is the shape that empty
    // commit elision drops, and dropping it is what breaks hash identity.
    let empty = write_commit(
        repo,
        &CommitSpec {
            tree: write_snapshot(repo, &files)?,
            parents: vec![odd_offset],
            author: "Ada Lovelace <ada@example.invalid> 1100000600 +0000",
            committer: "Ada Lovelace <ada@example.invalid> 1100000600 +0000",
            headers: &[],
            message: b"deliberately empty\n",
        },
    )?;
    commits.push(("empty", empty));

    // A mode change with no content change, a symlink, and a gitlink whose
    // target is absent.
    files.insert("src/main.rs", Content::Exec(b"fn main() { f() }\n"));
    files.insert("link", Content::Link("README"));
    files.insert("vendor/submodule", Content::Gitlink);
    let modes = write_commit(
        repo,
        &CommitSpec {
            tree: write_snapshot(repo, &files)?,
            parents: vec![empty],
            author: "Ada Lovelace <ada@example.invalid> 1100000700 +0000",
            committer: "Ada Lovelace <ada@example.invalid> 1100000700 +0000",
            headers: &[],
            message: b"modes, symlink, gitlink\n",
        },
    )?;
    commits.push(("modes-symlink-gitlink", modes));

    // Sibling names whose git ordering differs from plain byte ordering: `foo`
    // as a directory sorts as `foo/`, and `/` is above `.`, so `foo.txt` comes
    // first. A tree writer using plain byte order writes them the other way and
    // produces a different tree hash.
    files.insert("foo.txt", Content::File(b"file\n"));
    files.insert("foo/inner", Content::File(b"inner\n"));
    let siblings = write_commit(
        repo,
        &CommitSpec {
            tree: write_snapshot(repo, &files)?,
            parents: vec![modes],
            author: "Ada Lovelace <ada@example.invalid> 1100000800 +0000",
            committer: "Ada Lovelace <ada@example.invalid> 1100000800 +0000",
            headers: &[],
            // No trailing newline, which git preserves.
            message: b"foo vs foo.txt",
        },
    )?;
    commits.push(("tree-order-siblings", siblings));

    // Two side branches off `siblings`, to build merges from.
    let mut left_files = files.clone();
    left_files.insert("left", Content::File(b"left\n"));
    let left = write_commit(
        repo,
        &CommitSpec {
            tree: write_snapshot(repo, &left_files)?,
            parents: vec![siblings],
            author: "Left Author <left@example.invalid> 1100000900 -0700",
            committer: "Left Author <left@example.invalid> 1100000900 -0700",
            headers: &[],
            message: b"left side\n",
        },
    )?;
    commits.push(("left", left));

    let mut right_files = files.clone();
    right_files.insert("right", Content::File(b"right\n"));
    let right = write_commit(
        repo,
        &CommitSpec {
            tree: write_snapshot(repo, &right_files)?,
            parents: vec![siblings],
            author: "Right Author <right@example.invalid> 1100001000 +0530",
            committer: "Right Author <right@example.invalid> 1100001000 +0530",
            headers: &[],
            message: b"right side\n",
        },
    )?;
    commits.push(("right", right));

    let mut third_files = files.clone();
    third_files.insert("third", Content::File(b"third\n"));
    let third = write_commit(
        repo,
        &CommitSpec {
            tree: write_snapshot(repo, &third_files)?,
            parents: vec![siblings],
            author: "Third Author <third@example.invalid> 1100001100 +0000",
            committer: "Third Author <third@example.invalid> 1100001100 +0000",
            headers: &[],
            message: b"third side\n",
        },
    )?;
    commits.push(("third", third));

    // A merge carrying a `mergetag`, the header git adds when merging a signed
    // tag. It is folded over many lines and includes blank lines, so it also
    // exercises the fold and unfold path.
    let mut merged_files = left_files.clone();
    merged_files.insert("right", Content::File(b"right\n"));
    let merge = write_commit(
        repo,
        &CommitSpec {
            tree: write_snapshot(repo, &merged_files)?,
            parents: vec![left, right],
            author: "Ada Lovelace <ada@example.invalid> 1100001200 +0000",
            committer: "Ada Lovelace <ada@example.invalid> 1100001200 +0000",
            headers: &[("mergetag", FAKE_MERGETAG), ("gpgsig", FAKE_SIGNATURE)],
            message: b"merge left and right\n",
        },
    )?;
    commits.push(("merge-with-mergetag", merge));

    // An octopus merge.
    let mut octopus_files = merged_files.clone();
    octopus_files.insert("third", Content::File(b"third\n"));
    let octopus = write_commit(
        repo,
        &CommitSpec {
            tree: write_snapshot(repo, &octopus_files)?,
            parents: vec![merge, third, siblings],
            author: "Ada Lovelace <ada@example.invalid> 1100001300 +0000",
            committer: "Ada Lovelace <ada@example.invalid> 1100001300 +0000",
            headers: &[],
            message: b"octopus merge\n",
        },
    )?;
    commits.push(("octopus", octopus));

    // The same parent listed twice. git accepts this and preserves it; a
    // rewrite that deduplicates parents turns it into a non-merge and moves
    // its hash.
    let twins = write_commit(
        repo,
        &CommitSpec {
            tree: write_snapshot(repo, &octopus_files)?,
            parents: vec![octopus, octopus],
            author: "Ada Lovelace <ada@example.invalid> 1100001400 +0000",
            committer: "Ada Lovelace <ada@example.invalid> 1100001400 +0000",
            headers: &[],
            message: b"the same parent twice\n",
        },
    )?;
    commits.push(("duplicate-parents", twins));

    Ok(Upstream {
        commits,
        head: twins,
    })
}

/// Whether a parse and reserialize round trip through the owned commit
/// representation reproduces `raw` byte for byte.
///
/// This is the naive rewrite, and the reason the real one works on raw bytes.
#[must_use]
pub fn naive_rebuild_matches(raw: &[u8], hash: gix::hash::Kind) -> bool {
    let Ok(parsed) = gix::objs::CommitRef::from_bytes(raw, hash) else {
        return false;
    };
    let Ok(owned) = gix::objs::Commit::try_from(parsed) else {
        return false;
    };
    let mut out = Vec::with_capacity(raw.len());
    if gix::objs::WriteTo::write_to(&owned, &mut out).is_err() {
        return false;
    }
    out == raw
}

/// Names the first difference between two commit objects, for a report that
/// says which byte moved rather than only that a hash did.
#[must_use]
pub fn describe_difference(expected: &[u8], actual: &[u8]) -> String {
    if expected == actual {
        return "identical".to_owned();
    }
    let at = expected
        .iter()
        .zip(actual)
        .position(|(left, right)| left != right)
        .unwrap_or_else(|| expected.len().min(actual.len()));
    let line_of = |bytes: &[u8]| -> String {
        let start = bytes.get(..at).map_or(0, |head| {
            head.iter()
                .rposition(|byte| *byte == b'\n')
                .map_or(0, |nl| nl + 1)
        });
        let rest = bytes.get(start..).unwrap_or_default();
        let end = rest
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap_or(rest.len());
        rest.get(..end).unwrap_or_default().as_bstr().to_string()
    };
    format!(
        "first differing byte at offset {at}; expected line {:?}, got {:?}",
        line_of(expected),
        line_of(actual)
    )
}

/// Writes the blobs and trees for a snapshot and returns its root tree.
fn write_snapshot(
    repo: &gix::Repository,
    files: &BTreeMap<&'static str, Content>,
) -> Result<ObjectId, Error> {
    let mut leaves: Vec<(Vec<BString>, gix::objs::tree::EntryKind, ObjectId)> = Vec::new();
    for (path, content) in files {
        let components: Vec<BString> = path
            .split('/')
            .map(|part| BString::from(part.as_bytes()))
            .collect();
        let (kind, oid) = match *content {
            Content::File(bytes) => (
                gix::objs::tree::EntryKind::Blob,
                repo.objects.write_buf(gix::objs::Kind::Blob, bytes)?,
            ),
            Content::Exec(bytes) => (
                gix::objs::tree::EntryKind::BlobExecutable,
                repo.objects.write_buf(gix::objs::Kind::Blob, bytes)?,
            ),
            Content::Link(target) => (
                gix::objs::tree::EntryKind::Link,
                repo.objects
                    .write_buf(gix::objs::Kind::Blob, target.as_bytes())?,
            ),
            Content::Gitlink => (
                gix::objs::tree::EntryKind::Commit,
                ObjectId::from_hex(ABSENT_SUBMODULE.as_bytes())
                    .expect("fixture constant is valid hex"),
            ),
        };
        leaves.push((components, kind, oid));
    }
    write_tree(repo, &leaves, 0)
}

fn write_tree(
    repo: &gix::Repository,
    leaves: &[(Vec<BString>, gix::objs::tree::EntryKind, ObjectId)],
    depth: usize,
) -> Result<ObjectId, Error> {
    let mut entries: Vec<gix::objs::tree::Entry> = Vec::new();
    let mut at = 0;
    while at < leaves.len() {
        let Some((path, kind, oid)) = leaves.get(at) else {
            break;
        };
        let Some(name) = path.get(depth) else {
            at += 1;
            continue;
        };
        if path.len() == depth + 1 {
            entries.push(gix::objs::tree::Entry {
                mode: (*kind).into(),
                filename: name.clone(),
                oid: *oid,
            });
            at += 1;
            continue;
        }
        // The input is sorted by path, so every leaf under this directory is a
        // contiguous run starting here.
        let end = leaves
            .iter()
            .skip(at)
            .position(|(other, _, _)| other.get(depth) != Some(name))
            .map_or(leaves.len(), |offset| at + offset);
        let group = leaves.get(at..end).unwrap_or_default();
        entries.push(gix::objs::tree::Entry {
            mode: gix::objs::tree::EntryKind::Tree.into(),
            filename: name.clone(),
            oid: write_tree(repo, group, depth + 1)?,
        });
        at = end;
    }
    entries.sort();
    Ok(repo.objects.write(&gix::objs::Tree { entries })?)
}

fn write_commit(repo: &gix::Repository, spec: &CommitSpec) -> Result<ObjectId, Error> {
    let mut out = Vec::new();
    let mut hex = [0_u8; gix::hash::Kind::longest().len_in_hex()];
    out.extend_from_slice(b"tree ");
    out.extend_from_slice(spec.tree.hex_to_buf(&mut hex).as_bytes());
    out.push(b'\n');
    for parent in &spec.parents {
        out.extend_from_slice(b"parent ");
        out.extend_from_slice(parent.hex_to_buf(&mut hex).as_bytes());
        out.push(b'\n');
    }
    out.extend_from_slice(b"author ");
    out.extend_from_slice(spec.author.as_bytes());
    out.push(b'\n');
    out.extend_from_slice(b"committer ");
    out.extend_from_slice(spec.committer.as_bytes());
    out.push(b'\n');
    for (name, lines) in spec.headers {
        out.extend_from_slice(name.as_bytes());
        // git folds a multi-line header value by prefixing every continuation
        // line with one space, so a blank line inside the value becomes a line
        // holding a single space.
        for line in *lines {
            out.push(b' ');
            out.extend_from_slice(line.as_bytes());
            out.push(b'\n');
        }
    }
    out.push(b'\n');
    out.extend_from_slice(spec.message);
    Ok(repo.objects.write_buf(gix::objs::Kind::Commit, &out)?)
}
