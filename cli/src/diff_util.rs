use std::path::Path;
use jj_lib::backend::{BackendError, BackendResult, TreeValue};
use jj_lib::{diff, file_util, files};
use thiserror::Error;
use crate::merge_tools::{self, DiffGenerateError, ExternalMergeTool};
#[derive(Debug, Error)]
pub enum DiffRenderError {
    #[error("Failed to generate diff")]
    DiffGenerate(#[source] DiffGenerateError),
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Workspace information needed to render textual diff.
#[derive(Clone, Debug)]
pub struct DiffWorkspaceContext<'a> {
    pub cwd: &'a Path,
    pub workspace_root: &'a Path,
}

impl DiffWorkspaceContext<'_> {
    fn format_file_path(&self, file: &RepoPath) -> String {
        file_util::relative_path(self.cwd, &file.to_fs_path(self.workspace_root))
            .to_str()
            .unwrap()
            .to_owned()
    }
}

/// Configuration and environment to render textual diff.
pub struct DiffRenderer<'a> {
    repo: &'a dyn Repo,
    workspace_ctx: DiffWorkspaceContext<'a>,
    formats: Vec<DiffFormat>,
}

impl<'a> DiffRenderer<'a> {
    pub fn new(
        repo: &'a dyn Repo,
        workspace_ctx: DiffWorkspaceContext<'a>,
        formats: Vec<DiffFormat>,
    ) -> Self {
        DiffRenderer {
            repo,
            formats,
            workspace_ctx,
        }
    }

    /// Generates diff between `from_tree` and `to_tree`.
    pub fn show_diff(
        &self,
        ui: &Ui, // TODO: remove Ui dependency if possible
        formatter: &mut dyn Formatter,
        from_tree: &MergedTree,
        to_tree: &MergedTree,
        matcher: &dyn Matcher,
    ) -> Result<(), DiffRenderError> {
        let repo = self.repo;
        let workspace_ctx = &self.workspace_ctx;
        for format in &self.formats {
            match format {
                DiffFormat::Summary => {
                    let tree_diff = from_tree.diff_stream(to_tree, matcher);
                    show_diff_summary(formatter, tree_diff, workspace_ctx)?;
                }
                DiffFormat::Stat => {
                    let tree_diff = from_tree.diff_stream(to_tree, matcher);
                    // TODO: In graph log, graph width should be subtracted
                    let width = usize::from(ui.term_width().unwrap_or(80));
                    show_diff_stat(repo, formatter, tree_diff, workspace_ctx, width)?;
                }
                DiffFormat::Types => {
                    let tree_diff = from_tree.diff_stream(to_tree, matcher);
                    show_types(formatter, tree_diff, workspace_ctx)?;
                }
                DiffFormat::Git { context } => {
                    let tree_diff = from_tree.diff_stream(to_tree, matcher);
                    show_git_diff(repo, formatter, *context, tree_diff)?;
                }
                DiffFormat::ColorWords { context } => {
                    let tree_diff = from_tree.diff_stream(to_tree, matcher);
                    show_color_words_diff(repo, formatter, *context, tree_diff, workspace_ctx)?;
                }
                DiffFormat::Tool(tool) => {
                    merge_tools::generate_diff(
                        ui,
                        formatter.raw(),
                        from_tree,
                        to_tree,
                        matcher,
                        tool,
                    )
                    .map_err(DiffRenderError::DiffGenerate)?;
                }
        Ok(())
    /// Generates diff of the given `commit` compared to its parents.
    pub fn show_patch(
        &self,
        ui: &Ui,
        formatter: &mut dyn Formatter,
        commit: &Commit,
        matcher: &dyn Matcher,
    ) -> Result<(), DiffRenderError> {
        let from_tree = commit.parent_tree(self.repo)?;
        let to_tree = commit.tree()?;
        self.show_diff(ui, formatter, &from_tree, &to_tree, matcher)
    }
fn diff_content(path: &RepoPath, value: MaterializedTreeValue) -> io::Result<FileContent> {
    repo: &dyn Repo,
    workspace_ctx: &DiffWorkspaceContext,
) -> Result<(), DiffRenderError> {
    let mut diff_stream = materialized_diff_stream(repo.store(), tree_diff);
            let ui_path = workspace_ctx.format_file_path(&path);
        Ok::<(), DiffRenderError>(())
fn git_diff_part(path: &RepoPath, value: MaterializedTreeValue) -> io::Result<GitDiffPart> {
) -> io::Result<()> {
    repo: &dyn Repo,
) -> Result<(), DiffRenderError> {
    let mut diff_stream = materialized_diff_stream(repo.store(), tree_diff);
        Ok::<(), DiffRenderError>(())
    workspace_ctx: &DiffWorkspaceContext,
                let ui_path = workspace_ctx.format_file_path(&repo_path);
                    writeln!(formatter.labeled("modified"), "M {ui_path}")?;
                    writeln!(formatter.labeled("added"), "A {ui_path}")?;
                    // `R` could be interpreted as "renamed"
                    writeln!(formatter.labeled("removed"), "D {ui_path}")?;
    repo: &dyn Repo,
    workspace_ctx: &DiffWorkspaceContext,
    display_width: usize,
) -> Result<(), DiffRenderError> {
    let mut diff_stream = materialized_diff_stream(repo.store(), tree_diff);
            let path = workspace_ctx.format_file_path(&repo_path);
        Ok::<(), DiffRenderError>(())
    let available_width = display_width.saturating_sub(4 + " | ".len() + number_padding);
    workspace_ctx: &DiffWorkspaceContext,
                    workspace_ctx.format_file_path(&repo_path)