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

use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::str;

use jj_lib::file_util;

use super::Template;
use super::TemplateFormatter;

/// A filesystem path that renders as an absolute path by default.
///
/// The default template and JSON forms use the absolute path so output doesn't
/// depend on the process working directory. Use [`FsPath::relative()`] or
/// [`FsPath::display()`] when showing paths to a user in command output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FsPath {
    absolute_path: PathBuf,
    cwd: PathBuf,
}

impl FsPath {
    /// Creates a filesystem path from an absolute path and current working
    /// directory.
    ///
    /// The current working directory is only used to format the path for
    /// display. The path's default rendered and serialized forms remain
    /// absolute.
    pub fn from_absolute_path(absolute_path: PathBuf, cwd: PathBuf) -> Self {
        Self { absolute_path, cwd }
    }

    /// Returns the absolute path used by default rendering and serialization.
    pub fn absolute(&self) -> &Path {
        &self.absolute_path
    }

    /// Returns the path relative to the current working directory.
    pub fn relative(&self) -> PathBuf {
        file_util::relative_path(&self.cwd, &self.absolute_path)
    }

    /// Returns the path formatted for display relative to the current working
    /// directory.
    pub fn display(&self) -> PathBuf {
        self.relative()
    }
}

impl serde::Serialize for FsPath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Keep JSON ergonomic for normal paths, but don't silently replace
        // non-UTF-8 path bytes with U+FFFD.
        let bytes =
            file_util::path_to_bytes(&self.absolute_path).map_err(serde::ser::Error::custom)?;
        match str::from_utf8(bytes) {
            Ok(path) => serializer.serialize_str(path),
            Err(_) => serializer.serialize_bytes(bytes),
        }
    }
}

impl Template for FsPath {
    fn format(&self, formatter: &mut TemplateFormatter) -> io::Result<()> {
        let bytes = file_util::path_to_bytes(&self.absolute_path).map_err(io::Error::other)?;
        formatter.as_mut().write_all(bytes)
    }
}

#[cfg(test)]
mod tests {
    use bstr::BString;

    use super::*;
    use crate::formatter::ColorFormatter;
    use crate::templater::format_property_error_inline;

    fn format_plain_text(path: &FsPath) -> BString {
        let mut output = Vec::new();
        {
            let mut formatter = ColorFormatter::new(&mut output, Vec::new().into(), false);
            let mut formatter =
                TemplateFormatter::new(&mut formatter, format_property_error_inline);
            path.format(&mut formatter).unwrap();
        }
        output.into()
    }

    #[test]
    fn test_format_absolute_path() {
        let cwd = std::env::current_dir().unwrap();
        let path = cwd.join("workspace");
        let fs_path = FsPath::from_absolute_path(path.clone(), cwd);

        assert_eq!(format_plain_text(&fs_path), path.display().to_string());
        assert_eq!(fs_path.absolute(), path);
    }

    #[test]
    fn test_display_path_is_relative_to_cwd() {
        let cwd = std::env::current_dir().unwrap();
        let path = cwd.join("workspace");
        let fs_path = FsPath::from_absolute_path(path, cwd);

        assert_eq!(fs_path.display(), PathBuf::from("workspace"));
        assert_eq!(fs_path.relative(), PathBuf::from("workspace"));
    }

    #[test]
    fn test_serialize_absolute_path() {
        let cwd = std::env::current_dir().unwrap();
        let path = cwd.join("workspace");
        let fs_path = FsPath::from_absolute_path(path.clone(), cwd);

        assert_eq!(
            serde_json::to_value(&fs_path).unwrap(),
            serde_json::Value::String(path.display().to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_format_non_utf8_path() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt as _;

        let path = PathBuf::from(OsString::from_vec(b"/repo/\x80workspace".to_vec()));
        let fs_path = FsPath::from_absolute_path(path, PathBuf::from("/repo"));

        assert_eq!(
            format_plain_text(&fs_path),
            BString::from(b"/repo/\x80workspace".as_slice())
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_serialize_non_utf8_path_as_bytes() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt as _;

        let path_bytes = b"/repo/\x80workspace";
        let path = PathBuf::from(OsString::from_vec(path_bytes.to_vec()));
        let fs_path = FsPath::from_absolute_path(path, PathBuf::from("/repo"));

        assert_eq!(
            serde_json::to_value(&fs_path).unwrap(),
            serde_json::Value::Array(
                path_bytes
                    .iter()
                    .map(|byte| serde_json::Value::Number((*byte).into()))
                    .collect()
            )
        );
    }
}
