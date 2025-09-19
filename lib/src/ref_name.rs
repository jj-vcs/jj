// Copyright 2025 The Jujutsu Authors
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

//! Name types for commit references.
//!
//! Name types can be constructed from a string:
//! ```
//! # use jj_lib::ref_name::*;
//! let _: RefNameBuf = "main".into();
//! let _: &RemoteName = "origin".as_ref();
//! ```
//!
//! However, they cannot be converted to other name types:
//! ```compile_fail
//! # use jj_lib::ref_name::*;
//! let _: RefNameBuf = RemoteName::new("origin").into();
//! ```
//! ```compile_fail
//! # use jj_lib::ref_name::*;
//! let _: &RemoteName = RefName::new("main").as_ref();
//! ```

use std::borrow::Borrow;
use std::fmt;
use std::fmt::Display;
use std::ops::Deref;

use ref_cast::RefCastCustom;
use ref_cast::ref_cast_custom;

use crate::content_hash::ContentHash;
use crate::revset;

/// Owned Git ref name in fully-qualified form (e.g. `refs/heads/main`.)
///
/// Use `.as_str()` or `.as_symbol()` for displaying. Other than that, this can
/// be considered an immutable `String`.
// Eq, Hash, and Ord must be compatible with GitRefName.
#[derive(Clone, ContentHash, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GitRefNameBuf(String);

/// Borrowed Git ref name in fully-qualified form (e.g. `refs/heads/main`.)
///
/// Use `.as_str()` or `.as_symbol()` for displaying. Other than that, this can
/// be considered an immutable `str`.
#[derive(ContentHash, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, RefCastCustom)]
#[repr(transparent)]
pub struct GitRefName(str);

/// Owned local (or local part of remote) bookmark or tag name.
///
/// Use `.as_str()` or `.as_symbol()` for displaying. Other than that, this can
/// be considered an immutable `String`.
// Eq, Hash, and Ord must be compatible with RefName.
#[derive(Clone, ContentHash, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RefNameBuf(String);

/// Borrowed local (or local part of remote) bookmark or tag name.
///
/// Use `.as_str()` or `.as_symbol()` for displaying. Other than that, this can
/// be considered an immutable `str`.
#[derive(ContentHash, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, RefCastCustom)]
#[repr(transparent)]
pub struct RefName(str);

/// Owned remote name.
///
/// Use `.as_str()` or `.as_symbol()` for displaying. Other than that, this can
/// be considered an immutable `String`.
// Eq, Hash, and Ord must be compatible with RemoteName.
#[derive(Clone, ContentHash, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RemoteNameBuf(String);

/// Borrowed remote name.
///
/// Use `.as_str()` or `.as_symbol()` for displaying. Other than that, this can
/// be considered an immutable `str`.
#[derive(ContentHash, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, RefCastCustom)]
#[repr(transparent)]
pub struct RemoteName(str);

/// Owned workspace name.
///
/// Use `.as_str()` or `.as_symbol()` for displaying. Other than that, this can
/// be considered an immutable `String`.
// Eq, Hash, and Ord must be compatible with WorkspaceName.
#[derive(Clone, ContentHash, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Serialize)]
#[serde(transparent)]
pub struct WorkspaceNameBuf(String);

/// Borrowed workspace name.
///
/// Use `.as_str()` or `.as_symbol()` for displaying. Other than that, this can
/// be considered an immutable `str`.
#[derive(
    ContentHash, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, RefCastCustom, serde::Serialize,
)]
#[serde(transparent)]
#[repr(transparent)]
pub struct WorkspaceName(str);

macro_rules! impl_partial_eq {
    ($borrowed_ty:ty, $lhs:ty, $rhs:ty) => {
        impl PartialEq<$rhs> for $lhs {
            fn eq(&self, other: &$rhs) -> bool {
                <$borrowed_ty as PartialEq>::eq(self, other)
            }
        }

        impl PartialEq<$lhs> for $rhs {
            fn eq(&self, other: &$lhs) -> bool {
                <$borrowed_ty as PartialEq>::eq(self, other)
            }
        }
    };
}

