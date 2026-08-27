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
mod track;
mod untrack;

use std::io;

use itertools::Itertools as _;
use jj_lib::op_store::RemoteRef;
use jj_lib::ref_name::RefName;
use jj_lib::ref_name::RemoteName;
use jj_lib::ref_name::RemoteRefSymbol;
use jj_lib::ref_name::RemoteRefSymbolBuf;
use jj_lib::revset;
use jj_lib::str_util::StringExpression;
use jj_lib::str_util::StringMatcher;
use jj_lib::view::View;

use self::delete::TagDeleteArgs;
use self::delete::cmd_tag_delete;
use self::list::TagListArgs;
use self::list::cmd_tag_list;
use self::set::TagSetArgs;
use self::set::cmd_tag_set;
use self::track::TagTrackArgs;
use self::track::cmd_tag_track;
use self::untrack::TagUntrackArgs;
use self::untrack::cmd_tag_untrack;
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
    Track(TagTrackArgs),
    Untrack(TagUntrackArgs),
}

pub async fn cmd_tag(
    ui: &mut Ui,
    command: &CommandHelper,
    subcommand: &TagCommand,
) -> Result<(), CommandError> {
    match subcommand {
        TagCommand::Delete(args) => cmd_tag_delete(ui, command, args).await,
        TagCommand::List(args) => cmd_tag_list(ui, command, args).await,
        TagCommand::Set(args) => cmd_tag_set(ui, command, args).await,
        TagCommand::Track(args) => cmd_tag_track(ui, command, args).await,
        TagCommand::Untrack(args) => cmd_tag_untrack(ui, command, args).await,
    }
}

fn resolve_trackable_remote_tags<'a>(
    ui: &Ui,
    view: &'a View,
    symbols: &'a [RemoteRefSymbolBuf],
) -> Result<Vec<(RemoteRefSymbol<'a>, &'a RemoteRef)>, CommandError> {
    let mut trackable_refs = vec![];
    let mut unmatched_symbols = vec![];
    for symbol in symbols {
        let symbol = symbol.as_ref();
        let remote_ref = view.get_remote_tag(symbol);
        if remote_ref.is_present()
            || view.get_local_tag(symbol.name).is_present()
                && view.get_remote_view(symbol.remote).is_some()
        {
            trackable_refs.push((symbol, remote_ref));
        } else {
            unmatched_symbols.push(symbol);
        }
    }
    trackable_refs.sort_unstable_by_key(|(sym, _)| *sym);
    trackable_refs.dedup_by(|(sym1, _), (sym2, _)| sym1 == sym2);
    if !unmatched_symbols.is_empty() {
        writeln!(
            ui.warning_default(),
            "No matching remote tags for names: {}",
            unmatched_symbols
                .iter()
                .map(|&symbol| revset::format_remote_ref_symbol(symbol))
                .join(", ")
        )?;
    }
    Ok(trackable_refs)
}

fn trackable_remote_tags_matching<'a>(
    view: &'a View,
    tag_matcher: &StringMatcher,
    remote_matcher: &StringMatcher,
) -> impl Iterator<Item = (RemoteRefSymbol<'a>, &'a RemoteRef)> {
    let present_or_tracked_matches = view.remote_tags_matching(tag_matcher, remote_matcher);
    let absent_matches =
        view.remote_views_matching(remote_matcher)
            .flat_map(move |(remote, remote_view)| {
                view.local_tags_matching(tag_matcher)
                    .filter(|&(name, _)| !remote_view.tags.contains_key(name))
                    .map(|(name, _)| (name.to_remote_symbol(remote), RemoteRef::absent_ref()))
            });
    itertools::chain(present_or_tracked_matches, absent_matches)
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
        names.map(revset::format_ref_name).join(", ")
    )
}

/// Warns about exact patterns that don't match local or remote tags.
fn warn_unmatched_local_or_remote_tags(
    ui: &Ui,
    view: &View,
    name_expr: &StringExpression,
) -> io::Result<()> {
    let mut names = name_expr
        .exact_strings()
        .map(RefName::new)
        .filter(|&name| {
            view.get_local_tag(name).is_absent()
                && view
                    .remote_views()
                    .all(|(_, remote_view)| !remote_view.tags.contains_key(name))
        })
        .peekable();
    if names.peek().is_none() {
        return Ok(());
    }
    writeln!(
        ui.warning_default(),
        "No matching tags for names: {}",
        names.map(revset::format_ref_name).join(", ")
    )
}

/// Warns about exact patterns that don't match remotes.
fn warn_unmatched_remotes(ui: &Ui, view: &View, name_expr: &StringExpression) -> io::Result<()> {
    let mut names = name_expr
        .exact_strings()
        .map(RemoteName::new)
        .filter(|name| view.get_remote_view(name).is_none())
        .peekable();
    if names.peek().is_none() {
        return Ok(());
    }
    writeln!(
        ui.warning_default(),
        "No matching remotes for names: {}",
        names.map(revset::format_ref_name).join(", ")
    )
}
