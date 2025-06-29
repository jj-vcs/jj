use std::collections::BTreeMap;
use std::path::PathBuf;

use jj_lib::ref_name::{WorkspaceName, WorkspaceNameBuf};
use jj_lib::workspace::{WorkspaceLoaderFactory, WorkspaceLoader};

/// A table mapping workspace names to their root paths and loader factory.
///
/// This keeps workspace path metadata outside the content-hashed view,
/// so that view IDs remain stable when paths change.
pub struct WorkspaceLoaderTable {
    factory: Box<dyn WorkspaceLoaderFactory>,
    roots: BTreeMap<WorkspaceNameBuf, PathBuf>,
}

impl WorkspaceLoaderTable {
    /// Create a new workspace loader table with the given factory.
    pub fn new(factory: Box<dyn WorkspaceLoaderFactory>) -> Self {
        Self { factory, roots: BTreeMap::new() }
    }

    /// Record or update the root path for a named workspace.
    pub fn set_root(&mut self, name: WorkspaceNameBuf, root: PathBuf) {
        self.roots.insert(name, root);
    }

    /// Remove a named workspace from the table.
    pub fn remove_root(&mut self, name: &WorkspaceName) {
        self.roots.remove(name);
    }

    /// Rename an entry when the workspace name changes.
    pub fn rename(&mut self, old: &WorkspaceName, new: WorkspaceNameBuf) {
        if let Some(root) = self.roots.remove(old) {
            self.roots.insert(new, root);
        }
    }

    /// Get the recorded root path for a named workspace, if any.
    pub fn get_root(&self, name: &WorkspaceName) -> Option<&PathBuf> {
        self.roots.get(name)
    }

    /// Get a loader for the named workspace, if registered.
    pub fn loader(
        &self,
        name: &WorkspaceName,
    ) -> Option<Result<Box<dyn WorkspaceLoader>, jj_lib::workspace::WorkspaceLoadError>> {
        self.roots.get(name).map(|root| self.factory.create(root))
    }
}