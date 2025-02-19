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
use std::fmt;
use std::fmt::Display;
use std::path::Path;
use std::path::PathBuf;

use indoc::formatdoc;
use regex::Captures;
use regex::Regex;
use tempfile::TempDir;

use super::fake_diff_editor_path;
use super::fake_editor_path;
use super::get_stderr_string;
use super::get_stdout_string;
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
    /// If true, `jj_cmd_success` does not abort when `jj` exits with nonempty
    /// stderr and outputs it instead. This is meant only for debugging.
    ///
    /// This allows debugging the execution of an integration test by inserting
    /// `eprintln!` or `dgb!` statements in relevant parts of jj source code.
    ///
    /// You can change the value of this parameter directly, or you can set the
    /// `JJ_DEBUG_ALLOW_STDERR` environment variable.
    ///
    /// To see the output, you can run `cargo test` with the --show-output
    /// argument, like so:
    ///
    ///     RUST_BACKTRACE=1 JJ_DEBUG_ALLOW_STDERR=1 cargo test \
    ///         --test test_git_colocated -- --show-output fetch_del
    ///
    /// This would run all the tests that contain `fetch_del` in their name in a
    /// file that contains `test_git_colocated` and show the output.
    pub debug_allow_stderr: bool,
}

impl Default for TestEnvironment {
    fn default() -> Self {
        testutils::hermetic_libgit2();

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
            debug_allow_stderr: std::env::var("JJ_DEBUG_ALLOW_STDERR").is_ok(),
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
    pub fn jj_cmd(&self, current_dir: &Path, args: &[&str]) -> assert_cmd::Command {
        let mut cmd = assert_cmd::Command::cargo_bin("jj").unwrap();
        cmd.current_dir(current_dir);
        cmd.args(args);
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

    pub fn write_stdin(&self, cmd: &mut assert_cmd::Command, stdin: &str) {
        cmd.env("JJ_INTERACTIVE", "1");
        cmd.write_stdin(stdin);
    }

    pub fn jj_cmd_stdin(
        &self,
        current_dir: &Path,
        args: &[&str],
        stdin: &str,
    ) -> assert_cmd::Command {
        let mut cmd = self.jj_cmd(current_dir, args);
        self.write_stdin(&mut cmd, stdin);

        cmd
    }

    fn get_ok(&self, mut cmd: assert_cmd::Command) -> (CommandOutputString, CommandOutputString) {
        let assert = cmd.assert().success();
        let stdout = self.normalize_output(get_stdout_string(&assert));
        let stderr = self.normalize_output(get_stderr_string(&assert));
        (stdout, stderr)
    }

    /// Run a `jj` command, check that it was successful, and return its
    /// `(stdout, stderr)`.
    pub fn jj_cmd_ok(
        &self,
        current_dir: &Path,
        args: &[&str],
    ) -> (CommandOutputString, CommandOutputString) {
        self.get_ok(self.jj_cmd(current_dir, args))
    }

    pub fn jj_cmd_stdin_ok(
        &self,
        current_dir: &Path,
        args: &[&str],
        stdin: &str,
    ) -> (CommandOutputString, CommandOutputString) {
        self.get_ok(self.jj_cmd_stdin(current_dir, args, stdin))
    }

    /// Run a `jj` command, check that it was successful, and return its stdout
    #[track_caller]
    pub fn jj_cmd_success(&self, current_dir: &Path, args: &[&str]) -> CommandOutputString {
        if self.debug_allow_stderr {
            let (stdout, stderr) = self.jj_cmd_ok(current_dir, args);
            if !stderr.is_empty() {
                eprintln!(
                    "==== STDERR from running jj with {args:?} args in {current_dir:?} \
                     ====\n{stderr}==== END STDERR ===="
                );
            }
            stdout
        } else {
            let assert = self.jj_cmd(current_dir, args).assert().success().stderr("");
            self.normalize_output(get_stdout_string(&assert))
        }
    }

    /// Run a `jj` command, check that it failed with code 1, and return its
    /// stderr
    #[must_use]
    pub fn jj_cmd_failure(&self, current_dir: &Path, args: &[&str]) -> CommandOutputString {
        let assert = self.jj_cmd(current_dir, args).assert().code(1).stdout("");
        self.normalize_output(get_stderr_string(&assert))
    }

    /// Run a `jj` command and check that it failed with code 2 (for invalid
    /// usage)
    #[must_use]
    pub fn jj_cmd_cli_error(&self, current_dir: &Path, args: &[&str]) -> CommandOutputString {
        let assert = self.jj_cmd(current_dir, args).assert().code(2).stdout("");
        self.normalize_output(get_stderr_string(&assert))
    }

    /// Run a `jj` command, check that it failed with code 255, and return its
    /// stderr
    #[must_use]
    pub fn jj_cmd_internal_error(&self, current_dir: &Path, args: &[&str]) -> CommandOutputString {
        let assert = self.jj_cmd(current_dir, args).assert().code(255).stdout("");
        self.normalize_output(get_stderr_string(&assert))
    }

    /// Run a `jj` command, check that it failed with code 101, and return its
    /// stderr
    #[must_use]
    #[allow(dead_code)]
    pub fn jj_cmd_panic(&self, current_dir: &Path, args: &[&str]) -> CommandOutputString {
        let assert = self.jj_cmd(current_dir, args).assert().code(101).stdout("");
        self.normalize_output(get_stderr_string(&assert))
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
        let id_and_newline =
            self.jj_cmd_success(repo_path, &["debug", "operation", "--display=id"]);
        id_and_newline.raw().trim_end().to_owned()
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

    pub fn normalize_output(&self, raw: String) -> CommandOutputString {
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

/// Command output data to be displayed in normalized form.
// TODO: Maybe we can add wrapper that stores both stdout/stderr and print them.
#[derive(Clone)]
pub struct CommandOutputString {
    // TODO: use BString?
    raw: String,
    normalized: String,
}

impl CommandOutputString {
    /// Normalizes Windows directory separator to slash.
    pub fn normalize_backslash(self) -> Self {
        self.normalize_with(|s| s.replace('\\', "/"))
    }

    /// Normalizes [`std::process::ExitStatus`] message.
    ///
    /// On Windows, it prints "exit code" instead of "exit status".
    pub fn normalize_exit_status(self) -> Self {
        self.normalize_with(|s| s.replace("exit code:", "exit status:"))
    }

    /// Removes the last line (such as platform-specific error message) from the
    /// normalized text.
    pub fn strip_last_line(self) -> Self {
        self.normalize_with(|mut s| {
            s.truncate(strip_last_line(&s).len());
            s
        })
    }

    pub fn normalize_with(mut self, f: impl FnOnce(String) -> String) -> Self {
        self.normalized = f(self.normalized);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// Raw output data.
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Normalized text for snapshot testing.
    pub fn normalized(&self) -> &str {
        &self.normalized
    }

    /// Extracts raw output data.
    pub fn into_raw(self) -> String {
        self.raw
    }
}

impl Display for CommandOutputString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.normalized)
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
