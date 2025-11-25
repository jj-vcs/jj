// Copyright 2022 The Jujutsu Authors
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

//! Configuration store helpers.

use std::borrow::Borrow;
use std::convert::Infallible;
use std::fmt;
use std::fmt::Display;
use std::fs;
use std::io;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;
use std::slice;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::LazyLock;

use itertools::Itertools as _;
use serde::Deserialize;
use serde::de::IntoDeserializer as _;
use thiserror::Error;
use toml_edit::Document;
use toml_edit::DocumentMut;

pub use crate::config_resolver::ConfigMigrateError;
pub use crate::config_resolver::ConfigMigrateLayerError;
pub use crate::config_resolver::ConfigMigrationRule;
pub use crate::config_resolver::ConfigResolutionContext;
pub use crate::config_resolver::migrate;
pub use crate::config_resolver::resolve;
use crate::file_util::IoResultExt as _;
use crate::file_util::PathError;

/// Config value or table node.
pub type ConfigItem = toml_edit::Item;
/// Non-inline table of config key and value pairs.
pub type ConfigTable = toml_edit::Table;
/// Non-inline or inline table of config key and value pairs.
pub type ConfigTableLike<'a> = dyn toml_edit::TableLike + 'a;
/// Generic config value.
pub type ConfigValue = toml_edit::Value;

/// Error that can occur when parsing or loading config variables.
#[derive(Debug, Error)]
pub enum ConfigLoadError {
    /// Config file or directory cannot be read.
    #[error("Failed to read configuration file")]
    Read(#[source] PathError),
    /// TOML file or text cannot be parsed.
    #[error("Configuration cannot be parsed as TOML document")]
    Parse {
        /// Source error.
        #[source]
        error: Box<toml_edit::TomlError>,
        /// Source file path.
        source_path: Option<PathBuf>,
    },
}

/// Error that can occur when saving config variables to file.
#[derive(Debug, Error)]
#[error("Failed to write configuration file")]
pub struct ConfigFileSaveError(#[source] pub PathError);

/// Error that can occur when looking up config variable.
#[derive(Debug, Error)]
pub enum ConfigGetError {
    /// Config value is not set.
    #[error("Value not found for {name}")]
    NotFound {
        /// Dotted config name path.
        name: String,
    },
    /// Config value cannot be converted to the expected type.
    #[error("Invalid type or value for {name}")]
    Type {
        /// Dotted config name path.
        name: String,
        /// Source error.
        #[source]
        error: Box<dyn std::error::Error + Send + Sync>,
        /// Source file path where the value is defined.
        source_path: Option<PathBuf>,
    },
}

/// Error that can occur when updating config variable.
#[derive(Debug, Error)]
pub enum ConfigUpdateError {
    /// Non-table value exists at parent path, which shouldn't be removed.
    #[error("Would overwrite non-table value with parent table {name}")]
    WouldOverwriteValue {
        /// Dotted config name path.
        name: String,
    },
    /// Non-inline table exists at the path, which shouldn't be overwritten by a
    /// value.
    #[error("Would overwrite entire table {name}")]
    WouldOverwriteTable {
        /// Dotted config name path.
        name: String,
    },
    /// Non-inline table exists at the path, which shouldn't be deleted.
    #[error("Would delete entire table {name}")]
    WouldDeleteTable {
        /// Dotted config name path.
        name: String,
    },
}

/// Extension methods for `Result<T, ConfigGetError>`.
pub trait ConfigGetResultExt<T> {
    /// Converts `NotFound` error to `Ok(None)`, leaving other errors.
    fn optional(self) -> Result<Option<T>, ConfigGetError>;
}

impl<T> ConfigGetResultExt<T> for Result<T, ConfigGetError> {
    fn optional(self) -> Result<Option<T>, ConfigGetError> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(ConfigGetError::NotFound { .. }) => Ok(None),
            Err(err) => Err(err),
        }
    }
}

/// Dotted config name path.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ConfigNamePathBuf(Vec<toml_edit::Key>);

impl ConfigNamePathBuf {
    /// Creates an empty path pointing to the root table.
    ///
    /// This isn't a valid TOML key expression, but provided for convenience.
    pub fn root() -> Self {
        Self(vec![])
    }

    /// Returns true if the path is empty (i.e. pointing to the root table.)
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns true if the `base` is a prefix of this path.
    pub fn starts_with(&self, base: impl AsRef<[toml_edit::Key]>) -> bool {
        self.0.starts_with(base.as_ref())
    }

    /// Returns iterator of path components (or keys.)
    pub fn components(&self) -> slice::Iter<'_, toml_edit::Key> {
        self.0.iter()
    }

    /// Appends the given `key` component.
    pub fn push(&mut self, key: impl Into<toml_edit::Key>) {
        self.0.push(key.into());
    }
}

// Help obtain owned value from ToConfigNamePath::Output. If we add a slice
// type (like &Path for PathBuf), this will be From<&ConfigNamePath>.
impl From<&Self> for ConfigNamePathBuf {
    fn from(value: &Self) -> Self {
        value.clone()
    }
}

impl<K: Into<toml_edit::Key>> FromIterator<K> for ConfigNamePathBuf {
    fn from_iter<I: IntoIterator<Item = K>>(iter: I) -> Self {
        let keys = iter.into_iter().map(|k| k.into()).collect();
        Self(keys)
    }
}

impl FromStr for ConfigNamePathBuf {
    type Err = toml_edit::TomlError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // TOML parser ensures that the returned vec is not empty.
        toml_edit::Key::parse(s).map(ConfigNamePathBuf)
    }
}

impl AsRef<[toml_edit::Key]> for ConfigNamePathBuf {
    fn as_ref(&self) -> &[toml_edit::Key] {
        &self.0
    }
}

