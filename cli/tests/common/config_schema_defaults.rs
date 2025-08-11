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

/// Produce a synthetic JSON document containing a default config according to
/// the schema.
///
/// # Limitations
/// Defaults in `"additionalProperties"` are ignored, as are those in
/// `"definitions"` nodes (which might be referenced by `"$ref"`).
pub fn default_config_from_schema() -> serde_json::Value {
    fn visit(schema: &serde_json::Value) -> Option<serde_json::Value> {
        let schema = schema.as_object().expect("schemas to be objects");
        if let Some(default) = schema.get("default") {
            Some(default.clone())
        } else if let Some(properties) = schema.get("properties") {
            let prop_defaults: serde_json::Map<_, _> = properties
                .as_object()
                .expect("`properties` to be an object")
                .iter()
                .filter_map(|(prop_name, prop_schema)| {
                    visit(prop_schema).map(|prop_default| (prop_name.clone(), prop_default))
                })
                .collect();
            (!prop_defaults.is_empty()).then_some(serde_json::Value::Object(prop_defaults))
        } else {
            None
        }
    }

    visit(
        &serde_json::from_str(include_str!("../../src/config-schema.json"))
            .expect("`config-schema.json` to be valid JSON"),
    )
    .expect("some settings to have defaults")
}
