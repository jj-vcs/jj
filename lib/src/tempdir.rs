// Copyright 2024 The Jujutsu Authors
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

//! Provides UTF-8 `tempfile::TempDir` counterpart.

use std::io;
use std::ops;

use camino::FromPathBufError;
use camino::Utf8Path;
use camino::Utf8PathBuf;
use tempfile::NamedTempFile;
use tempfile::PersistError;
use tempfile::TempDir;

/// UTF-8 valid `tempfile::env::temp_dir` counterpart
pub fn temp_dir() -> io::Result<Utf8PathBuf> {
    tempfile::env::temp_dir()
        .try_into()
        .map_err(|err: FromPathBufError| err.into_io_error())
}

/// UTF-8 valid `tempfile::NamedTempFile` counterpart
#[derive(Debug)]
pub struct Utf8NamedTempFile<F = std::fs::File> {
    path: Utf8PathBuf,
    temp: NamedTempFile<F>,
}

impl<F> Utf8NamedTempFile<F> {
    /// Access underlying `Utf8Path`
    pub fn path(&self) -> &Utf8Path {
        self.path.as_path()
    }

    /// Access underlying `tempfile::NamedTempFile`
    pub fn temp(&self) -> &NamedTempFile<F> {
        &self.temp
    }

    /// UTF-8 counterpart of `tempfile::NamedTempFile::keep`
    pub fn keep(self) -> Result<(F, Utf8PathBuf), PersistError<F>> {
        self.temp.keep().map(|(f, _path)| (f, self.path))
    }
}

impl<F> TryFrom<NamedTempFile<F>> for Utf8NamedTempFile<F> {
    type Error = io::Error;

    fn try_from(temp: NamedTempFile<F>) -> Result<Self, Self::Error> {
        temp.path()
            .to_path_buf()
            .try_into()
            .map(|path| Self { path, temp })
            .map_err(|err| err.into_io_error())
    }
}

impl AsRef<Utf8Path> for Utf8NamedTempFile {
    fn as_ref(&self) -> &Utf8Path {
        &self.path
    }
}

impl AsRef<NamedTempFile> for Utf8NamedTempFile {
    fn as_ref(&self) -> &NamedTempFile {
        &self.temp
    }
}

impl AsMut<NamedTempFile> for Utf8NamedTempFile {
    fn as_mut(&mut self) -> &mut NamedTempFile {
        &mut self.temp
    }
}

impl ops::Deref for Utf8NamedTempFile {
    type Target = NamedTempFile;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl ops::DerefMut for Utf8NamedTempFile {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_mut()
    }
}

/// UTF-8 valid `tempfile::TempDir` counterpart
#[derive(Debug)]
pub struct Utf8TempDir {
    path: Utf8PathBuf,
    temp: TempDir,
}

impl Utf8TempDir {
    /// Creates new `Utf8TempDir` object
    pub fn new() -> io::Result<Self> {
        let temp = TempDir::new()?;
        temp.path()
            .to_path_buf()
            .try_into()
            .map(|dir| Self { path: dir, temp })
            .map_err(|err| err.into_io_error())
    }

    /// Access underlying `Utf8Path`
    pub fn path(&self) -> &Utf8Path {
        self.path.as_path()
    }

    /// Access underlying `tempfile::Tempdir`
    pub fn temp(&self) -> &tempfile::TempDir {
        &self.temp
    }

    /// Creates new `Utf8TempDir` from `tempfile::TempDir` containing valid
    /// UTF-8 characters.
    ///
    /// Errors with the original `tempfile::TempDir` if it is not valid UTF-8.
    pub fn from_tempdir(temp: TempDir) -> Result<Self, TempDir> {
        if let Some(path) = Utf8Path::from_path(temp.path()) {
            let dir = path.to_path_buf();
            Ok(Self { path: dir, temp })
        } else {
            Err(temp)
        }
    }
}

impl TryFrom<TempDir> for Utf8TempDir {
    type Error = io::Error;

    fn try_from(temp: TempDir) -> Result<Self, Self::Error> {
        temp.path()
            .to_path_buf()
            .try_into()
            .map(|path| Self { path, temp })
            .map_err(|err| err.into_io_error())
    }
}

impl AsRef<Utf8Path> for Utf8TempDir {
    fn as_ref(&self) -> &Utf8Path {
        self.path()
    }
}

impl AsRef<TempDir> for Utf8TempDir {
    fn as_ref(&self) -> &tempfile::TempDir {
        self.temp()
    }
}

#[expect(clippy::disallowed_types)]
impl AsRef<std::path::Path> for Utf8TempDir {
    fn as_ref(&self) -> &std::path::Path {
        self.path().as_std_path()
    }
}

impl ops::Deref for Utf8TempDir {
    type Target = Utf8Path;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}