impl Display for ConfigNamePathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut components = self.0.iter().fuse();
        if let Some(key) = components.next() {
            write!(f, "{key}")?;
        }
        components.try_for_each(|key| write!(f, ".{key}"))
    }
}

/// Value that can be converted to a dotted config name path.
///
/// This is an abstraction to specify a config name path in either a string or a
/// parsed form. It's similar to `Into<T>`, but the output type `T` is
/// constrained by the source type.
pub trait ToConfigNamePath: Sized {
    /// Path type to be converted from `Self`.
    type Output: Borrow<ConfigNamePathBuf> + Into<ConfigNamePathBuf>;

    /// Converts this object into a dotted config name path.
    fn into_name_path(self) -> Self::Output;
}

impl ToConfigNamePath for ConfigNamePathBuf {
    type Output = Self;

    fn into_name_path(self) -> Self::Output {
        self
    }
}

impl ToConfigNamePath for &ConfigNamePathBuf {
    type Output = Self;

    fn into_name_path(self) -> Self::Output {
        self
    }
}

impl ToConfigNamePath for &'static str {
    // This can be changed to ConfigNamePathStr(str) if allocation cost matters.
    type Output = ConfigNamePathBuf;

    /// Parses this string into a dotted config name path.
    ///
    /// The string must be a valid TOML dotted key. A static str is required to
    /// prevent API misuse.
    fn into_name_path(self) -> Self::Output {
        self.parse()
            .expect("valid TOML dotted key must be provided")
    }
}

impl<const N: usize> ToConfigNamePath for [&str; N] {
    type Output = ConfigNamePathBuf;

    fn into_name_path(self) -> Self::Output {
        self.into_iter().collect()
    }
}

impl<const N: usize> ToConfigNamePath for &[&str; N] {
    type Output = ConfigNamePathBuf;

    fn into_name_path(self) -> Self::Output {
        self.as_slice().into_name_path()
    }
}

impl ToConfigNamePath for &[&str] {
    type Output = ConfigNamePathBuf;

    fn into_name_path(self) -> Self::Output {
        self.iter().copied().collect()
    }
}

/// Source of configuration variables in order of precedence.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ConfigSource {
    /// Default values (which has the lowest precedence.)
    Default,
    /// Base environment variables.
    EnvBase,
    /// User configuration files.
    User,
    /// Repo configuration files.
    Repo,
    /// Workspace configuration files.
    Workspace,
    /// Override environment variables.
    EnvOverrides,
    /// Command-line arguments (which has the highest precedence.)
    CommandArg,
}

impl Display for ConfigSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ConfigSource::*;
        let c = match self {
            Default => "default",
            User => "user",
            Repo => "repo",
            Workspace => "workspace",
            CommandArg => "cli",
            EnvBase | EnvOverrides => "env",
        };
        write!(f, "{c}")
    }
}

/// Set of configuration variables with source information.
#[derive(Clone, Debug)]
pub struct ConfigLayer {
    /// Source type of this layer.
    pub source: ConfigSource,
    /// Source file path of this layer if any.
    pub path: Option<PathBuf>,
    /// Configuration variables.
    pub data: DocumentMut,
}

impl ConfigLayer {
    /// Creates new layer with empty data.
    pub fn empty(source: ConfigSource) -> Self {
        Self::with_data(source, DocumentMut::new())
    }

    /// Creates new layer with the configuration variables `data`.
    pub fn with_data(source: ConfigSource, data: DocumentMut) -> Self {
        Self {
            source,
            path: None,
            data,
        }
    }

    /// Parses TOML document `text` into new layer.
    pub fn parse(source: ConfigSource, text: &str) -> Result<Self, ConfigLoadError> {
        let data = Document::parse(text).map_err(|error| ConfigLoadError::Parse {
            error: Box::new(error),
            source_path: None,
        })?;
        Ok(Self::with_data(source, data.into_mut()))
    }

    /// Loads TOML file from the specified `path`.
    pub fn load_from_file(source: ConfigSource, path: PathBuf) -> Result<Self, ConfigLoadError> {
        let text = fs::read_to_string(&path)
            .context(&path)
            .map_err(ConfigLoadError::Read)?;
        let data = Document::parse(text).map_err(|error| ConfigLoadError::Parse {
            error: Box::new(error),
            source_path: Some(path.clone()),
        })?;
        Ok(Self {
            source,
            path: Some(path),
            data: data.into_mut(),
        })
    }

    fn load_from_dir(source: ConfigSource, path: &Path) -> Result<Vec<Self>, ConfigLoadError> {
        // TODO: Walk the directory recursively?
        let mut file_paths: Vec<_> = path
            .read_dir()
            .and_then(|dir_entries| {
                dir_entries
                    .map(|entry| Ok(entry?.path()))
                    .filter_ok(|path| path.is_file() && path.extension() == Some("toml".as_ref()))
                    .try_collect()
            })
            .context(path)
            .map_err(ConfigLoadError::Read)?;
        file_paths.sort_unstable();
        file_paths
            .into_iter()
            .map(|path| Self::load_from_file(source, path))
            .try_collect()
    }

    /// Returns true if the table has no configuration variables.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    // Add .get_value(name) if needed. look_up_*() are low-level API.

