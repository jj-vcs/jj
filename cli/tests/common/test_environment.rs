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
use std::fmt;
use std::fmt::Debug;
use std::fmt::Display;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitStatus;

use indoc::formatdoc;
use regex::Captures;
use regex::Regex;
use tempfile::TempDir;

use super::fake_diff_editor_path;
use super::fake_editor_path;
use super::strip_last_line;
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
    /// Runs `jj args..` in the `current_dir`, returns the output.
    #[must_use = "either snapshot the output or assert the exit status with .success()"]
    pub fn run_jj_in<I>(&self, current_dir: impl AsRef<Path>, args: I) -> CommandOutput
    where
        I: IntoIterator,
        I::Item: AsRef<OsStr>,
    {
        let current_dir = self.env_root.join(current_dir);
        self.run_jj_with(|cmd| cmd.current_dir(current_dir).args(args))
    }

    /// Runs `jj` command with additional configuration, returns the output.
    #[must_use = "either snapshot the output or assert the exit status with .success()"]
    pub fn run_jj_with(
        &self,
        configure: impl FnOnce(&mut assert_cmd::Command) -> &mut assert_cmd::Command,
    ) -> CommandOutput {
        let mut cmd = self.new_jj_cmd();
        let output = configure(&mut cmd).output().unwrap();
        CommandOutput {
            stdout: self.normalize_output(String::from_utf8(output.stdout).unwrap()),
            stderr: self.normalize_output(String::from_utf8(output.stderr).unwrap()),
            status: output.status,
        }
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
        for (key, value) in &self.env_vars {
            cmd.env(key, value);
        }
        cmd.env("RUST_BACKTRACE", "1");
        // We want to keep the "PATH" environment variable to allow accessing
        // executables like `git` from the PATH.
        cmd.env("PATH", std::env::var_os("PATH").unwrap_or_default());
        cmd.env("HOME", self.home_dir.to_str().unwrap());
        cmd.env("JJ_CONFIG", self.config_path.to_str().unwrap());
        cmd.env("JJ_USER", "Test User");
        cmd.env("JJ_EMAIL", "test.user@example.com");
        cmd.env("JJ_OP_HOSTNAME", "host.example.com");
        cmd.env("JJ_OP_USERNAME", "test-username");
        cmd.env("JJ_TZ_OFFSET_MINS", "660");

        let mut command_number = self.command_number.borrow_mut();
        *command_number += 1;
        cmd.env("JJ_RANDOMNESS_SEED", command_number.to_string());
        let timestamp = chrono::DateTime::parse_from_rfc3339("2001-02-03T04:05:06+07:00").unwrap();
        let timestamp = timestamp + chrono::Duration::try_seconds(*command_number).unwrap();
        cmd.env("JJ_TIMESTAMP", timestamp.to_rfc3339());
        cmd.env("JJ_OP_TIMESTAMP", timestamp.to_rfc3339());

        // libgit2 always initializes OpenSSL, and it takes a few tens of milliseconds
        // to load the system CA certificates in X509_load_cert_crl_file_ex(). As we
        // don't use HTTPS in our tests, we can disable the cert loading to speed up the
        // CLI tests. If we migrated to gitoxide, maybe we can remove this hack.
        if cfg!(unix) {
            cmd.env("SSL_CERT_FILE", "/dev/null");
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

    pub fn current_operation_id(&self, repo_path: &Path) -> String {
        let output = self
            .run_jj_in(repo_path, ["debug", "operation", "--display=id"])
            .success();
        output.stdout.raw().trim_end().to_owned()
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

/// Command output and exit status to be displayed in normalized form.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandOutput {
    pub stdout: CommandOutputString,
    pub stderr: CommandOutputString,
    pub status: ExitStatus,
}

impl CommandOutput {
    /// Normalizes Windows directory separator to slash.
    #[must_use]
    pub fn normalize_backslash(self) -> Self {
        CommandOutput {
            stdout: self.stdout.normalize_backslash(),
            stderr: self.stderr.normalize_backslash(),
            status: self.status,
        }
    }

    /// Normalizes [`ExitStatus`] message in stderr text.
    #[must_use]
    pub fn normalize_stderr_exit_status(self) -> Self {
        CommandOutput {
            stdout: self.stdout,
            stderr: self.stderr.normalize_exit_status(),
            status: self.status,
        }
    }

    /// Removes the last line (such as platform-specific error message) from the
    /// normalized stderr text.
    #[must_use]
    pub fn strip_stderr_last_line(self) -> Self {
        CommandOutput {
            stdout: self.stdout,
            stderr: self.stderr.strip_last_line(),
            status: self.status,
        }
    }

    #[must_use]
    pub fn normalize_stdout_with(self, f: impl FnOnce(String) -> String) -> Self {
        CommandOutput {
            stdout: self.stdout.normalize_with(f),
            stderr: self.stderr,
            status: self.status,
        }
    }

    #[must_use]
    pub fn normalize_stderr_with(self, f: impl FnOnce(String) -> String) -> Self {
        CommandOutput {
            stdout: self.stdout,
            stderr: self.stderr.normalize_with(f),
            status: self.status,
        }
    }

    /// Ensures that the command exits with success status.
    #[track_caller]
    pub fn success(self) -> Self {
        assert!(self.status.success(), "{self}");
        self
    }
}

impl Display for CommandOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let CommandOutput {
            stdout,
            stderr,
            status,
        } = self;
        write!(f, "{stdout}")?;
        if !stderr.is_empty() {
            writeln!(f, "------- stderr -------")?;
            write!(f, "{stderr}")?;
        }
        if !status.success() {
            // If there is an exit code, `{status}` would get rendered as "exit
            // code: N" on Windows, so we render it ourselves for compatibility.
            if let Some(code) = status.code() {
                writeln!(f, "[exit status: {code}]")?;
            } else {
                writeln!(f, "[{status}]")?;
            }
        }
        Ok(())
    }
}

