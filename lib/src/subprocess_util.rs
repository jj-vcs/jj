// Copyright 2026 The Jujutsu Authors
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

//! Common helpers for configuring and running subprocesses.

#[cfg(unix)]
use std::env;
#[cfg(unix)]
use std::ffi::OsStr;
#[cfg(unix)]
use std::ffi::OsString;
use std::io;
#[cfg(unix)]
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::ExitStatus;

#[cfg(unix)]
use rustix::io::Errno;

/// Keeps a console window from popping up when `jj` is used from a GUI
/// application on Windows.
///
/// The flag is only set if the current process has no console. When `jj` runs
/// in a terminal, its console is passed down to the child instead. That
/// matters for programs that prompt the user, such as `ssh` asking for a key
/// passphrase or for confirmation of an unknown host key: those prompts are
/// written to the console, not to the inherited standard streams.
///
/// Setting `CREATE_NO_WINDOW` unconditionally gives the child an *invisible*
/// console, which the child then happily prompts on, leaving `jj` waiting
/// forever for input that the user can neither see nor provide.
#[cfg(windows)]
pub fn suppress_console_window(command: &mut Command) {
    use std::os::windows::process::CommandExt as _;

    // Note that `DETACHED_PROCESS` is not used here, even though that's how Git
    // hides the window (see `compat/mingw.c`). It leaves the child without a
    // console at all, and Win32-OpenSSH then busy-loops in `readpassphrase()`
    // when it has to prompt, flooding stderr instead of failing.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    if !has_console() {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

/// Does nothing on platforms other than Windows.
#[cfg(not(windows))]
pub fn suppress_console_window(_command: &mut Command) {}

/// Returns whether the current process has a console window.
///
/// [`std::io::IsTerminal`] can't be used for this: it is false whenever the
/// standard streams are redirected, even though a console is still attached
/// and usable for prompting.
#[cfg(windows)]
#[allow(unsafe_code)]
fn has_console() -> bool {
    use windows_sys::Win32::System::Console::GetConsoleWindow;

    // SAFETY: `GetConsoleWindow()` takes no arguments, has no preconditions,
    // and only returns a window handle (or null) without touching any memory
    // we own.
    let hwnd = unsafe { GetConsoleWindow() };
    !hwnd.is_null()
}

/// Runs `command` and waits for it to exit.
///
/// This is [`Command::status`], except that a missing executable is
/// [`io::ErrorKind::NotFound`] instead of exit status 127. qemu-user, OpenBSD,
/// and HPPA surface a successful spawn and an exit code of 127 instead.
pub fn command_status(command: &mut Command) -> io::Result<ExitStatus> {
    if let Some(err) = missing_executable_error(command) {
        return Err(err);
    }
    command.status()
}

/// Spawns `command`.
///
/// This is [`Command::spawn`], except that a missing executable is
/// [`io::ErrorKind::NotFound`] instead of exit status 127. qemu-user, OpenBSD,
/// and HPPA surface a successful spawn and an exit code of 127 instead.
pub fn command_spawn(command: &mut Command) -> io::Result<Child> {
    if let Some(err) = missing_executable_error(command) {
        return Err(err);
    }
    command.spawn()
}

/// Returns `ENOENT` when `command`'s executable does not resolve.
///
/// Rust's [`Command`] uses `posix_spawn` on Unix. When `execve` fails after
/// `posix_spawn` has already returned, POSIX requires the child to exit with
/// code 127 rather than reporting `errno` to the parent. glibc usually copies
/// that failure back through a `CLONE_VM|CLONE_VFORK` shared-stack channel, but
/// not every implementation preserves it. qemu-user, OpenBSD, and HPPA surface
/// a successful spawn and an exit code of 127 instead.
///
/// Callers that distinguish "failed to start" from "exited 127" then take the
/// wrong branch. Returning `ENOENT` before the spawn keeps the glibc error on
/// every platform. A program that does exist and exits 127 is left alone:
/// [`Command::status`] still reports that status.
fn missing_executable_error(command: &Command) -> Option<io::Error> {
    #[cfg(unix)]
    if !executable_resolves(command) {
        return Some(Errno::NOENT.into());
    }
    #[cfg(not(unix))]
    let _ = command;
    None
}

/// Whether `execvp` would find `command`'s program.
///
/// A path containing a slash is used as-is, relative to the child's working
/// directory. Anything else is searched in the child's `PATH`, matching
/// `posix_spawnp` / `execvp`. An empty `PATH` entry means the working
/// directory. When `PATH` is unset, glibc uses `confstr(_CS_PATH)`, which is
/// `/bin:/usr/bin`.
#[cfg(unix)]
fn executable_resolves(command: &Command) -> bool {
    use std::os::unix::ffi::OsStrExt as _;

    let program = command.get_program();
    if program.is_empty() {
        return false;
    }
    if program.as_bytes().contains(&b'/') {
        return path_exists(&resolve_dir(command, PathBuf::from(program)));
    }
    let path = command_search_path(command);
    env::split_paths(&path).any(|dir| path_exists(&candidate_path(command, &dir, program)))
}

#[cfg(unix)]
fn command_search_path(command: &Command) -> OsString {
    for (key, value) in command.get_envs() {
        if key == "PATH" {
            return match value {
                Some(path) => path.to_os_string(),
                // Removed. `execvp` then uses the `confstr(_CS_PATH)` default.
                None => OsString::from("/bin:/usr/bin"),
            };
        }
    }
    env::var_os("PATH").unwrap_or_else(|| OsString::from("/bin:/usr/bin"))
}

#[cfg(unix)]
fn resolve_dir(command: &Command, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else if let Some(cwd) = command.get_current_dir() {
        cwd.join(path)
    } else {
        path
    }
}

#[cfg(unix)]
fn candidate_path(command: &Command, dir: &Path, program: &OsStr) -> PathBuf {
    let joined = if dir.as_os_str().is_empty() {
        PathBuf::from(program)
    } else {
        dir.join(program)
    };
    resolve_dir(command, joined)
}

/// `true` when the path exists, or when we could not tell.
///
/// Only a definite "not found" is treated as a missing executable. A directory,
/// a non-executable file, or a permission error is left for `execve`, which
/// reports `EACCES` rather than `ENOENT` when it finds something it cannot run.
#[cfg(unix)]
fn path_exists(path: &Path) -> bool {
    match path.metadata() {
        Ok(_) => true,
        Err(err) if err.kind() == io::ErrorKind::NotFound => false,
        Err(_) => true,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::io;
    use std::io::ErrorKind;
    use std::os::unix::fs::PermissionsExt as _;
    use std::process::Command;

    use rustix::io::Errno;

    use super::command_spawn;
    use super::command_status;

    fn assert_not_found(err: io::Error) {
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert_eq!(err.raw_os_error(), Some(Errno::NOENT.raw_os_error()));
    }

    #[test]
    fn missing_program_is_not_found() {
        let mut command = Command::new("jj-test-missing-program-subprocess-util");
        assert_not_found(command_status(&mut command).unwrap_err());

        let mut command = Command::new("/no/such/jj-test-missing-program-subprocess-util");
        assert_not_found(command_spawn(&mut command).unwrap_err());
    }

    #[test]
    fn existing_program_exit_127_is_preserved() {
        let mut command = Command::new("sh");
        command.args(["-c", "exit 127"]);
        let status = command_status(&mut command).unwrap();
        assert_eq!(status.code(), Some(127));
    }

    #[test]
    fn path_override_hides_an_existing_program() {
        let mut command = Command::new("sh");
        command.env("PATH", "/jj-test-empty-path-subprocess-util");
        command.args(["-c", "exit 0"]);
        assert_not_found(command_status(&mut command).unwrap_err());
    }

    #[test]
    fn relative_program_uses_current_dir() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("tool");
        std::fs::write(&program, "#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = std::fs::metadata(&program).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&program, permissions).unwrap();

        let mut command = Command::new("./tool");
        command.current_dir(dir.path());
        let status = command_status(&mut command).unwrap();
        assert!(status.success());

        let mut command = Command::new("./tool");
        command.current_dir("/");
        assert_not_found(command_status(&mut command).unwrap_err());
    }
}
