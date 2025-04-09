use std::path::Path;
use std::process::Command;
use std::process::Stdio;

use testutils::ensure_running_outside_ci;

pub(crate) fn taplo_check_config(file: &Path) -> datatest_stable::Result<()> {
    if Command::new("taplo")
        .arg("--version")
        .stdout(Stdio::null())
        .status()
        .is_err()
    {
        ensure_running_outside_ci("`taplo` must be in the PATH");
        eprintln!("Skipping test because taplo is not installed on the system");
        return Ok(());
    }

    // Taplo requires an absolute URL to the schema :/
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let taplo_res = Command::new("taplo")
        .args([
            "check",
            "--schema",
            &format!("file://{}/src/config-schema.json", root.display()),
        ])
        .arg(file.as_os_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?
        .wait_with_output()?;

    if !taplo_res.status.success() {
        eprintln!("Failed to validate {}:", file.display());
        eprintln!("{}", String::from_utf8_lossy(&taplo_res.stderr));
        Err("Validation failed".into())
    } else {
        Ok(())
    }
}
