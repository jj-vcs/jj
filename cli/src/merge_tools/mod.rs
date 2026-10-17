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

mod builtin;

use async_trait::async_trait;
use jj_frontend::merge_tools::BuiltinMergeTool;
use jj_frontend::merge_tools::MergeToolFile;
use jj_lib::conflicts::ConflictMarkerStyle;
use jj_lib::matchers::Matcher;
use jj_lib::merge::Diff;
use jj_lib::merged_tree::MergedTree;

pub use self::builtin::BuiltinToolError;
use self::builtin::edit_diff_builtin;
use self::builtin::edit_merge_builtin;

/// The `:builtin` diff editor and merge tool, which runs in the terminal.
#[derive(Debug)]
pub struct TerminalBuiltinMergeTool;

#[async_trait(?Send)]
impl BuiltinMergeTool for TerminalBuiltinMergeTool {
    async fn edit_diff(
        &self,
        trees: Diff<&MergedTree>,
        matcher: &dyn Matcher,
        conflict_marker_style: ConflictMarkerStyle,
    ) -> Result<MergedTree, Box<dyn std::error::Error + Send + Sync>> {
        Ok(edit_diff_builtin(trees, matcher, conflict_marker_style).await?)
    }

    async fn edit_merge(
        &self,
        tree: &MergedTree,
        merge_tool_files: &[MergeToolFile],
    ) -> Result<MergedTree, Box<dyn std::error::Error + Send + Sync>> {
        Ok(edit_merge_builtin(tree, merge_tool_files).await?)
    }
}
