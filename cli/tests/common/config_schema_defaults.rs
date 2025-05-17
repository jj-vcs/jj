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
use std::process::Stdio;

use bstr::ByteSlice as _;
use testutils::is_external_tool_installed;

/// Produce a synthetic TOML document containing a default config according to
/// the schema.
///
/// Uses the `jq` program to extract "default" nodes from the schema's JSON
/// document and construct a corresponding TOML file. This TOML document
/// consists of a single table with fully-qualified dotted keys.
///
/// # Limitations
/// Defaults in `"additionalProperties"` are ignored, as are those in
/// `"definitions"` nodes (which might be referenced by `"$ref"`).
///
/// Defaults which are JSON object-valued are supported and are specified as
/// multiple lines, descending into the object fields. Arrays of scalars or
/// other arrays are also supported (as they have the same representation in
/// JSON and TOML), but arrays of objects are not.
///
/// When `jq` is not available on the system, returns `None`. This allows the
/// caller to decide how to handle this.
///
/// # Panics
/// Panics for all other error conditions related to
/// - process spawning,
/// - errors from running the jq program,
/// - non-UTF-8 encoding,
/// - parsing of the TOML output from `jq`.
pub fn default_toml_from_schema() -> Option<toml_edit::DocumentMut> {
    const JQ_PROGRAM: &str = r#"
        paths as $p
        | select($p | any(. == "default") and all(type == "string" and . != "additionalProperties" and . != "definitions"))
        | getpath($p)
        | select(type != "object")
        | tojson as $v
        | $p
        | map(select(. != "properties" and . != "default")
        | if test("^[A-Za-z0-9_-]+$") then . else "'\(.)'" end)
        | join(".") as $k
        | "\($k)=\($v)"
    "#;

    if !is_external_tool_installed("jq") {
        return None;
    }

    let output = Command::new("jq")
        .args(["-r", JQ_PROGRAM, "src/config-schema.json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
        .wait_with_output()
        .unwrap();

    if output.status.success() {
        Some(
            toml_edit::ImDocument::parse(String::from_utf8(output.stdout).unwrap())
                .unwrap()
                .into_mut(),
        )
    } else {
        panic!(
            "failed to extract default TOML from schema using `jq`: exit code {}:\n{}",
            output.status,
            output.stderr.to_str_lossy(),
        );
    }
}
