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

#![expect(missing_docs)]

use std::fmt;
use std::fmt::Debug;

use crate::hex_util;

pub trait ObjectId {
    fn object_type(&self) -> String;
    fn as_bytes(&self) -> &[u8];
    fn to_bytes(&self) -> Vec<u8>;
    fn hex(&self) -> String;
}

// Defines a new struct type with visibility `vis` and name `ident` containing
// a single Vec<u8> used to store an identifier (typically the output of a hash
// function) as bytes. Types defined using this macro automatically implement
// the `ObjectId` and `ContentHash` traits.
// Documentation comments written inside the macro definition will be captured
// and associated with the type defined by the macro.
//
// Example:
// ```no_run
// id_type!(
//     /// My favorite id type.
//     pub MyId { hex() }
// );
// ```
macro_rules! id_type {
    (   $(#[$attr:meta])*
        $vis:vis $name:ident { $hex_method:ident() }
    ) => {
        $(#[$attr])*
        #[derive($crate::content_hash::ContentHash, PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
        $vis struct $name(Vec<u8>);
        $crate::object_id::impl_id_type!($name, $hex_method);
    };
}

macro_rules! impl_id_type {
    ($name:ident, $hex_method:ident) => {
        #[allow(dead_code)]
        impl $name {
            pub fn new(value: Vec<u8>) -> Self {
                Self(value)
            }

            pub fn from_bytes(bytes: &[u8]) -> Self {
                Self(bytes.to_vec())
            }

            /// Parses the given hex string into an ObjectId.
            ///
            /// The given string must be valid. A static str is required to
            /// prevent API misuse.
            pub fn from_hex(hex: &'static str) -> Self {
                Self::try_from_hex(hex).unwrap()
            }

            /// Parses the given hex string into an ObjectId.
            pub fn try_from_hex(hex: impl AsRef<[u8]>) -> Option<Self> {
                $crate::hex_util::decode_hex(hex).map(Self)
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
                // TODO: should we use $hex_method here?
                f.debug_tuple(stringify!($name)).field(&self.hex()).finish()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
                f.pad(&self.$hex_method())
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                if serializer.is_human_readable() {
                    self.$hex_method().serialize(serializer)
                } else {
                    self.as_bytes().serialize(serializer)
                }
            }
        }

        impl crate::object_id::ObjectId for $name {
            fn object_type(&self) -> String {
                stringify!($name)
                    .strip_suffix("Id")
                    .unwrap()
                    .to_ascii_lowercase()
                    .to_string()
            }

            fn as_bytes(&self) -> &[u8] {
                &self.0
            }

            fn to_bytes(&self) -> Vec<u8> {
                self.0.clone()
            }

            fn hex(&self) -> String {
                $crate::hex_util::encode_hex(&self.0)
            }
        }
    };
}

pub(crate) use id_type;
pub(crate) use impl_id_type;

/// An identifier prefix (typically from a type implementing the [`ObjectId`]
/// trait) with facilities for converting between bytes and a hex string.
#[derive(Clone, PartialEq, Eq)]
pub struct HexPrefix {
    // For odd-length prefixes, the lower 4 bits of the last byte are
    // zero-filled (e.g. the prefix "abc" is stored in two bytes as "abc0").
    min_prefix_bytes: Vec<u8>,
    has_odd_byte: bool,
}

impl HexPrefix {
    /// Returns a new `HexPrefix` or `None` if `prefix` cannot be decoded from
    /// hex to bytes.
    pub fn try_from_hex(prefix: impl AsRef<[u8]>) -> Option<Self> {
        let (min_prefix_bytes, has_odd_byte) = hex_util::decode_hex_prefix(prefix)?;
        Some(Self {
            min_prefix_bytes,
            has_odd_byte,
        })
    }

    /// Returns a new `HexPrefix` or `None` if `prefix` cannot be decoded from
    /// "reverse" hex to bytes.
    pub fn try_from_reverse_hex(prefix: impl AsRef<[u8]>) -> Option<Self> {
        let (min_prefix_bytes, has_odd_byte) = hex_util::decode_reverse_hex_prefix(prefix)?;
        Some(Self {
            min_prefix_bytes,
            has_odd_byte,
        })
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            min_prefix_bytes: bytes.to_owned(),
            has_odd_byte: false,
        }
    }

    /// Returns a new `HexPrefix` representing the given `id`.
    pub fn from_id<T: ObjectId + ?Sized>(id: &T) -> Self {
        Self::from_bytes(id.as_bytes())
    }

    /// Returns string representation of this prefix using hex digits.
    pub fn hex(&self) -> String {
        let mut hex_string = hex_util::encode_hex(&self.min_prefix_bytes);
        if self.has_odd_byte {
            hex_string.pop().unwrap();
        }
        hex_string
    }

    /// Returns string representation of this prefix using `z-k` "digits".
    pub fn reverse_hex(&self) -> String {
        let mut hex_string = hex_util::encode_reverse_hex(&self.min_prefix_bytes);
        if self.has_odd_byte {
            hex_string.pop().unwrap();
        }
        hex_string
    }

    /// Minimum bytes that would match this prefix. (e.g. "abc0" for "abc")
    ///
    /// Use this to partition a sorted slice, and test `matches(id)` from there.
    pub fn min_prefix_bytes(&self) -> &[u8] {
        &self.min_prefix_bytes
    }

    /// Returns the bytes representation if this prefix can be a full id.
    pub fn as_full_bytes(&self) -> Option<&[u8]> {
        (!self.has_odd_byte).then_some(&self.min_prefix_bytes)
    }

    fn split_odd_byte(&self) -> (Option<u8>, &[u8]) {
        if self.has_odd_byte {
            let (&odd, prefix) = self.min_prefix_bytes.split_last().unwrap();
            (Some(odd), prefix)
        } else {
            (None, &self.min_prefix_bytes)
        }
    }

    /// Returns whether the stored prefix matches the prefix of `id`.
    pub fn matches<Q: ObjectId>(&self, id: &Q) -> bool {
        let id_bytes = id.as_bytes();
        let (maybe_odd, prefix) = self.split_odd_byte();
        if id_bytes.starts_with(prefix) {
            if let Some(odd) = maybe_odd {
                matches!(id_bytes.get(prefix.len()), Some(v) if v & 0xf0 == odd)
            } else {
                true
            }
        } else {
            false
        }
    }
}

impl Debug for HexPrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.debug_tuple("HexPrefix").field(&self.hex()).finish()
    }
}

