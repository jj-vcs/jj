mod datatest_config_schema;

datatest_stable::harness! {
    {
        test = datatest_config_schema::taplo_check_config_valid,
        root = "src/config",
        pattern = r".*\.toml",
    },
    {
        test = datatest_config_schema::taplo_check_config_valid,
        root = "tests/sample-configs/valid",
        pattern = r".*\.toml",
    },
    {
        test = datatest_config_schema::taplo_check_config_invalid,
        root = "tests/sample-configs/invalid",
        pattern = r".*\.toml",
    }
}
