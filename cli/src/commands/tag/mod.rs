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

use std::io;

use itertools::Itertools as _;
use jj_lib::ref_name::RefName;
use jj_lib::str_util::StringExpression;
use jj_lib::view::View;

use self::delete::TagDeleteArgs;
use self::delete::cmd_tag_delete;
use self::list::TagListArgs;
use self::list::cmd_tag_list;
use self::set::TagSetArgs;
use self::set::cmd_tag_set;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
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

/// Warns about exact patterns that don't match local tags.
fn warn_unmatched_local_tags(ui: &Ui, view: &View, name_expr: &StringExpression) -> io::Result<()> {
    let mut names = name_expr
        .exact_strings()
        .map(RefName::new)
        .filter(|name| view.get_local_tag(name).is_absent())
        .peekable();
    if names.peek().is_none() {
        return Ok(());
    }
    writeln!(
        ui.warning_default(),
        "No matching tags for names: {}",
        names.map(|name| name.as_symbol()).join(", ")
    )
}