    /// Looks up sub table by the `name` path. Returns `Some(table)` if a table
    /// was found at the path. Returns `Err(item)` if middle or leaf node wasn't
    /// a table.
    pub fn look_up_table(
        &self,
        name: impl ToConfigNamePath,
    ) -> Result<Option<&ConfigTableLike<'_>>, &ConfigItem> {
        match self.look_up_item(name) {
            Ok(Some(item)) => match item.as_table_like() {
                Some(table) => Ok(Some(table)),
                None => Err(item),
            },
            Ok(None) => Ok(None),
            Err(item) => Err(item),
        }
    }

    /// Looks up item by the `name` path. Returns `Some(item)` if an item
    /// found at the path. Returns `Err(item)` if middle node wasn't a table.
    pub fn look_up_item(
        &self,
        name: impl ToConfigNamePath,
    ) -> Result<Option<&ConfigItem>, &ConfigItem> {
        look_up_item(self.data.as_item(), name.into_name_path().borrow())
    }

    /// Sets `new_value` to the `name` path. Returns old value if any.
    ///
    /// This function errors out if attempted to overwrite a non-table middle
    /// node or a leaf non-inline table. An inline table can be overwritten
    /// because it's syntactically a value.
    pub fn set_value(
        &mut self,
        name: impl ToConfigNamePath,
        new_value: impl Into<ConfigValue>,
    ) -> Result<Option<ConfigValue>, ConfigUpdateError> {
        let would_overwrite_table = |name| ConfigUpdateError::WouldOverwriteValue { name };
        let name = name.into_name_path();
        let name = name.borrow();
        let (leaf_key, table_keys) = name
            .0
            .split_last()
            .ok_or_else(|| would_overwrite_table(name.to_string()))?;
        let parent_table = ensure_table(self.data.as_table_mut(), table_keys)
            .map_err(|keys| would_overwrite_table(keys.join(".")))?;
        match parent_table.entry_format(leaf_key) {
            toml_edit::Entry::Occupied(mut entry) => {
                if !entry.get().is_value() {
                    return Err(ConfigUpdateError::WouldOverwriteTable {
                        name: name.to_string(),
                    });
                }
                let old_item = entry.insert(toml_edit::value(new_value));
                Ok(Some(old_item.into_value().unwrap()))
            }
            toml_edit::Entry::Vacant(entry) => {
                entry.insert(toml_edit::value(new_value));
                // Reset whitespace formatting (i.e. insert space before '=')
                let mut new_key = parent_table.key_mut(leaf_key).unwrap();
                new_key.leaf_decor_mut().clear();
                Ok(None)
            }
        }
    }

    /// Deletes value specified by the `name` path. Returns old value if any.
    ///
    /// Returns `Ok(None)` if middle node wasn't a table or a value wasn't
    /// found. Returns `Err` if attempted to delete a non-inline table. An
    /// inline table can be deleted because it's syntactically a value.
    pub fn delete_value(
        &mut self,
        name: impl ToConfigNamePath,
    ) -> Result<Option<ConfigValue>, ConfigUpdateError> {
        let would_delete_table = |name| ConfigUpdateError::WouldDeleteTable { name };
        let name = name.into_name_path();
        let name = name.borrow();
        let mut keys = name.components();
        let leaf_key = keys
            .next_back()
            .ok_or_else(|| would_delete_table(name.to_string()))?;
        let Some(parent_table) = keys.try_fold(
            self.data.as_table_mut() as &mut ConfigTableLike,
            |table, key| table.get_mut(key)?.as_table_like_mut(),
        ) else {
            return Ok(None);
        };
        match parent_table.entry(leaf_key) {
            toml_edit::Entry::Occupied(entry) => {
                if !entry.get().is_value() {
                    return Err(would_delete_table(name.to_string()));
                }
                let old_item = entry.remove();
                Ok(Some(old_item.into_value().unwrap()))
            }
            toml_edit::Entry::Vacant(_) => Ok(None),
        }
    }

    /// Inserts tables down to the `name` path. Returns mutable reference to the
    /// leaf table.
    ///
    /// This function errors out if attempted to overwrite a non-table node. In
    /// file-system analogy, this is equivalent to `std::fs::create_dir_all()`.
    pub fn ensure_table(
        &mut self,
        name: impl ToConfigNamePath,
    ) -> Result<&mut ConfigTableLike<'_>, ConfigUpdateError> {
        let would_overwrite_table = |name| ConfigUpdateError::WouldOverwriteValue { name };
        let name = name.into_name_path();
        let name = name.borrow();
        ensure_table(self.data.as_table_mut(), &name.0)
            .map_err(|keys| would_overwrite_table(keys.join(".")))
    }
}

/// Looks up item from the `root_item`. Returns `Some(item)` if an item found at
/// the path. Returns `Err(item)` if middle node wasn't a table.
fn look_up_item<'a>(
    root_item: &'a ConfigItem,
    name: &ConfigNamePathBuf,
) -> Result<Option<&'a ConfigItem>, &'a ConfigItem> {
    let mut cur_item = root_item;
    for key in name.components() {
        let Some(table) = cur_item.as_table_like() else {
            return Err(cur_item);
        };
        cur_item = match table.get(key) {
            Some(item) => item,
            None => return Ok(None),
        };
    }
    Ok(Some(cur_item))
}

/// Inserts tables recursively. Returns `Err(keys)` if middle node exists at the
/// prefix name `keys` and wasn't a table.
fn ensure_table<'a, 'b>(
    root_table: &'a mut ConfigTableLike<'a>,
    keys: &'b [toml_edit::Key],
) -> Result<&'a mut ConfigTableLike<'a>, &'b [toml_edit::Key]> {
    keys.iter()
        .enumerate()
        .try_fold(root_table, |table, (i, key)| {
            let sub_item = table.entry_format(key).or_insert_with(new_implicit_table);
            sub_item.as_table_like_mut().ok_or(&keys[..=i])
        })
}

fn new_implicit_table() -> ConfigItem {
    let mut table = ConfigTable::new();
    table.set_implicit(true);
    ConfigItem::Table(table)
}

