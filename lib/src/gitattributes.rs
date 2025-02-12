use gix_attributes;
use std::borrow::Cow;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitAttributesError {
    #[error("Failed to read ignore patterns from file {path}")]
    ReadFile { path: PathBuf, source: io::Error },
}

/// Models the effective contents of multiple .gitignore files.
#[derive(Debug)]
pub struct GitAttributesFile {
    search: gix_attributes::Search,
    collection: gix_attributes::search::MetadataCollection,
}

impl GitAttributesFile {
    /// Concatenates new `.gitattributes` file.
    pub fn chain_with_file(
        self: &Arc<GitAttributesFile>,
        file: PathBuf,
    ) -> Result<Arc<GitAttributesFile>, GitAttributesError> {
        if file.is_file() {
            let mut buf = Vec::new();
            let mut search = self.search.clone();
            let mut collection = self.collection.clone();
            search
                .add_patterns_file(
                    file.clone(),
                    true,
                    Some(Path::new(".")),
                    &mut buf,
                    &mut collection,
                    true,
                )
                .map_err(|err| GitAttributesError::ReadFile {
                    path: file.clone(),
                    source: err,
                })?;
            Ok(Arc::new(GitAttributesFile { search, collection }))
        } else {
            Ok(self.clone())
        }
    }

    pub fn matches(&self, path: &str) -> bool {
        //If path ends with slash, consider it as a directory.
        let (path, is_dir) = match path.strip_suffix('/') {
            Some(path) => (path, true),
            None => (path, false),
        };

        let mut out = gix_attributes::search::Outcome::default();
        out.initialize_with_selection(&self.collection, ["filter"]);
        self.search.pattern_matching_relative_path(
            path.into(),
            gix_glob::pattern::Case::Sensitive,
            Some(is_dir),
            &mut out,
        );

        let is_lfs = out
            .iter_selected()
            .filter_map(|attr| {
                if let gix_attributes::StateRef::Value(value_ref) = attr.assignment.state {
                    Some(value_ref.as_bstr())
                } else {
                    None
                }
            })
            .any(|value| value == "lfs");
        is_lfs
    }
}

impl Default for GitAttributesFile {
    fn default() -> Self {
        let files = [
            gix_attributes::Source::GitInstallation,
            gix_attributes::Source::System,
            gix_attributes::Source::Git,
            gix_attributes::Source::Local,
        ]
        .iter()
        .filter_map(|source| {
            source
                .storage_location(&mut gix_path::env::var)
                .and_then(|p| p.is_file().then_some(p))
                .map(Cow::into_owned)
        });

        let mut buf = Vec::new();
        let mut collection = gix_attributes::search::MetadataCollection::default();
        let search = gix_attributes::Search::new_globals(files, &mut buf, &mut collection)
            .unwrap_or_else(|_| gix_attributes::Search::default());

        GitAttributesFile { search, collection }
    }
}