macro_rules! impl_partial_eq_str {
    ($borrowed_ty:ty, $lhs:ty, $rhs:ty) => {
        impl PartialEq<$rhs> for $lhs {
            fn eq(&self, other: &$rhs) -> bool {
                <$borrowed_ty as PartialEq>::eq(self, other.as_ref())
            }
        }

        impl PartialEq<$lhs> for $rhs {
            fn eq(&self, other: &$lhs) -> bool {
                <$borrowed_ty as PartialEq>::eq(self.as_ref(), other)
            }
        }
    };
}

macro_rules! impl_name_type {
    ($owned_ty:ident, $borrowed_ty:ident) => {
        impl $owned_ty {
            /// Consumes this and returns the underlying string.
            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl $borrowed_ty {
            /// Wraps string name.
            #[ref_cast_custom]
            pub const fn new(name: &str) -> &Self;

            /// Returns the underlying string.
            pub const fn as_str(&self) -> &str {
                &self.0
            }

            /// Converts to symbol for displaying.
            pub fn as_symbol(&self) -> &RefSymbol {
                RefSymbol::new(&self.0)
            }
        }

        // Owned type can be constructed from (weakly-typed) string:

        impl From<String> for $owned_ty {
            fn from(value: String) -> Self {
                $owned_ty(value)
            }
        }

        impl From<&String> for $owned_ty {
            fn from(value: &String) -> Self {
                $owned_ty(value.clone())
            }
        }

        impl From<&str> for $owned_ty {
            fn from(value: &str) -> Self {
                $owned_ty(value.to_owned())
            }
        }

        // Owned type can be constructed from borrowed type:

        impl From<&$owned_ty> for $owned_ty {
            fn from(value: &$owned_ty) -> Self {
                value.clone()
            }
        }

        impl From<&$borrowed_ty> for $owned_ty {
            fn from(value: &$borrowed_ty) -> Self {
                value.to_owned()
            }
        }

        // Borrowed type can be constructed from (weakly-typed) string:

        impl AsRef<$borrowed_ty> for String {
            fn as_ref(&self) -> &$borrowed_ty {
                $borrowed_ty::new(self)
            }
        }

        impl AsRef<$borrowed_ty> for str {
            fn as_ref(&self) -> &$borrowed_ty {
                $borrowed_ty::new(self)
            }
        }

        // Types can be converted to (weakly-typed) string:

        impl From<$owned_ty> for String {
            fn from(value: $owned_ty) -> Self {
                value.0
            }
        }

        impl From<&$owned_ty> for String {
            fn from(value: &$owned_ty) -> Self {
                value.0.clone()
            }
        }

        impl From<&$borrowed_ty> for String {
            fn from(value: &$borrowed_ty) -> Self {
                value.0.to_owned()
            }
        }

        impl AsRef<str> for $owned_ty {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl AsRef<str> for $borrowed_ty {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        // Types can be converted to borrowed type, and back to owned type:

        impl AsRef<$borrowed_ty> for $owned_ty {
            fn as_ref(&self) -> &$borrowed_ty {
                self
            }
        }

        impl AsRef<$borrowed_ty> for $borrowed_ty {
            fn as_ref(&self) -> &$borrowed_ty {
                self
            }
        }

        impl Borrow<$borrowed_ty> for $owned_ty {
            fn borrow(&self) -> &$borrowed_ty {
                self
            }
        }

        impl Deref for $owned_ty {
            type Target = $borrowed_ty;

            fn deref(&self) -> &Self::Target {
                $borrowed_ty::new(&self.0)
            }
        }

        impl ToOwned for $borrowed_ty {
            type Owned = $owned_ty;

            fn to_owned(&self) -> Self::Owned {
                $owned_ty(self.0.to_owned())
            }
        }

        // Owned and borrowed types can be compared:
        impl_partial_eq!($borrowed_ty, $owned_ty, $borrowed_ty);
        impl_partial_eq!($borrowed_ty, $owned_ty, &$borrowed_ty);

        // Types can be compared with (weakly-typed) string:
        impl_partial_eq_str!($borrowed_ty, $owned_ty, str);
        impl_partial_eq_str!($borrowed_ty, $owned_ty, &str);
        impl_partial_eq_str!($borrowed_ty, $owned_ty, String);
        impl_partial_eq_str!($borrowed_ty, $borrowed_ty, str);
        impl_partial_eq_str!($borrowed_ty, $borrowed_ty, &str);
        impl_partial_eq_str!($borrowed_ty, $borrowed_ty, String);
        impl_partial_eq_str!($borrowed_ty, &$borrowed_ty, str);
        impl_partial_eq_str!($borrowed_ty, &$borrowed_ty, String);
    };
}

impl_name_type!(GitRefNameBuf, GitRefName);
// TODO: split RefName into BookmarkName and TagName? That will make sense at
// repo/view API surface, but we'll need generic RemoteRefSymbol type, etc.
impl_name_type!(RefNameBuf, RefName);
impl_name_type!(RemoteNameBuf, RemoteName);
impl_name_type!(WorkspaceNameBuf, WorkspaceName);

impl RefName {
    /// Constructs a remote symbol with this local name.
    pub fn to_remote_symbol<'a>(&'a self, remote: &'a RemoteName) -> RemoteRefSymbol<'a> {
        RemoteRefSymbol { name: self, remote }
    }
}

impl WorkspaceName {
    /// Default workspace name.
    pub const DEFAULT: &Self = Self::new("default");
}

/// Symbol for displaying.
///
/// This type can be displayed with quoting and escaping if necessary.
#[derive(Debug, RefCastCustom)]
#[repr(transparent)]
pub struct RefSymbol(str);

impl RefSymbol {
    /// Wraps string name.
    #[ref_cast_custom]
    const fn new(name: &str) -> &Self;
}

impl Display for RefSymbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&revset::format_symbol(&self.0))
    }
}