/// Wrapper for file-based [`ConfigLayer`], providing convenient methods for
/// modification.
#[derive(Clone, Debug)]
pub struct ConfigFile {
    layer: Arc<ConfigLayer>,
}

impl ConfigFile {
    /// Loads TOML file from the specified `path` if exists. Returns an empty
    /// object if the file doesn't exist.
    pub fn load_or_empty(
        source: ConfigSource,
        path: impl Into<PathBuf>,
    ) -> Result<Self, ConfigLoadError> {
        let layer = match ConfigLayer::load_from_file(source, path.into()) {
            Ok(layer) => Arc::new(layer),
            Err(ConfigLoadError::Read(PathError {
                path,
                source: error,
            })) if error.kind() == io::ErrorKind::NotFound => {
                let mut data = DocumentMut::new();
                data.decor_mut()
                    .set_prefix("#:schema https://docs.jj-vcs.dev/latest/config-schema.json\n\n");
                let layer = ConfigLayer {
                    source,
                    path: Some(path),
                    data,
                };
                Arc::new(layer)
            }
            Err(err) => return Err(err),
        };
        Ok(Self { layer })
    }

    /// Wraps file-based [`ConfigLayer`] for modification. Returns `Err(layer)`
    /// if the source `path` is unknown.
    pub fn from_layer(layer: Arc<ConfigLayer>) -> Result<Self, Arc<ConfigLayer>> {
        if layer.path.is_some() {
            Ok(Self { layer })
        } else {
            Err(layer)
        }
    }

    /// Writes serialized data to the source file.
    pub fn save(&self) -> Result<(), ConfigFileSaveError> {
        fs::write(self.path(), self.layer.data.to_string())
            .context(self.path())
            .map_err(ConfigFileSaveError)
    }

    /// Source file path.
    pub fn path(&self) -> &Path {
        self.layer.path.as_ref().expect("path must be known")
    }

    /// Returns the underlying config layer.
    pub fn layer(&self) -> &Arc<ConfigLayer> {
        &self.layer
    }

    /// See [`ConfigLayer::set_value()`].
    pub fn set_value(
        &mut self,
        name: impl ToConfigNamePath,
        new_value: impl Into<ConfigValue>,
    ) -> Result<Option<ConfigValue>, ConfigUpdateError> {
        Arc::make_mut(&mut self.layer).set_value(name, new_value)
    }

    /// See [`ConfigLayer::delete_value()`].
    pub fn delete_value(
        &mut self,
        name: impl ToConfigNamePath,
    ) -> Result<Option<ConfigValue>, ConfigUpdateError> {
        Arc::make_mut(&mut self.layer).delete_value(name)
    }
}

/// Stack of configuration layers which can be merged as needed.
///
/// A [`StackedConfig`] is something like a read-only `overlayfs`. Tables and
/// values are directories and files respectively, and tables are merged across
/// layers. Tables and values can be addressed by [dotted name
/// paths](ToConfigNamePath).
///
/// There's no tombstone notation to remove items from the lower layers.
///
/// Beware that arrays of tables are no different than inline arrays. They are
/// values, so are never merged. This might be confusing because they would be
/// merged if two TOML documents are concatenated literally. Avoid using array
/// of tables syntax.
#[derive(Clone, Debug)]
pub struct StackedConfig {
    /// Layers sorted by `source` (the lowest precedence one first.)
    layers: Vec<Arc<ConfigLayer>>,
}

impl StackedConfig {
    /// Creates an empty stack of configuration layers.
    pub fn empty() -> Self {
        Self { layers: vec![] }
    }

    /// Creates a stack of configuration layers containing the default variables
    /// referred to by `jj-lib`.
    pub fn with_defaults() -> Self {
        Self {
            layers: DEFAULT_CONFIG_LAYERS.to_vec(),
        }
    }

    /// Loads config file from the specified `path`, inserts it at the position
    /// specified by `source`. The file should exist.
    pub fn load_file(
        &mut self,
        source: ConfigSource,
        path: impl Into<PathBuf>,
    ) -> Result<(), ConfigLoadError> {
        let layer = ConfigLayer::load_from_file(source, path.into())?;
        self.add_layer(layer);
        Ok(())
    }

    /// Loads config files from the specified directory `path`, inserts them at
    /// the position specified by `source`. The directory should exist.
    pub fn load_dir(
        &mut self,
        source: ConfigSource,
        path: impl AsRef<Path>,
    ) -> Result<(), ConfigLoadError> {
        let layers = ConfigLayer::load_from_dir(source, path.as_ref())?;
        self.extend_layers(layers);
        Ok(())
    }

    /// Inserts new layer at the position specified by `layer.source`.
    pub fn add_layer(&mut self, layer: impl Into<Arc<ConfigLayer>>) {
        let layer = layer.into();
        let index = self.insert_point(layer.source);
        self.layers.insert(index, layer);
    }

    /// Inserts multiple layers at the positions specified by `layer.source`.
    pub fn extend_layers<I>(&mut self, layers: I)
    where
        I: IntoIterator,
        I::Item: Into<Arc<ConfigLayer>>,
    {
        let layers = layers.into_iter().map(Into::into);
        for (source, chunk) in &layers.chunk_by(|layer| layer.source) {
            let index = self.insert_point(source);
            self.layers.splice(index..index, chunk);
        }
    }

    /// Removes layers of the specified `source`.
    pub fn remove_layers(&mut self, source: ConfigSource) {
        self.layers.drain(self.layer_range(source));
    }

    fn layer_range(&self, source: ConfigSource) -> Range<usize> {
        // Linear search since the size of Vec wouldn't be large.
        let start = self
            .layers
            .iter()
            .take_while(|layer| layer.source < source)
            .count();
        let count = self.layers[start..]
            .iter()
            .take_while(|layer| layer.source == source)
            .count();
        start..(start + count)
    }

