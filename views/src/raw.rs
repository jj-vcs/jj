//! Byte level surgery on the identifier block at the head of a commit object.

use gix::ObjectId;
use gix::hash::oid;

use crate::Error;

/// Replaces the `tree` and `parent` lines of a commit object, copying every
/// remaining byte through unchanged.
///
/// Rebuilding the commit from a parsed representation instead loses the hash.
/// git accepts, and stores verbatim, author and committer lines that a parse
/// and reserialize round trip normalizes away: timezone offsets outside the
/// range a `(hours, minutes)` pair can hold, a negative zero offset, and
/// timestamps too large for the parser's integer type. It also preserves the
/// relative order of `encoding`, `gpgsig` and `mergetag`, which a struct with
/// a dedicated `encoding` field cannot express. Either difference moves the
/// commit hash, and a filtered history whose hashes moved no longer shares
/// ancestry with upstream, which is the whole point of deriving it.
pub(crate) fn replace_ids(raw: &[u8], tree: &oid, parents: &[ObjectId]) -> Result<Vec<u8>, Error> {
    let tail_at = ids_end(raw)?;
    let tail = raw.get(tail_at..).ok_or(Error::MalformedCommit)?;

    let hex_len = tree.kind().len_in_hex();
    let line_len = |name: usize| name + 1 + hex_len + 1;
    let mut out = Vec::with_capacity(line_len(4) + parents.len() * line_len(6) + tail.len());
    write_id_line(&mut out, b"tree", tree);
    for parent in parents {
        write_id_line(&mut out, b"parent", parent);
    }
    out.extend_from_slice(tail);
    Ok(out)
}

/// Byte offset just past the last `parent` line, or past the `tree` line when
/// the commit is a root.
fn ids_end(raw: &[u8]) -> Result<usize, Error> {
    let mut cursor = skip_header_line(raw, 0, b"tree").ok_or(Error::MalformedCommit)?;
    while let Some(next) = skip_header_line(raw, cursor, b"parent") {
        cursor = next;
    }
    Ok(cursor)
}

fn skip_header_line(raw: &[u8], cursor: usize, name: &[u8]) -> Option<usize> {
    let rest = raw.get(cursor..)?;
    let rest = rest.strip_prefix(name)?;
    let rest = rest.strip_prefix(b" ")?;
    let newline = rest.iter().position(|byte| *byte == b'\n')?;
    Some(cursor + name.len() + 1 + newline + 1)
}

fn write_id_line(out: &mut Vec<u8>, name: &[u8], id: &oid) {
    // Wide enough for every hash gix knows, so the slice never reallocates.
    let mut hex = [0_u8; gix::hash::Kind::longest().len_in_hex()];
    let hex = id.hex_to_buf(&mut hex);
    out.extend_from_slice(name);
    out.push(b' ');
    out.extend_from_slice(hex.as_bytes());
    out.push(b'\n');
}

#[cfg(test)]
mod tests {
    use gix::ObjectId;

    use super::replace_ids;

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";

    fn oid(hex: &str) -> ObjectId {
        ObjectId::from_hex(hex.as_bytes()).expect("test constant is valid hex")
    }

    #[test]
    fn keeps_every_byte_after_the_parent_lines() {
        // A committer line with a timezone offset git stores but cannot
        // normalize, an `encoding` header placed *after* `gpgsig`, and a
        // folded multi line value: all three are dropped or reordered by a
        // parse and reserialize round trip.
        let raw = format!(
            "tree {A}\nparent {A}\nauthor A <a@e> 1000000000 +051800\ncommitter C <c@e> \
             1000000000 -0000\ngpgsig -----BEGIN-----\n body\n -----END-----\nencoding \
             ISO-8859-1\n\nsubject\n"
        );
        let out =
            replace_ids(raw.as_bytes(), &oid(B), &[oid(B), oid(A)]).expect("well formed commit");
        let expected = raw.replacen(
            &format!("tree {A}\nparent {A}\n"),
            &format!("tree {B}\nparent {B}\nparent {A}\n"),
            1,
        );
        assert_eq!(String::from_utf8_lossy(&out), expected);
    }

    #[test]
    fn handles_a_root_commit() {
        let raw = format!("tree {A}\nauthor A <a@e> 1 +0000\ncommitter A <a@e> 1 +0000\n\nm\n");
        let out = replace_ids(raw.as_bytes(), &oid(B), &[]).expect("well formed commit");
        assert_eq!(String::from_utf8_lossy(&out), raw.replace(A, B));
    }

    #[test]
    fn rejects_an_object_that_does_not_start_with_a_tree_line() {
        let err = replace_ids(b"parent x\n\nm\n", &oid(A), &[]);
        assert!(
            err.is_err(),
            "expected a malformed commit error, got {err:?}"
        );
    }
}