/// Owned remote bookmark or tag name.
///
/// This type can be displayed in `{name}@{remote}` form, with quoting and
/// escaping if necessary.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RemoteRefSymbolBuf {
    /// Local name.
    pub name: RefNameBuf,
    /// Remote name.
    pub remote: RemoteNameBuf,
}

impl RemoteRefSymbolBuf {
    /// Converts to reference type.
    pub fn as_ref(&self) -> RemoteRefSymbol<'_> {
        RemoteRefSymbol {
            name: &self.name,
            remote: &self.remote,
        }
    }
}

/// Borrowed remote bookmark or tag name.
///
/// This type can be displayed in `{name}@{remote}` form, with quoting and
/// escaping if necessary.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RemoteRefSymbol<'a> {
    /// Local name.
    pub name: &'a RefName,
    /// Remote name.
    pub remote: &'a RemoteName,
}

impl RemoteRefSymbol<'_> {
    /// Converts to owned type.
    pub fn to_owned(self) -> RemoteRefSymbolBuf {
        RemoteRefSymbolBuf {
            name: self.name.to_owned(),
            remote: self.remote.to_owned(),
        }
    }
}

impl From<RemoteRefSymbol<'_>> for RemoteRefSymbolBuf {
    fn from(value: RemoteRefSymbol<'_>) -> Self {
        value.to_owned()
    }
}

impl PartialEq<RemoteRefSymbol<'_>> for RemoteRefSymbolBuf {
    fn eq(&self, other: &RemoteRefSymbol) -> bool {
        self.as_ref() == *other
    }
}

impl PartialEq<RemoteRefSymbol<'_>> for &RemoteRefSymbolBuf {
    fn eq(&self, other: &RemoteRefSymbol) -> bool {
        self.as_ref() == *other
    }
}

impl PartialEq<RemoteRefSymbolBuf> for RemoteRefSymbol<'_> {
    fn eq(&self, other: &RemoteRefSymbolBuf) -> bool {
        *self == other.as_ref()
    }
}

impl PartialEq<&RemoteRefSymbolBuf> for RemoteRefSymbol<'_> {
    fn eq(&self, other: &&RemoteRefSymbolBuf) -> bool {
        *self == other.as_ref()
    }
}

impl Display for RemoteRefSymbolBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.as_ref(), f)
    }
}

impl Display for RemoteRefSymbol<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let RemoteRefSymbol { name, remote } = self;
        f.pad(&revset::format_remote_symbol(&name.0, &remote.0))
    }
}