    fn insert_point(&self, source: ConfigSource) -> usize {
        // Search from end since layers are usually added in order, and the size
        // of Vec wouldn't be large enough to do binary search.
        let skip = self
            .layers
            .iter()
            .rev()
            .take_while(|layer| layer.source > source)
            .count();
        self.layers.len() - skip
    }

    /// Layers sorted by precedence.
    pub fn layers(&self) -> &[Arc<ConfigLayer>] {
        &self.layers
    }

    /// Mutable references to layers sorted by precedence.
    pub fn layers_mut(&mut self) -> &mut [Arc<ConfigLayer>] {
        &mut self.layers
    }

    /// Layers of the specified `source`.
    pub fn layers_for(&self, source: ConfigSource) -> &[Arc<ConfigLayer>] {
        &self.layers[self.layer_range(source)]
    }

    /// Looks up value of the specified type `T` from all layers, merges sub
    /// fields as needed.
    pub fn get<'de, T: Deserialize<'de>>(
        &self,
        name: impl ToConfigNamePath,
    ) -> Result<T, ConfigGetError> {
        self.get_value_with(name, |value| T::deserialize(value.into_deserializer()))
    }

    /// Looks up value from all layers, merges sub fields as needed.
    pub fn get_value(&self, name: impl ToConfigNamePath) -> Result<ConfigValue, ConfigGetError> {
        self.get_value_with::<_, Infallible>(name, Ok)
    }

    /// Looks up value from all layers, merges sub fields as needed, then
    /// converts the value by using the given function.
    pub fn get_value_with<T, E: Into<Box<dyn std::error::Error + Send + Sync>>>(
        &self,
        name: impl ToConfigNamePath,
        convert: impl FnOnce(ConfigValue) -> Result<T, E>,
    ) -> Result<T, ConfigGetError> {
        self.get_item_with(name, |item| {
            // Item variants other than Item::None can be converted to a Value,
            // and Item::None is not a valid TOML type. See also the following
            // thread: https://github.com/toml-rs/toml/issues/299
            let value = item
                .into_value()
                .expect("Item::None should not exist in loaded tables");
            convert(value)
        })
    }

    /// Looks up sub table from all layers, merges fields as needed.
    ///
    /// Use `table_keys(prefix)` and `get([prefix, key])` instead if table
    /// values have to be converted to non-generic value type.
    pub fn get_table(&self, name: impl ToConfigNamePath) -> Result<ConfigTable, ConfigGetError> {
        self.get_item_with(name, |item| {
            item.into_table()
                .map_err(|item| format!("Expected a table, but is {}", item.type_name()))
        })
    }

    fn get_item_with<T, E: Into<Box<dyn std::error::Error + Send + Sync>>>(
        &self,
        name: impl ToConfigNamePath,
        convert: impl FnOnce(ConfigItem) -> Result<T, E>,
    ) -> Result<T, ConfigGetError> {
        let name = name.into_name_path();
        let name = name.borrow();
        let (item, layer_index) =
            get_merged_item(&self.layers, name).ok_or_else(|| ConfigGetError::NotFound {
                name: name.to_string(),
            })?;
        // If the value is a table, the error might come from lower layers. We
        // cannot report precise source information in that case. However,
        // toml_edit captures dotted keys in the error object. If the keys field
        // were public, we can look up the source information. This is probably
        // simpler than reimplementing Deserializer.
        convert(item).map_err(|err| ConfigGetError::Type {
            name: name.to_string(),
            error: err.into(),
            source_path: self.layers[layer_index].path.clone(),
        })
    }

    /// Returns iterator over sub table keys in order of layer precedence.
    /// Duplicated keys are omitted.
    pub fn table_keys(&self, name: impl ToConfigNamePath) -> impl Iterator<Item = &str> {
        let name = name.into_name_path();
        let name = name.borrow();
        let to_merge = get_tables_to_merge(&self.layers, name);
        to_merge
            .into_iter()
            .rev()
            .flat_map(|table| table.iter().map(|(k, _)| k))
            .unique()
    }
}

/// Looks up item from `layers`, merges sub fields as needed. Returns a merged
/// item and the uppermost layer index where the item was found.
fn get_merged_item(
    layers: &[Arc<ConfigLayer>],
    name: &ConfigNamePathBuf,
) -> Option<(ConfigItem, usize)> {
    let mut to_merge = Vec::new();
    for (index, layer) in layers.iter().enumerate().rev() {
        let item = match layer.look_up_item(name) {
            Ok(Some(item)) => item,
            Ok(None) => continue, // parent is a table, but no value found
            Err(_) => break,      // parent is not a table, shadows lower layers
        };
        if item.is_table_like() {
            to_merge.push((item, index));
        } else if to_merge.is_empty() {
            return Some((item.clone(), index)); // no need to allocate vec
        } else {
            break; // shadows lower layers
        }
    }

    // Simply merge tables from the bottom layer. Upper items should override
    // the lower items (including their children) no matter if the upper items
    // are shadowed by the other upper items.
    let (item, mut top_index) = to_merge.pop()?;
    let mut merged = item.clone();
    for (item, index) in to_merge.into_iter().rev() {
        merge_items(&mut merged, item);
        top_index = index;
    }
    Some((merged, top_index))
}

/// Looks up tables to be merged from `layers`, returns in reverse order.
fn get_tables_to_merge<'a>(
    layers: &'a [Arc<ConfigLayer>],
    name: &ConfigNamePathBuf,
) -> Vec<&'a ConfigTableLike<'a>> {
    let mut to_merge = Vec::new();
    for layer in layers.iter().rev() {
        match layer.look_up_table(name) {
            Ok(Some(table)) => to_merge.push(table),
            Ok(None) => {}   // parent is a table, but no value found
            Err(_) => break, // parent/leaf is not a table, shadows lower layers
        }
    }
    to_merge
}

