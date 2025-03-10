use std::process::Command;
use std::process::Stdio;

use testutils::TestEnvironment;

fn taplo_check_config(file: &str) {
    if Command::new("taplo")
        .arg("--version")
        .stdout(Stdio::null())
        .status()
        .is_err()
    {
        TestEnvironment::ensure_running_outside_ci("`taplo` must be in the PATH");
        eprintln!("Skipping test because taplo is not installed on the system");
        return;
    }

    // Taplo requires an absolute URL to the schema :/
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let taplo_res = Command::new("taplo")
        .args([
            "check",
            "--schema",
            &format!("file://{}/src/config-schema.json", root.display()),
            file,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
        .wait_with_output()
        .unwrap();

    if !taplo_res.status.success() {
        eprintln!("Failed to validate {file}:");
        eprintln!("{}", String::from_utf8_lossy(&taplo_res.stderr));
        panic!("Validation failed");
    }
}

#[test]
fn test_taplo_check_colors_config() {
    taplo_check_config("src/config/colors.toml");
}

#[test]
fn test_taplo_check_merge_tools_config() {
    taplo_check_config("src/config/merge_tools.toml");
}

#[test]
fn test_taplo_check_misc_config() {
    taplo_check_config("src/config/misc.toml");
}

#[test]
fn test_taplo_check_revsets_config() {
    taplo_check_config("src/config/revsets.toml");
}

#[test]
fn test_taplo_check_templates_config() {
    taplo_check_config("src/config/templates.toml");
}

#[test]
fn test_taplo_check_unix_config() {
    taplo_check_config("src/config/unix.toml");
}

#[test]
fn test_taplo_check_windows_config() {
    taplo_check_config("src/config/windows.toml");
}
