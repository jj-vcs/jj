use crate::common::CommandOutput;
use crate::common::TestEnvironment;
use crate::common::TestWorkDir;

/// Standard rerere test setup
pub fn setup_rerere_test() -> TestEnvironment {
    let test_env = TestEnvironment::default();
    test_env.add_config("rerere.enabled = true");
    test_env.run_jj_in(".", ["git", "init", "repo"]).success();
    test_env
}

/// Create a commit with file and return its ID
pub fn create_commit(work_dir: &TestWorkDir, name: &str, file: &str, content: &str) -> String {
    work_dir.write_file(file, content);
    work_dir.run_jj(&["commit", "-m", name]).success();
    get_commit_id_by_description(work_dir, name)
}

/// Get commit ID by description
pub fn get_commit_id_by_description(work_dir: &TestWorkDir, description: &str) -> String {
    work_dir
        .run_jj(&[
            "log",
            "--no-graph",
            "-r",
            &format!(r#"description("{description}")"#),
            "-T",
            "commit_id",
        ])
        .success()
        .stdout
        .raw()
        .trim()
        .to_string()
}

/// Trait for rerere-specific assertions
pub trait RerereAssertions {
    fn assert_has_conflict(&self);
    fn assert_rerere_applied(&self, expected_count: usize);
}

impl RerereAssertions for CommandOutput {
    fn assert_has_conflict(&self) {
        assert!(
            self.stderr.raw().contains("unresolved conflicts"),
            "Expected conflict but none found. Output:\n{self}"
        );
    }

    fn assert_rerere_applied(&self, expected_count: usize) {
        let stderr = self.stderr.raw();
        let message = format!("Applied {expected_count} cached conflict resolution");
        assert!(
            stderr.contains(&message),
            "Expected rerere to apply {expected_count} resolutions. Output:\n{self}"
        );
    }
}