/// Merges `upper_item` fields into `lower_item` recursively.
fn merge_items(lower_item: &mut ConfigItem, upper_item: &ConfigItem) {
    let (Some(lower_table), Some(upper_table)) =
        (lower_item.as_table_like_mut(), upper_item.as_table_like())
    else {
        // Not a table, the upper item wins.
        *lower_item = upper_item.clone();
        return;
    };
    for (key, upper) in upper_table.iter() {
        match lower_table.entry(key) {
            toml_edit::Entry::Occupied(entry) => {
                merge_items(entry.into_mut(), upper);
            }
            toml_edit::Entry::Vacant(entry) => {
                entry.insert(upper.clone());
            }
        };
    }
}

static DEFAULT_CONFIG_LAYERS: LazyLock<[Arc<ConfigLayer>; 1]> = LazyLock::new(|| {
    let parse = |text: &str| Arc::new(ConfigLayer::parse(ConfigSource::Default, text).unwrap());
    [parse(include_str!("config/misc.toml"))]
});

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_config_layer_set_value() {
        let mut layer = ConfigLayer::empty(ConfigSource::User);
        // Cannot overwrite the root table
        assert_matches!(
            layer.set_value(ConfigNamePathBuf::root(), 0),
            Err(ConfigUpdateError::WouldOverwriteValue { name }) if name.is_empty()
        );

        // Insert some values
        layer.set_value("foo", 1).unwrap();
        layer.set_value("bar.baz.blah", "2").unwrap();
        layer
            .set_value("bar.qux", ConfigValue::from_iter([("inline", "table")]))
            .unwrap();
        layer
            .set_value("bar.to-update", ConfigValue::from_iter([("some", true)]))
            .unwrap();
        insta::assert_snapshot!(layer.data, @r#"
        foo = 1

        [bar]
        qux = { inline = "table" }
        to-update = { some = true }

        [bar.baz]
        blah = "2"
        "#);

        // Can overwrite value
        layer
            .set_value("foo", ConfigValue::from_iter(["new", "foo"]))
            .unwrap();
        // Can overwrite inline table
        layer.set_value("bar.qux", "new bar.qux").unwrap();
        // Can add value to inline table
        layer
            .set_value(
                "bar.to-update.new",
                ConfigValue::from_iter([("table", "value")]),
            )
            .unwrap();
        // Cannot overwrite table
        assert_matches!(
            layer.set_value("bar", 0),
            Err(ConfigUpdateError::WouldOverwriteTable { name }) if name == "bar"
        );
        // Cannot overwrite value by table
        assert_matches!(
            layer.set_value("bar.baz.blah.blah", 0),
            Err(ConfigUpdateError::WouldOverwriteValue { name }) if name == "bar.baz.blah"
        );
        insta::assert_snapshot!(layer.data, @r#"
        foo = ["new", "foo"]

        [bar]
        qux = "new bar.qux"
        to-update = { some = true, new = { table = "value" } }

        [bar.baz]
        blah = "2"
        "#);
    }

    #[test]
    fn test_config_layer_set_value_formatting() {
        let mut layer = ConfigLayer::empty(ConfigSource::User);
        // Quoting style should be preserved on insertion
        layer
            .set_value(
                "'foo' . bar . 'baz'",
                ConfigValue::from_str("'value'").unwrap(),
            )
            .unwrap();
        insta::assert_snapshot!(layer.data, @r"
        ['foo' . bar]
        'baz' = 'value'
        ");

        // Style of existing keys isn't updated
        layer.set_value("foo.bar.baz", "new value").unwrap();
        layer.set_value("foo.'bar'.blah", 0).unwrap();
        insta::assert_snapshot!(layer.data, @r#"
        ['foo' . bar]
        'baz' = "new value"
        blah = 0
        "#);
    }

    #[test]
    fn test_config_layer_set_value_inline_table() {
        let mut layer = ConfigLayer::empty(ConfigSource::User);
        layer
            .set_value("a", ConfigValue::from_iter([("b", "a.b")]))
            .unwrap();
        insta::assert_snapshot!(layer.data, @r#"a = { b = "a.b" }"#);

        // Should create nested inline tables
        layer.set_value("a.c.d", "a.c.d").unwrap();
        insta::assert_snapshot!(layer.data, @r#"a = { b = "a.b", c.d = "a.c.d" }"#);
    }

    #[test]
    fn test_config_layer_delete_value() {
        let mut layer = ConfigLayer::empty(ConfigSource::User);
        // Cannot delete the root table
        assert_matches!(
            layer.delete_value(ConfigNamePathBuf::root()),
            Err(ConfigUpdateError::WouldDeleteTable { name }) if name.is_empty()
        );

        // Insert some values
        layer.set_value("foo", 1).unwrap();
        layer.set_value("bar.baz.blah", "2").unwrap();
        layer
            .set_value("bar.qux", ConfigValue::from_iter([("inline", "table")]))
            .unwrap();
        layer
            .set_value("bar.to-update", ConfigValue::from_iter([("some", true)]))
            .unwrap();
        insta::assert_snapshot!(layer.data, @r#"
        foo = 1

        [bar]
        qux = { inline = "table" }
        to-update = { some = true }

        [bar.baz]
        blah = "2"
        "#);

        // Can delete value
        let old_value = layer.delete_value("foo").unwrap();
        assert_eq!(old_value.and_then(|v| v.as_integer()), Some(1));
        // Can delete inline table
        let old_value = layer.delete_value("bar.qux").unwrap();
        assert!(old_value.is_some_and(|v| v.is_inline_table()));
        // Can delete inner value from inline table
        let old_value = layer.delete_value("bar.to-update.some").unwrap();
        assert_eq!(old_value.and_then(|v| v.as_bool()), Some(true));
        // Cannot delete table
        assert_matches!(
            layer.delete_value("bar"),
            Err(ConfigUpdateError::WouldDeleteTable { name }) if name == "bar"
        );
        // Deleting a non-table child isn't an error because the value doesn't
        // exist
        assert_matches!(layer.delete_value("bar.baz.blah.blah"), Ok(None));
        insta::assert_snapshot!(layer.data, @r#"
        [bar]
        to-update = {}

        [bar.baz]
        blah = "2"
        "#);
    }

    #[test]
    fn test_stacked_config_layer_order() {
        let empty_data = || DocumentMut::new();
        let layer_sources = |config: &StackedConfig| {
            config
                .layers()
                .iter()
                .map(|layer| layer.source)
                .collect_vec()
        };

        // Insert in reverse order
        let mut config = StackedConfig::empty();
        config.add_layer(ConfigLayer::with_data(ConfigSource::Repo, empty_data()));
        config.add_layer(ConfigLayer::with_data(ConfigSource::User, empty_data()));
        config.add_layer(ConfigLayer::with_data(ConfigSource::Default, empty_data()));
        assert_eq!(
            layer_sources(&config),
            vec![
                ConfigSource::Default,
                ConfigSource::User,
                ConfigSource::Repo,
            ]
        );

        // Insert some more
        config.add_layer(ConfigLayer::with_data(
            ConfigSource::CommandArg,
            empty_data(),
        ));
        config.add_layer(ConfigLayer::with_data(ConfigSource::EnvBase, empty_data()));
        config.add_layer(ConfigLayer::with_data(ConfigSource::User, empty_data()));
        assert_eq!(
            layer_sources(&config),
            vec![
                ConfigSource::Default,
                ConfigSource::EnvBase,
                ConfigSource::User,
                ConfigSource::User,
                ConfigSource::Repo,
                ConfigSource::CommandArg,
            ]
        );

        // Remove last, first, middle
        config.remove_layers(ConfigSource::CommandArg);
        config.remove_layers(ConfigSource::Default);
        config.remove_layers(ConfigSource::User);
        assert_eq!(
            layer_sources(&config),
            vec![ConfigSource::EnvBase, ConfigSource::Repo]
        );

        // Remove unknown
        config.remove_layers(ConfigSource::Default);
        config.remove_layers(ConfigSource::EnvOverrides);
        assert_eq!(
            layer_sources(&config),
            vec![ConfigSource::EnvBase, ConfigSource::Repo]
        );

        // Insert multiple
        config.extend_layers([
            ConfigLayer::with_data(ConfigSource::Repo, empty_data()),
            ConfigLayer::with_data(ConfigSource::Repo, empty_data()),
            ConfigLayer::with_data(ConfigSource::User, empty_data()),
        ]);
        assert_eq!(
            layer_sources(&config),
            vec![
                ConfigSource::EnvBase,
                ConfigSource::User,
                ConfigSource::Repo,
                ConfigSource::Repo,
                ConfigSource::Repo,
            ]
        );

        // Remove remainders
        config.remove_layers(ConfigSource::EnvBase);
        config.remove_layers(ConfigSource::User);
        config.remove_layers(ConfigSource::Repo);
        assert_eq!(layer_sources(&config), vec![]);
    }

    fn new_user_layer(text: &str) -> ConfigLayer {
        ConfigLayer::parse(ConfigSource::User, text).unwrap()
    }

    #[test]
    fn test_stacked_config_get_simple_value() {
        let mut config = StackedConfig::empty();
        config.add_layer(new_user_layer(indoc! {"
            a.b.c = 'a.b.c #0'
        "}));
        config.add_layer(new_user_layer(indoc! {"
            a.d = ['a.d #1']
        "}));

        assert_eq!(config.get::<String>("a.b.c").unwrap(), "a.b.c #0");

        assert_eq!(
            config.get::<Vec<String>>("a.d").unwrap(),
            vec!["a.d #1".to_owned()]
        );

        // Table "a.b" exists, but key doesn't
        assert_matches!(
            config.get::<String>("a.b.missing"),
            Err(ConfigGetError::NotFound { name }) if name == "a.b.missing"
        );

        // Node "a.b.c" is not a table
        assert_matches!(
            config.get::<String>("a.b.c.d"),
            Err(ConfigGetError::NotFound { name }) if name == "a.b.c.d"
        );

        // Type error
        assert_matches!(
            config.get::<String>("a.b"),
            Err(ConfigGetError::Type { name, .. }) if name == "a.b"
        );
    }

    #[test]
    fn test_stacked_config_get_table_as_value() {
        let mut config = StackedConfig::empty();
        config.add_layer(new_user_layer(indoc! {"
            a.b = { c = 'a.b.c #0' }
        "}));
        config.add_layer(new_user_layer(indoc! {"
            a.d = ['a.d #1']
        "}));

        // Table can be converted to a value (so it can be deserialized to a
        // structured value.)
        insta::assert_snapshot!(
            config.get_value("a").unwrap(),
            @"{ b = { c = 'a.b.c #0' }, d = ['a.d #1'] }");
    }

    #[test]
    fn test_stacked_config_get_inline_table() {
        let mut config = StackedConfig::empty();
        config.add_layer(new_user_layer(indoc! {"
            a.b = { c = 'a.b.c #0' }
        "}));
        config.add_layer(new_user_layer(indoc! {"
            a.b = { d = 'a.b.d #1' }
        "}));

        // Inline tables are merged
        insta::assert_snapshot!(
            config.get_value("a.b").unwrap(),
            @" { c = 'a.b.c #0' , d = 'a.b.d #1' }");
    }

    #[test]
    fn test_stacked_config_get_inline_non_inline_table() {
        let mut config = StackedConfig::empty();
        config.add_layer(new_user_layer(indoc! {"
            a.b = { c = 'a.b.c #0' }
        "}));
        config.add_layer(new_user_layer(indoc! {"
            a.b.d = 'a.b.d #1'
        "}));

        insta::assert_snapshot!(
            config.get_value("a.b").unwrap(),
            @" { c = 'a.b.c #0' , d = 'a.b.d #1'}");
        insta::assert_snapshot!(
            config.get_table("a").unwrap(),
            @"b = { c = 'a.b.c #0' , d = 'a.b.d #1'}");
    }

    #[test]
    fn test_stacked_config_get_value_shadowing_table() {
        let mut config = StackedConfig::empty();
        config.add_layer(new_user_layer(indoc! {"
            a.b.c = 'a.b.c #0'
        "}));
        // a.b.c is shadowed by a.b
        config.add_layer(new_user_layer(indoc! {"
            a.b = 'a.b #1'
        "}));

        assert_eq!(config.get::<String>("a.b").unwrap(), "a.b #1");

        assert_matches!(
            config.get::<String>("a.b.c"),
            Err(ConfigGetError::NotFound { name }) if name == "a.b.c"
        );
    }

    #[test]
    fn test_stacked_config_get_table_shadowing_table() {
        let mut config = StackedConfig::empty();
        config.add_layer(new_user_layer(indoc! {"
            a.b = 'a.b #0'
        "}));
        // a.b is shadowed by a.b.c
        config.add_layer(new_user_layer(indoc! {"
            a.b.c = 'a.b.c #1'
        "}));
        insta::assert_snapshot!(config.get_table("a.b").unwrap(), @"c = 'a.b.c #1'");
    }

    #[test]
    fn test_stacked_config_get_merged_table() {
        let mut config = StackedConfig::empty();
        config.add_layer(new_user_layer(indoc! {"
            a.a.a = 'a.a.a #0'
            a.a.b = 'a.a.b #0'
            a.b = 'a.b #0'
        "}));
        config.add_layer(new_user_layer(indoc! {"
            a.a.b = 'a.a.b #1'
            a.a.c = 'a.a.c #1'
            a.c = 'a.c #1'
        "}));
        insta::assert_snapshot!(config.get_table("a").unwrap(), @r"
        a.a = 'a.a.a #0'
        a.b = 'a.a.b #1'
        a.c = 'a.a.c #1'
        b = 'a.b #0'
        c = 'a.c #1'
        ");
        assert_eq!(config.table_keys("a").collect_vec(), vec!["a", "b", "c"]);
        assert_eq!(config.table_keys("a.a").collect_vec(), vec!["a", "b", "c"]);
        assert_eq!(config.table_keys("a.b").collect_vec(), vec![""; 0]);
        assert_eq!(config.table_keys("a.missing").collect_vec(), vec![""; 0]);
    }

    #[test]
    fn test_stacked_config_get_merged_table_shadowed_top() {
        let mut config = StackedConfig::empty();
        config.add_layer(new_user_layer(indoc! {"
            a.a.a = 'a.a.a #0'
            a.b = 'a.b #0'
        "}));
        // a.a.a and a.b are shadowed by a
        config.add_layer(new_user_layer(indoc! {"
            a = 'a #1'
        "}));
        // a is shadowed by a.a.b
        config.add_layer(new_user_layer(indoc! {"
            a.a.b = 'a.a.b #2'
        "}));
        insta::assert_snapshot!(config.get_table("a").unwrap(), @"a.b = 'a.a.b #2'");
        assert_eq!(config.table_keys("a").collect_vec(), vec!["a"]);
        assert_eq!(config.table_keys("a.a").collect_vec(), vec!["b"]);
    }

    #[test]
    fn test_stacked_config_get_merged_table_shadowed_child() {
        let mut config = StackedConfig::empty();
        config.add_layer(new_user_layer(indoc! {"
            a.a.a = 'a.a.a #0'
            a.b = 'a.b #0'
        "}));
        // a.a.a is shadowed by a.a
        config.add_layer(new_user_layer(indoc! {"
            a.a = 'a.a #1'
        "}));
        // a.a is shadowed by a.a.b
        config.add_layer(new_user_layer(indoc! {"
            a.a.b = 'a.a.b #2'
        "}));
        insta::assert_snapshot!(config.get_table("a").unwrap(), @r"
        a.b = 'a.a.b #2'
        b = 'a.b #0'
        ");
        assert_eq!(config.table_keys("a").collect_vec(), vec!["a", "b"]);
        assert_eq!(config.table_keys("a.a").collect_vec(), vec!["b"]);
    }

    #[test]
    fn test_stacked_config_get_merged_table_shadowed_parent() {
        let mut config = StackedConfig::empty();
        config.add_layer(new_user_layer(indoc! {"
            a.a.a = 'a.a.a #0'
        "}));
        // a.a.a is shadowed by a
        config.add_layer(new_user_layer(indoc! {"
            a = 'a #1'
        "}));
        // a is shadowed by a.a.b
        config.add_layer(new_user_layer(indoc! {"
            a.a.b = 'a.a.b #2'
        "}));
        // a is not under a.a, but it should still shadow lower layers
        insta::assert_snapshot!(config.get_table("a.a").unwrap(), @"b = 'a.a.b #2'");
        assert_eq!(config.table_keys("a.a").collect_vec(), vec!["b"]);
    }
}
