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

use std::fs;

use jj_lib::conflicts::ConflictMaterializeOptions;
use jj_lib::file_util::IoResultExt as _;
use jj_lib::merge::Diff;
use jj_lib::merge::Merge;
use jj_lib::tree_merge::MergeOptions;

use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::diff_util;
use crate::diff_util::DiffFormatArgs;
use crate::formatter::FormatterExt as _;
use crate::ui::Ui;

/// Compare two files on disk
#[derive(clap::Args, Clone, Debug)]
// Hide useless short-format args. They could be removed entirely, but these
// formats can still be enabled via --tool=:<format>.
#[command(mut_arg("summary", |a| a.hide(true)))]
#[command(mut_arg("stat", |a| a.hide(true)))]
#[command(mut_arg("types", |a| a.hide(true)))]
#[command(mut_arg("name_only", |a| a.hide(true)))]
#[command(mut_arg("ignore_all_space", |a| a.short('w')))]
#[command(mut_arg("ignore_space_change", |a| a.short('b')))]
pub struct UtilDiffArgs {
    /// First path to compare
    #[arg(value_hint = clap::ValueHint::FilePath)]
    path1: String,

    /// Second path to compare
    #[arg(value_hint = clap::ValueHint::FilePath)]
    path2: String,

    #[command(flatten)]
    format: DiffFormatArgs,
}

pub async fn cmd_util_diff(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &UtilDiffArgs,
) -> Result<(), CommandError> {
    let formats = diff_util::diff_formats_for(command.settings(), &args.format)?;
    let materialize_options = ConflictMaterializeOptions {
        marker_style: command.settings().get("ui.conflict-marker-style")?,
        marker_len: None,
        merge: MergeOptions::from_settings(command.settings())?,
    };

    let paths: Diff<&str> = Diff::new(&args.path1, &args.path2);
    let content1 = Merge::resolved(fs::read(&args.path1).context(&args.path1)?);
    let content2 = Merge::resolved(fs::read(&args.path2).context(&args.path2)?);
    let contents = Diff::new(&content1, &content2);

    ui.request_pager();
    diff_util::show_diff_bytes(
        *ui.stdout_formatter().labeled("diff"),
        &formats,
        paths,
        contents,
        &materialize_options,
    )?;
    Ok(())
}
