// Copyright 2020-2024 The Jujutsu Authors
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

mod delete;
mod list;
mod set;

use itertools::Itertools as _;
use jj_lib::op_store::RefTarget;
use jj_lib::ref_name::RefName;
use jj_lib::str_util::StringMatcher;
use jj_lib::str_util::StringPattern;
use jj_lib::view::View;

use self::delete::TagDeleteArgs;
use self::delete::cmd_tag_delete;
use self::list::TagListArgs;
use self::list::cmd_tag_list;
use self::set::TagSetArgs;
use self::set::cmd_tag_set;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::command_error::user_error;
use crate::ui::Ui;

/// Manage tags.
#[derive(clap::Subcommand, Clone, Debug)]
pub enum TagCommand {
    #[command(visible_alias("d"))]
    Delete(TagDeleteArgs),
    #[command(visible_alias("l"))]
    List(TagListArgs),
    #[command(visible_alias("s"))]
    Set(TagSetArgs),
}

pub fn cmd_tag(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &TagCommand,
) -> Result<(), CommandError> {
    match subcommand {
        TagCommand::Delete(args) => cmd_tag_delete(ui, command, args),
        TagCommand::List(args) => cmd_tag_list(ui, command, args),
        TagCommand::Set(args) => cmd_tag_set(ui, command, args),
    }
}

fn find_local_tags<'a>(
    view: &'a View,
    name_patterns: &[StringPattern],
) -> Result<Vec<(&'a RefName, &'a RefTarget)>, CommandError> {
    find_tags_with(name_patterns, |matcher| {
        Ok(view.local_tags_matching(matcher).collect())
    })
}

fn find_tags_with<'a, V>(
    name_patterns: &[StringPattern],
    mut find_matches: impl FnMut(&StringMatcher) -> Result<Vec<(&'a RefName, V)>, CommandError>,
) -> Result<Vec<(&'a RefName, V)>, CommandError> {
    let mut matching_tags: Vec<(&'a RefName, V)> = vec![];
    let mut unmatched_patterns = vec![];
    for pattern in name_patterns {
        let matches = find_matches(&pattern.to_matcher())?;
        if matches.is_empty() {
            unmatched_patterns.push(pattern);
        }
        matching_tags.extend(matches);
    }
    match &unmatched_patterns[..] {
        [] => {
            matching_tags.sort_unstable_by_key(|(name, _)| *name);
            matching_tags.dedup_by_key(|(name, _)| *name);
            Ok(matching_tags)
        }
        [pattern] if pattern.is_exact() => Err(user_error(format!("No such tag: {pattern}"))),
        patterns => Err(user_error(format!(
            "No matching tags for patterns: {}",
            patterns.iter().join(", ")
        ))),
    }
}
