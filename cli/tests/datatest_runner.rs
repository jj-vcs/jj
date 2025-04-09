mod test_config_schema;

datatest_stable::harness! {
    {
        test = test_config_schema::taplo_check_config,
        root = "src/config",
        pattern = r".*\.toml",
    }
}
