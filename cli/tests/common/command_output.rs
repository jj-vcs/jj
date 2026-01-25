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

use std::fmt;
use std::fmt::Debug;
use std::fmt::Display;
use std::process::ExitStatus;

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
        Self {
            stdout: self.stdout.normalize_backslash(),
            stderr: self.stderr.normalize_backslash(),
            status: self.status,
        }
    }

    /// Normalizes [`ExitStatus`] message in stderr text.
    #[must_use]
    pub fn normalize_stderr_exit_status(self) -> Self {
        Self {
            stdout: self.stdout,
            stderr: self.stderr.normalize_exit_status(),
            status: self.status,
        }
    }

    /// Removes the last line (such as platform-specific error message) from the
    /// normalized stderr text.
    #[must_use]
    pub fn strip_stderr_last_line(self) -> Self {
        Self {
            stdout: self.stdout,
            stderr: self.stderr.strip_last_line(),
            status: self.status,
        }
    }

    /// Removes all but the first `n` lines from normalized stdout text.
    #[must_use]
    pub fn take_stdout_n_lines(self, n: usize) -> Self {
        Self {
            stdout: self.stdout.take_n_lines(n),
            stderr: self.stderr,
            status: self.status,
        }
    }

    #[must_use]
    pub fn normalize_stdout_with(self, f: impl FnOnce(String) -> String) -> Self {
        Self {
            stdout: self.stdout.normalize_with(f),
            stderr: self.stderr,
            status: self.status,
        }
    }

    #[must_use]
    pub fn normalize_stderr_with(self, f: impl FnOnce(String) -> String) -> Self {
        Self {
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
        let Self {
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
    pub(super) raw: String,
    pub(super) normalized: String,
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

    /// Removes all but the first `n` lines from the normalized text.
    #[must_use]
    pub fn take_n_lines(self, n: usize) -> Self {
        self.normalize_with(|s| s.split_inclusive('\n').take(n).collect())
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

/// Returns a string with the last line removed.
///
/// Use this to remove the root error message containing platform-specific
/// content for example.
pub fn strip_last_line(s: &str) -> &str {
    s.trim_end_matches('\n')
        .rsplit_once('\n')
        .map_or(s, |(h, _)| &s[..h.len() + 1])
}
