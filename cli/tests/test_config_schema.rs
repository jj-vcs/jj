use itertools::Itertools as _;
use jsonschema::JSONSchema;

fn taplo_check_config(toml: &str) {
    const SCHEMA_SRC: &str = include_str!("../src/config-schema.json");
    let schema_json =
        serde_json::from_str(SCHEMA_SRC).expect("`config-schema.json` to be valid JSON");
    let schema =
        JSONSchema::compile(&schema_json).expect("`config-schema.json` to be a valid schema");
    let config = toml_edit::de::from_str(toml).expect("default configuration to be valid TOML");
    let result = schema.validate(&config);
    if let Err(errs) = result {
        panic!(
            "Failed to validate default configuration:\n{}",
            errs.into_iter()
                .map(|err| format!("* {}: {}", err.instance_path, err))
                .join("\n")
        );
    }
}

#[test]
fn test_taplo_check_colors_config() {
    taplo_check_config(include_str!("../src/config/colors.toml"));
}

#[test]
fn test_taplo_check_merge_tools_config() {
    taplo_check_config(include_str!("../src/config/merge_tools.toml"));
}

#[test]
fn test_taplo_check_misc_config() {
    taplo_check_config(include_str!("../src/config/misc.toml"));
}

#[test]
fn test_taplo_check_revsets_config() {
    taplo_check_config(include_str!("../src/config/revsets.toml"));
}

#[test]
fn test_taplo_check_templates_config() {
    taplo_check_config(include_str!("../src/config/templates.toml"));
}

#[test]
fn test_taplo_check_unix_config() {
    taplo_check_config(include_str!("../src/config/unix.toml"));
}

#[test]
fn test_taplo_check_windows_config() {
    taplo_check_config(include_str!("../src/config/windows.toml"));
}