/// The result of a prefix search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefixResolution<T> {
    NoMatch,
    SingleMatch(T),
    AmbiguousMatch,
}

impl<T> PrefixResolution<T> {
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> PrefixResolution<U> {
        match self {
            Self::NoMatch => PrefixResolution::NoMatch,
            Self::SingleMatch(x) => PrefixResolution::SingleMatch(f(x)),
            Self::AmbiguousMatch => PrefixResolution::AmbiguousMatch,
        }
    }

    pub fn filter_map<U>(self, f: impl FnOnce(T) -> Option<U>) -> PrefixResolution<U> {
        match self {
            Self::NoMatch => PrefixResolution::NoMatch,
            Self::SingleMatch(x) => match f(x) {
                None => PrefixResolution::NoMatch,
                Some(y) => PrefixResolution::SingleMatch(y),
            },
            Self::AmbiguousMatch => PrefixResolution::AmbiguousMatch,
        }
    }
}

impl<T: Clone> PrefixResolution<T> {
    pub fn plus(&self, other: &Self) -> Self {
        match (self, other) {
            (Self::NoMatch, other) => other.clone(),
            (local, Self::NoMatch) => local.clone(),
            (Self::AmbiguousMatch, _) => Self::AmbiguousMatch,
            (_, Self::AmbiguousMatch) => Self::AmbiguousMatch,
            (Self::SingleMatch(_), Self::SingleMatch(_)) => Self::AmbiguousMatch,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ChangeId;
    use crate::backend::CommitId;

    #[test]
    fn test_display_object_id() {
        let commit_id = CommitId::from_hex("deadbeef0123");
        assert_eq!(format!("{commit_id}"), "deadbeef0123");
        assert_eq!(format!("{commit_id:.6}"), "deadbe");

        let change_id = ChangeId::from_hex("deadbeef0123");
        assert_eq!(format!("{change_id}"), "mlpmollkzyxw");
        assert_eq!(format!("{change_id:.6}"), "mlpmol");
    }

    #[test]
    fn test_hex_prefix_prefixes() {
        let prefix = HexPrefix::try_from_hex("").unwrap();
        assert_eq!(prefix.min_prefix_bytes(), b"");

        let prefix = HexPrefix::try_from_hex("1").unwrap();
        assert_eq!(prefix.min_prefix_bytes(), b"\x10");

        let prefix = HexPrefix::try_from_hex("12").unwrap();
        assert_eq!(prefix.min_prefix_bytes(), b"\x12");

        let prefix = HexPrefix::try_from_hex("123").unwrap();
        assert_eq!(prefix.min_prefix_bytes(), b"\x12\x30");

        let bad_prefix = HexPrefix::try_from_hex("0x123");
        assert_eq!(bad_prefix, None);

        let bad_prefix = HexPrefix::try_from_hex("foobar");
        assert_eq!(bad_prefix, None);
    }

    #[test]
    fn test_hex_prefix_matches() {
        let id = CommitId::from_hex("1234");

        assert!(HexPrefix::try_from_hex("").unwrap().matches(&id));
        assert!(HexPrefix::try_from_hex("1").unwrap().matches(&id));
        assert!(HexPrefix::try_from_hex("12").unwrap().matches(&id));
        assert!(HexPrefix::try_from_hex("123").unwrap().matches(&id));
        assert!(HexPrefix::try_from_hex("1234").unwrap().matches(&id));
        assert!(!HexPrefix::try_from_hex("12345").unwrap().matches(&id));

        assert!(!HexPrefix::try_from_hex("a").unwrap().matches(&id));
        assert!(!HexPrefix::try_from_hex("1a").unwrap().matches(&id));
        assert!(!HexPrefix::try_from_hex("12a").unwrap().matches(&id));
        assert!(!HexPrefix::try_from_hex("123a").unwrap().matches(&id));
    }
}
