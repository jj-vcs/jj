// Copyright 2024 The Jujutsu Authors
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

use std::io;
use std::path::Path;

use crate::ui::Ui;

/// Supported filesystems for copy-on-write semantics
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilesystemType {
    /// No Copy-on-write semantics supported.
    /// We just use normal filesystem operations.
    Generic,
    /// Btrfs isn't available on other OSes
    #[cfg(target_os = "linux")]
    Btrfs,
}

/// Calculates the filesystem that a path exists on.
pub fn filesystem_type(p: &Path) -> FilesystemType {
    #[cfg(target_os = "linux")]
    if libbtrfs::fs::is_btrfs(p).unwrap_or(false) {
        return FilesystemType::Btrfs;
    }
    let _ = p;
    FilesystemType::Generic
}

/// Creates a directory that can be CoW'd in the future.
pub fn create_cow_dir(ui: &mut Ui, p: &Path) -> io::Result<()> {
    let is_dir = p.is_dir();
    let fs_type = if is_dir {
        filesystem_type(p)
    } else if let Some(parent) = p.parent() {
        filesystem_type(parent)
    } else {
        FilesystemType::Generic
    };
    match fs_type {
        FilesystemType::Generic => {}
        #[cfg(target_os = "linux")]
        FilesystemType::Btrfs => {
            if is_dir {
                if !libbtrfs::subvol::is_subvol(p).unwrap_or(false) {
                    writeln!(
                        ui.warning_default(),
                        "Not a subvolume. Copy-on-write features will be disabled when using jj \
                         workspaces"
                    )?;
                }
                return Ok(());
            } else {
                match libbtrfs::subvol::create(p) {
                    Ok(()) => return Ok(()),
                    Err(err) => {
                        writeln!(
                            ui.warning_default(),
                            "Failed to create Btrfs subvolume at {}: {err}. Falling back to \
                             standard directory.",
                            p.display()
                        )?;
                    }
                }
            }
        }
    };
    jj_lib::file_util::create_or_reuse_dir(p)
}
