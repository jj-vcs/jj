// Copyright 2025 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::process::Command;
use std::process::Output;
use std::process::Stdio;

use camino::Utf8Path;
use camino::Utf8PathBuf;
use testutils::ensure_running_outside_ci;
use testutils::is_external_tool_installed;

fn taplo_check_config(file: &Utf8Path) -> datatest_stable::Result<Option<Output>> {
    if !is_external_tool_installed("taplo") {
        ensure_running_outside_ci("`taplo` must be in the PATH");
        eprintln!("Skipping test because taplo is not installed on the system");
        return Ok(None);
    }

    // Taplo requires an absolute URL to the schema :/
    let root = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Ok(Some(
        Command::new("taplo")
            .args([
                "check",
                "--schema",
                &format!("file://{root}/src/config-schema.json"),
            ])
            .arg(file.as_os_str())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?
            .wait_with_output()?,
    ))
}

pub(crate) fn taplo_check_config_valid(file: &Utf8Path) -> datatest_stable::Result<()> {
    if let Some(taplo_res) = taplo_check_config(file)? {
        if !taplo_res.status.success() {
            eprintln!("Failed to validate {file}:");
            eprintln!("{}", String::from_utf8_lossy(&taplo_res.stderr));
            return Err("Validation failed".into());
        }
    }
    Ok(())
}

pub(crate) fn taplo_check_config_invalid(file: &Utf8Path) -> datatest_stable::Result<()> {
    if let Some(taplo_res) = taplo_check_config(file)? {
        if taplo_res.status.success() {
            return Err("Validation unexpectedly passed".into());
        }
    }
    Ok(())
}
