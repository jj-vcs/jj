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

//! Common helpers for configuring subprocesses

use std::process::Command;

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

/// Returns whether a console is attached to the current process.
///
/// Opening `CONOUT$` is how Git probes for a console. Neither
/// `GetConsoleWindow()` nor [`std::io::IsTerminal`] can be used instead: the
/// former returns null for a console that has no window, and the latter is
/// false whenever the standard streams are redirected, even though a console
/// is still attached and usable for prompting.
#[cfg(windows)]
fn has_console() -> bool {
    std::fs::OpenOptions::new()
        .write(true)
        .open("CONOUT$")
        .is_ok()
}