/// Command output data to be displayed in normalized form.
#[derive(Clone)]
pub struct CommandOutputString {
    // TODO: use BString?
    raw: String,
    normalized: String,
}

impl CommandOutputString {
    /// Normalizes Windows directory separator to slash.
    #[must_use]
    pub fn normalize_backslash(self) -> Self {
        self.normalize_with(|s| s.replace('\\', "/"))
    }

    /// Normalizes [`ExitStatus`] message.
    ///
    /// On Windows, it prints "exit code" instead of "exit status".
    #[must_use]
    pub fn normalize_exit_status(self) -> Self {
        self.normalize_with(|s| s.replace("exit code:", "exit status:"))
    }

    /// Removes the last line (such as platform-specific error message) from the
    /// normalized text.
    #[must_use]
    pub fn strip_last_line(self) -> Self {
        self.normalize_with(|mut s| {
            s.truncate(strip_last_line(&s).len());
            s
        })
    }

    #[must_use]
    pub fn normalize_with(mut self, f: impl FnOnce(String) -> String) -> Self {
        self.normalized = f(self.normalized);
        self
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// Raw output data.
    #[must_use]
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Normalized text for snapshot testing.
    #[must_use]
    pub fn normalized(&self) -> &str {
        &self.normalized
    }

    /// Extracts raw output data.
    #[must_use]
    pub fn into_raw(self) -> String {
        self.raw
    }
}

impl Debug for CommandOutputString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Print only raw data. Normalized string should be nearly identical.
        Debug::fmt(&self.raw, f)
    }
}

impl Display for CommandOutputString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return Ok(());
        }
        // Append "[EOF]" marker to test line ending
        // https://github.com/mitsuhiko/insta/issues/384
        writeln!(f, "{}[EOF]", self.normalized)
    }
}

impl Eq for CommandOutputString {}

impl PartialEq for CommandOutputString {
    fn eq(&self, other: &Self) -> bool {
        // Compare only raw data. Normalized string is for displaying purpose.
        self.raw == other.raw
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
