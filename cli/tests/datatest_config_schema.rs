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

use std::path::Path;

use itertools::Itertools as _;

fn validate_config_toml(config_toml: String) -> Result<(), String> {
    let config = toml_edit::de::from_str(&config_toml).unwrap();

    // TODO: Fix unfortunate duplication with `test_config_schema.rs`.
    const SCHEMA_SRC: &str = include_str!("../src/config-schema.json");
    let schema = serde_json::from_str(SCHEMA_SRC).expect("`config-schema.json` to be valid JSON");
    let validator =
        jsonschema::validator_for(&schema).expect("`config-schema.json` to be a valid schema");
    let evaluation = validator.evaluate(&config);
    if evaluation.flag().valid {
        Ok(())
    } else {
        Err(evaluation
            .iter_errors()
            .map(|err| format!("* {}: {}", err.instance_location, err.error))
            .join("\n"))
    }
}

pub(crate) fn check_config_file_valid(
    path: &Path,
    config_toml: String,
) -> datatest_stable::Result<()> {
    if let Err(err) = validate_config_toml(config_toml) {
        panic!("Failed to validate `{}`:\n{err}", path.display());
    }
    Ok(())
}

pub(crate) fn check_config_file_invalid(
    path: &Path,
    config_toml: String,
) -> datatest_stable::Result<()> {
    if let Ok(()) = validate_config_toml(config_toml) {
        panic!("Validation for `{}` unexpectedly passed", path.display());
    }
    Ok(())
}
