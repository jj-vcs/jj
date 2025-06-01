mod test_config_schema;

datatest_stable::harness! {
    {
        test = test_config_schema::taplo_check_config_valid,
        root = "../src/config",
        pattern = r".*\.toml",
    },
    {
        test = test_config_schema::taplo_check_config_valid,
        root = "sample-configs/valid",
        pattern = r".*\.toml",
    },
    {
        test = test_config_schema::taplo_check_config_invalid,
        root = "sample-configs/invalid",
        pattern = r".*\.toml",
    }
}
