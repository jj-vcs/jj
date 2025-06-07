// Copyright 2025 The Jujutsu Authors
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

mod read;
mod write;

#[cfg(feature = "git")]
use std::fs::File;
#[cfg(feature = "git")]
use std::io::Read as _;
use std::path::Path;
use std::sync::Arc;

#[cfg(feature = "git")]
use gix::filter::plumbing::eol::AutoCrlf;
pub(crate) use read::ReadExt;
pub(crate) use write::WriteExt;

use crate::backend::FileId;
#[cfg(feature = "git")]
use crate::git_backend::GitBackend;
use crate::repo_path::RepoPath;
use crate::store::Store;

/// The target EOL to convert to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TargetEol {
    /// Do not convert EOL.
    PassThrough,
    /// Convert to CRLF (Carriage Return Line Feed, `0x0D 0x0A`, `\r\n`).
    #[allow(
        unused,
        reason = "if the git feature is not enabled, the CRLF target EOL won't be used"
    )]
    Crlf,
    /// Convert to LF (Line Feed, `0x0A`, `\n`).
    #[allow(
        unused,
        reason = "if the git feature is not enabled, the LF target EOL won't be used"
    )]
    Lf,
}

// Read at most 8KB to decide whether this file is binary.
#[cfg(feature = "git")]
const PROBE_SIZE: usize = 8 << 10;

pub(crate) struct TargetEolStrategy {
    #[cfg(feature = "git")]
    auto_crlf: Option<AutoCrlf>,
    #[allow(unused)]
    store: Arc<Store>,
}

impl TargetEolStrategy {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            #[cfg(feature = "git")]
            auto_crlf: {
                use gix::config::tree::Core;
                store
                    .backend_impl()
                    .downcast_ref::<GitBackend>()
                    .and_then(|git_backend| {
                        let config = git_backend.git_config();
                        let auto_crlf = config.raw_value(Core::AUTO_CRLF).ok()?;
                        Core::AUTO_CRLF.try_into_autocrlf(auto_crlf).ok()
                    })
            },
            store,
        }
    }

    pub fn get_snapshot_reader_target_eol(
        &self,
        #[cfg_attr(not(feature = "git"), allow(unused))] file_path: &Path,
    ) -> TargetEol {
        cfg_if::cfg_if! {
            if #[cfg(feature = "git")] {
                fn is_file_binary(file_path: &Path) -> Option<bool> {
                    let file =
                        File::options()
                            .read(true)
                            .open(file_path)
                            .unwrap();

                    let mut first = file.take(PROBE_SIZE as u64);
                    let mut content = Vec::with_capacity(PROBE_SIZE);
                    first.read_to_end(&mut content).ok()?;
                    let stats = gix::filter::plumbing::eol::Stats::from_bytes(&content);
                    Some(stats.is_binary())
                }

                if let Some(auto_crlf) = self.auto_crlf {
                    match auto_crlf {
                        AutoCrlf::Disabled => return TargetEol::PassThrough,
                        AutoCrlf::Enabled | AutoCrlf::Input => {
                            match is_file_binary(file_path) {
                                Some(true) => return TargetEol::PassThrough,
                                Some(false) => return TargetEol::Lf,
                                None => {
                                    // Fall through to the default.
                                }
                            }
                        }
                    }
                }
            }
        }
        TargetEol::PassThrough
    }

    pub async fn get_update_writer_target_eol(
        &self,
        #[cfg_attr(not(feature = "git"), allow(unused))] repo_path: &RepoPath,
        #[cfg_attr(not(feature = "git"), allow(unused))] file_id: &FileId,
    ) -> TargetEol {
        cfg_if::cfg_if! {
            if #[cfg(feature = "git")] {
                use tokio::io::AsyncReadExt as _;

                async fn is_file_binary(
                    repo_path: &RepoPath,
                    file_id: &FileId,
                    store: &Store
                ) -> Option<bool> {
                    let reader = store.read_file(repo_path, file_id).await.ok()?;
                    let mut content = Vec::with_capacity(PROBE_SIZE);
                    reader.take(PROBE_SIZE as u64).read_to_end(&mut content).await.ok()?;
                    let stats = gix::filter::plumbing::eol::Stats::from_bytes(&content);
                    Some(stats.is_binary())
                }

                if let Some(auto_crlf) = self.auto_crlf {
                    match auto_crlf {
                        AutoCrlf::Disabled | AutoCrlf::Input => return TargetEol::PassThrough,
                        AutoCrlf::Enabled => {
                            match is_file_binary(repo_path, file_id, &self.store).await {
                                Some(true) => return TargetEol::PassThrough,
                                Some(false) => return TargetEol::Crlf,
                                None => {
                                    // Fall through to the default.
                                }
                            }
                        }
                    }
                }
            }
        }
        TargetEol::PassThrough
    }
}
