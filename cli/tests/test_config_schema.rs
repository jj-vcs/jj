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

use std::io::Write as _;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;

use testutils::ensure_running_outside_ci;
use testutils::is_external_tool_installed;

use crate::common::default_toml_from_schema;

#[test]
fn test_config_schema_default_values_are_consistent_with_schema() {
    if !is_external_tool_installed("taplo") {
        ensure_running_outside_ci("`taplo` must be in the PATH");
        eprintln!("Skipping test because taplo is not installed on the system");
        return;
    }

    let Some(schema_defaults) = default_toml_from_schema() else {
        ensure_running_outside_ci("`jq` must be in the PATH");
        eprintln!("Skipping test because jq is not installed on the system");
        return;
    };

    // Taplo requires an absolute URL to the schema :/
    let root = camino::Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut taplo_child = Command::new("taplo")
        .args([
            "check",
            "--schema",
            &format!("file://{root}/src/config-schema.json"),
            "-", // read from stdin
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    {
        let mut stdin = taplo_child.stdin.take().unwrap();
        write!(stdin, "{schema_defaults}").unwrap();
        // pipe is closed here by dropping it
    }

    let Output { status, stderr, .. } = taplo_child.wait_with_output().unwrap();
    if !status.success() {
        eprintln!(
            "taplo exited with status {status}:\n{}",
            String::from_utf8_lossy(&stderr)
        );
        eprintln!("while validating synthetic defaults TOML:\n{schema_defaults}");
        panic!("Schema defaults are not valid according to schema");
    }
}
