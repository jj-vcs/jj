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
use std::path::Path;
use std::path::PathBuf;

use bstr::BString;
use indoc::formatdoc;
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
    env_vars: HashMap<String, String>,
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
        let env = Self {
            _temp_dir: tmp_dir,
            env_root,
            home_dir,
            config_path: config_dir,
            env_vars,
            config_file_number: RefCell::new(0),
            command_number: RefCell::new(0),
        };
        // Use absolute timestamps in the operation log to make tests independent of the
        // current time.
        env.add_config(
            r#"
[template-aliases]
'format_time_range(time_range)' = 'time_range.start() ++ " - " ++ time_range.end()'
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
        let mut cmd = assert_cmd::Command::cargo_bin("jj").unwrap();
        cmd.current_dir(&self.env_root);
        cmd.env_clear();
        cmd.env("COLUMNS", "100");
        cmd.env("RUST_BACKTRACE", "1");
        // We want to keep the "PATH" environment variable to allow accessing
        // executables like `git` from the PATH.
        cmd.env("PATH", std::env::var_os("PATH").unwrap_or_default());
        cmd.env("HOME", self.home_dir.to_str().unwrap());
        // Prevent git.subprocess from reading outside git config
        cmd.env("GIT_CONFIG_SYSTEM", "/dev/null");
        cmd.env("GIT_CONFIG_GLOBAL", "/dev/null");
        cmd.env("JJ_CONFIG", self.config_path.to_str().unwrap());
        cmd.env("JJ_USER", "Test User");
        cmd.env("JJ_EMAIL", "test.user@example.com");
        cmd.env("JJ_OP_HOSTNAME", "host.example.com");
        cmd.env("JJ_OP_USERNAME", "test-username");
        cmd.env("JJ_TZ_OFFSET_MINS", "660");
        for (key, value) in &self.env_vars {
            cmd.env(key, value);
        }

        let mut command_number = self.command_number.borrow_mut();
        *command_number += 1;
        cmd.env("JJ_RANDOMNESS_SEED", command_number.to_string());
        let timestamp = chrono::DateTime::parse_from_rfc3339("2001-02-03T04:05:06+07:00").unwrap();
        let timestamp = timestamp + chrono::Duration::try_seconds(*command_number).unwrap();
        cmd.env("JJ_TIMESTAMP", timestamp.to_rfc3339());
        cmd.env("JJ_OP_TIMESTAMP", timestamp.to_rfc3339());

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

    pub fn add_env_var(&mut self, key: impl Into<String>, val: impl Into<String>) {
        self.env_vars.insert(key.into(), val.into());
    }

    // TODO: Remove with the `git.subprocess` setting.
    pub fn with_git_subprocess(self, subprocess: bool) -> Self {
        assert!(subprocess);
        self
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
        self.add_env_var("EDIT_SCRIPT", edit_script.to_str().unwrap());
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
        self.add_env_var("DIFF_EDIT_SCRIPT", edit_script.to_str().unwrap());
        edit_script
    }

    #[must_use]
    fn normalize_output(&self, raw: String) -> CommandOutputString {
        let normalized = normalize_output(&raw, &self.env_root);
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

    #[track_caller]
    pub fn current_operation_id(&self) -> String {
        let output = self
            .run_jj(["debug", "operation", "--display=id"])
            .success();
        output.stdout.raw().trim_end().to_owned()
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

fn normalize_output(text: &str, env_root: &Path) -> String {
    let text = text.replace("jj.exe", "jj");
    let env_root = env_root.display().to_string();
    // Platform-native $TEST_ENV
    let regex = Regex::new(&format!(r"{}(\S+)", regex::escape(&env_root))).unwrap();
    let text = regex.replace_all(&text, |caps: &Captures| {
        format!("$TEST_ENV{}", caps[1].replace('\\', "/"))
    });
    // Slash-separated $TEST_ENV
    let text = if cfg!(windows) {
        let regex = Regex::new(&regex::escape(&env_root.replace('\\', "/"))).unwrap();
        regex.replace_all(&text, regex::NoExpand("$TEST_ENV"))
    } else {
        text
    };
    text.into_owned()
}
