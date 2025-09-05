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

use std::process::Command;

use jj_lib::config::ConfigGetError;
use jj_lib::config::ConfigGetResultExt;
use jj_lib::settings::UserSettings;

use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::command_error::user_error_with_message;

#[derive(Debug)]
pub enum HookEvent {
    Commit,
    Squash,
}

#[derive(Debug)]
struct HooksSettings {
    post_commit: Vec<String>,
    post_squash: Vec<String>,
}

impl HooksSettings {
    fn from_settings(settings: &UserSettings) -> Result<Self, ConfigGetError> {
        Ok(Self {
            post_commit: settings
                .get("hooks.post-commit")
                .optional()?
                .unwrap_or_default(),
            post_squash: settings
                .get("hooks.post-squash")
                .optional()?
                .unwrap_or_default(),
        })
    }
}

fn run(command: &[String]) -> Result<(), CommandError> {
    if command.is_empty() {
        return Ok(());
    }
    let status = Command::new(&command[0])
        .args(&command[1..])
        .status()
        .map_err(|err| {
            user_error_with_message(format!("Hook '{}' failed to run", command[0],), err)
        })?;
    match status.code() {
        Some(0) => Ok(()),
        Some(exit_code) => Err(user_error(format!(
            "Hook '{}' exited with code {}",
            command[0], exit_code
        ))),
        None => Err(user_error(format!(
            "Hook '{}' was terminated by {}",
            command[0], status
        ))),
    }
}

pub fn run_post_hook_for_event(
    settings: &UserSettings,
    event: HookEvent,
) -> Result<(), CommandError> {
    let hooks_settings = HooksSettings::from_settings(settings)?;
    match event {
        HookEvent::Commit => run(&hooks_settings.post_commit),
        HookEvent::Squash => run(&hooks_settings.post_squash),
    }
}
