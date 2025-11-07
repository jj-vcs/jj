// Copyright 2020 The Jujutsu Authors
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

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;

use bstr::BString;
use indoc::formatdoc;
use itertools::Itertools as _;
use regex::Captures;
use regex::Regex;
use tempfile::TempDir;

use super::command_output::CommandOutput;
use super::command_output::CommandOutputString;
use super::fake_diff_editor_path;
use super::fake_editor_path;
use super::to_toml_value;

pub struct TestEnvironment {
    _temp_dir: TempDir,
    env_root: PathBuf,
    home_dir: PathBuf,
    config_path: PathBuf,
    env_vars: HashMap<OsString, OsString>,
    paths_to_normalize: Vec<(PathBuf, String)>,
    config_file_number: RefCell<i64>,
    command_number: RefCell<i64>,
}

impl Default for TestEnvironment {
    fn default() -> Self {
        testutils::hermetic_git();

        let tmp_dir = testutils::new_temp_dir();
        let env_root = dunce::canonicalize(tmp_dir.path()).unwrap();
        let home_dir = env_root.join("home");
        std::fs::create_dir(&home_dir).unwrap();
        let config_dir = env_root.join("config");
        std::fs::create_dir(&config_dir).unwrap();
        let env_vars = HashMap::new();
        let paths_to_normalize = [(env_root.clone(), "$TEST_ENV".to_string())]
            .into_iter()
            .collect();
        let env = Self {
            _temp_dir: tmp_dir,
            env_root,
            home_dir,
            config_path: config_dir,
            env_vars,
            paths_to_normalize,
            config_file_number: RefCell::new(0),
            command_number: RefCell::new(0),
        };
        // Use absolute timestamps in the operation log to make tests independent of the
        // current time. Use non-colocated workspaces by default for simplicity.
        env.add_config(
            r#"
[template-aliases]
'format_time_range(time_range)' = 'time_range.start() ++ " - " ++ time_range.end()'

[git]
colocate = false
        "#,
        );

        env
    }
}

