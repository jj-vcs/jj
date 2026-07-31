//! Materializes the test fixture into a real repository, for inspection with
//! git itself.
//!
//! The fixture is assembled from raw bytes, so `git fsck` on the result is the
//! check that its hand folded headers and its tree entry ordering are what git
//! actually expects, rather than only what this crate agrees with itself about.
//!
//! ```text
//! cargo run --example fixture_repo -- /tmp/fixture.git
//! git -C /tmp/fixture.git fsck --strict
//! git -C /tmp/fixture.git log --graph --format='%h %d %s'
//! ```

use std::path::Path;

type Failure = Box<dyn std::error::Error + Send + Sync>;

fn main() -> Result<(), Failure> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [dir] = args.as_slice() else {
        return Err("usage: fixture_repo <dir.git>".into());
    };
    let dir = Path::new(dir);
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    let repo = gix::init_bare(dir)?;
    let upstream = jj_views::fixture::write_upstream(&repo)?;
    for (label, commit) in &upstream.commits {
        println!("{commit} {label}");
        std::fs::write(
            dir.join("refs").join("heads").join(label),
            format!("{commit}\n"),
        )?;
    }
    println!("head {}", upstream.head);
    Ok(())
}
