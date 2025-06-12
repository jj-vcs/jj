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

#![deny(unused_must_use)]

pub mod cleanup_guard;
pub mod cli_util;
pub mod command_error;
pub mod commands;
pub mod commit_templater;
pub mod complete;
pub mod config;
pub mod description_util;
pub mod diff_util;
pub mod formatter;
pub mod generic_templater;
#[cfg(feature = "git")]
pub mod git_util;
#[cfg(not(feature = "git"))]
/// A stub module that provides a no-op implementation of some of the functions
/// in the `git` module.
pub mod git_util {
    use jj_lib::repo::ReadonlyRepo;
    use jj_lib::workspace::Workspace;

    pub fn is_colocated_git_workspace(_workspace: &Workspace, _repo: &ReadonlyRepo) -> bool {
        false
    }
}
pub mod graphlog;
pub mod merge_tools;
pub mod movement_util;
pub mod operation_templater;
mod progress;
pub mod revset_util;
pub mod template_builder;
pub mod template_parser;
pub mod templater;
pub mod text_util;
pub mod time_util;
pub mod ui;

fn home_dir() -> Result<camino::Utf8PathBuf, etcetera::HomeDirError> {
    etcetera::home_dir()?
        .try_into()
        .map_err(|_err| etcetera::HomeDirError)
}
