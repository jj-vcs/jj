use clap::Parser;
use scm_diff_editor::{run, Opts, Result};

pub fn main() -> Result<()> {
    let opts = Opts::parse();
    run(opts)?;
    Ok(())
}