impl TestEnvironment {
    /// Returns test helper for the specified directory.
    ///
    /// The `root` path usually points to the workspace root, but it may be
    /// arbitrary path including non-existent directory.
    #[must_use]
    pub fn work_dir(&self, root: impl AsRef<Path>) -> TestWorkDir<'_> {
        let root = self.env_root.join(root);
        TestWorkDir { env: self, root }
    }

    /// Runs `jj args..` in the `current_dir`, returns the output.
    #[must_use = "either snapshot the output or assert the exit status with .success()"]
    pub fn run_jj_in<I>(&self, current_dir: impl AsRef<Path>, args: I) -> CommandOutput
    where
        I: IntoIterator,
        I::Item: AsRef<OsStr>,
    {
        self.work_dir(current_dir).run_jj(args)
    }

    /// Runs `jj` command with additional configuration, returns the output.
    #[must_use = "either snapshot the output or assert the exit status with .success()"]
    pub fn run_jj_with(
        &self,
        configure: impl FnOnce(&mut assert_cmd::Command) -> &mut assert_cmd::Command,
    ) -> CommandOutput {
        self.work_dir("").run_jj_with(configure)
    }

    /// Returns command builder to run `jj` in the test environment.
    ///
    /// Use `run_jj_with()` to run command within customized environment.
    #[must_use]
    pub fn new_jj_cmd(&self) -> assert_cmd::Command {
        let jj_path = assert_cmd::cargo::cargo_bin!("jj");
        let mut cmd = assert_cmd::Command::new(jj_path);
        cmd.current_dir(&self.env_root);
        cmd.env_clear();
        cmd.env("COLUMNS", "100");
        cmd.env("RUST_BACKTRACE", "1");
        // We want to keep the "PATH" environment variable to allow accessing
        // executables like `git` from the PATH.
        cmd.env("PATH", std::env::var_os("PATH").unwrap_or_default());
        cmd.env("HOME", &self.home_dir);
        // Prevent git.subprocess from reading outside git config
        cmd.env("GIT_CONFIG_SYSTEM", "/dev/null");
        cmd.env("GIT_CONFIG_GLOBAL", "/dev/null");
        cmd.env("JJ_CONFIG", &self.config_path);
        cmd.env("JJ_USER", "Test User");
        cmd.env("JJ_EMAIL", "test.user@example.com");
        cmd.env("JJ_OP_HOSTNAME", "host.example.com");
        cmd.env("JJ_OP_USERNAME", "test-username");
        cmd.env("JJ_TZ_OFFSET_MINS", "660");
        // Coverage files should not pollute the working directory
        if let Some(cov_var) = std::env::var_os("LLVM_PROFILE_FILE") {
            cmd.env("LLVM_PROFILE_FILE", cov_var);
        }

        let mut command_number = self.command_number.borrow_mut();
        *command_number += 1;
        cmd.env("JJ_RANDOMNESS_SEED", command_number.to_string());
        let timestamp = chrono::DateTime::parse_from_rfc3339("2001-02-03T04:05:06+07:00").unwrap();
        let timestamp = timestamp + chrono::Duration::try_seconds(*command_number).unwrap();
        cmd.env("JJ_TIMESTAMP", timestamp.to_rfc3339());
        cmd.env("JJ_OP_TIMESTAMP", timestamp.to_rfc3339());
        for (key, value) in &self.env_vars {
            cmd.env(key, value);
        }

        if cfg!(windows) {
            // Windows uses `TEMP` to create temporary directories, which we need for some
            // tests.
            if let Ok(tmp_var) = std::env::var("TEMP") {
                cmd.env("TEMP", tmp_var);
            }
        }

        cmd
    }

    pub fn env_root(&self) -> &Path {
        &self.env_root
    }

    pub fn home_dir(&self) -> &Path {
        &self.home_dir
    }

    pub fn config_path(&self) -> &PathBuf {
        &self.config_path
    }

    pub fn first_config_file_path(&self) -> PathBuf {
        let config_file_number = 1;
        self.config_path
            .join(format!("config{config_file_number:04}.toml"))
    }

    pub fn last_config_file_path(&self) -> PathBuf {
        let config_file_number = self.config_file_number.borrow();
        self.config_path
            .join(format!("config{config_file_number:04}.toml"))
    }

    pub fn set_config_path(&mut self, config_path: impl Into<PathBuf>) {
        self.config_path = config_path.into();
    }

    pub fn add_config(&self, content: impl AsRef<[u8]>) {
        if self.config_path.is_file() {
            panic!("add_config not supported when config_path is a file");
        }
        // Concatenating two valid TOML files does not (generally) result in a valid
        // TOML file, so we create a new file every time instead.
        let mut config_file_number = self.config_file_number.borrow_mut();
        *config_file_number += 1;
        let config_file_number = *config_file_number;
        std::fs::write(
            self.config_path
                .join(format!("config{config_file_number:04}.toml")),
            content,
        )
        .unwrap();
    }

    pub fn add_env_var(&mut self, key: impl Into<OsString>, val: impl Into<OsString>) {
        self.env_vars.insert(key.into(), val.into());
    }

    #[allow(dead_code)]
    pub fn add_paths_to_normalize(
        &mut self,
        path: impl Into<PathBuf>,
        replacement: impl Into<String>,
    ) {
        self.paths_to_normalize
            .push((path.into(), replacement.into()));
    }

    /// Sets up the fake bisection test command to read a script from the
    /// returned path
    pub fn set_up_fake_bisector(&mut self) -> PathBuf {
        let bisection_script = self.env_root().join("bisection_script");
        std::fs::write(&bisection_script, "").unwrap();
        self.add_env_var("BISECTION_SCRIPT", &bisection_script);
        bisection_script
    }

    /// Sets up the fake editor to read an edit script from the returned path
    /// Also sets up the fake editor as a merge tool named "fake-editor"
    pub fn set_up_fake_editor(&mut self) -> PathBuf {
        let editor_path = to_toml_value(fake_editor_path());
        self.add_config(formatdoc! {r#"
            [ui]
            editor = {editor_path}
            merge-editor = "fake-editor"

            [merge-tools]
            fake-editor.program = {editor_path}
            fake-editor.merge-args = ["$output"]
        "#});
        let edit_script = self.env_root().join("edit_script");
        std::fs::write(&edit_script, "").unwrap();
        self.add_env_var("EDIT_SCRIPT", &edit_script);
        edit_script
    }

    /// Sets up the fake diff-editor to read an edit script from the returned
    /// path
    pub fn set_up_fake_diff_editor(&mut self) -> PathBuf {
        let diff_editor_path = to_toml_value(fake_diff_editor_path());
        self.add_config(formatdoc! {r#"
            ui.diff-editor = "fake-diff-editor"
            merge-tools.fake-diff-editor.program = {diff_editor_path}
        "#});
        let edit_script = self.env_root().join("diff_edit_script");
        std::fs::write(&edit_script, "").unwrap();
        self.add_env_var("DIFF_EDIT_SCRIPT", &edit_script);
        edit_script
    }

    #[must_use]
    fn normalize_output(&self, raw: String) -> CommandOutputString {
        let mut normalized = raw.replace("jj.exe", "jj");
        for (path, replacement) in &self.paths_to_normalize {
            let path = path.display().to_string();
            // Platform-native $TEST_ENV
            let regex = Regex::new(&format!(r"{}(\S+)", regex::escape(&path))).unwrap();
            normalized = regex
                .replace_all(&normalized, |caps: &Captures| {
                    format!("{}{}", replacement, caps[1].replace('\\', "/"))
                })
                .to_string();
            // Slash-separated $TEST_ENV
            if cfg!(windows) {
                let regex = Regex::new(&regex::escape(&path.replace('\\', "/"))).unwrap();
                normalized = regex
                    .replace_all(&normalized, regex::NoExpand(replacement))
                    .to_string();
            };
        }
        CommandOutputString { raw, normalized }
    }

    /// Used before mutating operations to create more predictable commit ids
    /// and change ids in tests
    ///
    /// `test_env.advance_test_rng_seed_to_multiple_of(200_000)` can be inserted
    /// wherever convenient throughout your test. If desired, you can have
    /// "subheadings" with steps of (e.g.) 10_000, 500, 25.
    pub fn advance_test_rng_seed_to_multiple_of(&self, step: i64) {
        assert!(step > 0, "step must be >0, got {step}");
        let mut command_number = self.command_number.borrow_mut();
        *command_number = step * (*command_number / step) + step;
    }
}

/// Helper to execute `jj` or file operation in sub directory.
pub struct TestWorkDir<'a> {
    env: &'a TestEnvironment,
    root: PathBuf,
}

impl TestWorkDir<'_> {
    /// Path to the working directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Runs `jj args..` in the working directory, returns the output.
    #[must_use = "either snapshot the output or assert the exit status with .success()"]
    pub fn run_jj<I>(&self, args: I) -> CommandOutput
    where
        I: IntoIterator,
        I::Item: AsRef<OsStr>,
    {
        self.run_jj_with(|cmd| cmd.args(args))
    }

    /// Runs `jj` command with additional configuration, returns the output.
    #[must_use = "either snapshot the output or assert the exit status with .success()"]
    pub fn run_jj_with(
        &self,
        configure: impl FnOnce(&mut assert_cmd::Command) -> &mut assert_cmd::Command,
    ) -> CommandOutput {
        let env = &self.env;
        let mut cmd = env.new_jj_cmd();
        let output = configure(cmd.current_dir(&self.root)).output().unwrap();
        CommandOutput {
            stdout: env.normalize_output(String::from_utf8(output.stdout).unwrap()),
            stderr: env.normalize_output(String::from_utf8(output.stderr).unwrap()),
            status: output.status,
        }
    }

    /// Reads the current operation id without resolving divergence nor
    /// snapshotting. `TestWorkDir` must be at the workspace root.
    #[track_caller]
    pub fn current_operation_id(&self) -> String {
        let heads_dir = self
            .root()
            .join(PathBuf::from_iter([".jj", "repo", "op_heads", "heads"]));
        let head_entry = heads_dir
            .read_dir()
            .expect("TestWorkDir must point to the workspace root")
            .exactly_one()
            .expect("divergence not supported")
            .unwrap();
        head_entry.file_name().into_string().unwrap()
    }

    /// Returns test helper for the specified sub directory.
    #[must_use]
    pub fn dir(&self, path: impl AsRef<Path>) -> Self {
        let env = self.env;
        let root = self.root.join(path);
        TestWorkDir { env, root }
    }

    #[track_caller]
    pub fn create_dir(&self, path: impl AsRef<Path>) -> Self {
        let dir = self.dir(path);
        std::fs::create_dir(&dir.root).unwrap();
        dir
    }

    #[track_caller]
    pub fn create_dir_all(&self, path: impl AsRef<Path>) -> Self {
        let dir = self.dir(path);
        std::fs::create_dir_all(&dir.root).unwrap();
        dir
    }

    #[track_caller]
    pub fn remove_dir_all(&self, path: impl AsRef<Path>) {
        std::fs::remove_dir_all(self.root.join(path)).unwrap();
    }

    #[track_caller]
    pub fn remove_file(&self, path: impl AsRef<Path>) {
        std::fs::remove_file(self.root.join(path)).unwrap();
    }

    #[track_caller]
    pub fn read_file(&self, path: impl AsRef<Path>) -> BString {
        std::fs::read(self.root.join(path)).unwrap().into()
    }

    #[track_caller]
    pub fn write_file(&self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            self.create_dir_all(dir);
        }
        std::fs::write(self.root.join(path), contents).unwrap();
    }
}
