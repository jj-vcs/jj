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

use itertools::Itertools as _;

use crate::common::default_config_from_schema;

#[test]
fn test_config_schema_default_values_are_consistent_with_schema() {
    let schema = serde_json::from_str(include_str!("../src/config-schema.json"))
        .expect("`config-schema.json` to be valid JSON");
    let validator =
        jsonschema::validator_for(&schema).expect("`config-schema.json` to be a valid schema");
    let schema_defaults = default_config_from_schema();

    let evaluation = validator.evaluate(&schema_defaults);
    if !evaluation.flag().valid {
        panic!(
            "Failed to validate the schema defaults:\n{}",
            evaluation
                .iter_errors()
                .map(|err| format!("* {}: {}", err.instance_location, err.error))
                .join("\n")
        );
    }
}
