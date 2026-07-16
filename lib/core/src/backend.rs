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

//! Contains a basic shim for some Backend types such as [`ChangeId`] and
//! [`CommitId`].
// TODO: move the `Backend` trait into this.

use crate::hex_util;
use crate::object_id::ObjectId as _;
use crate::object_id::id_type;

id_type!(
   /// Identifier for a `Commit` based on its content. When a commit is
    /// rewritten, its `CommitId` changes.
    pub CommitId { hex() }
);
id_type!(
    /// Stable identifier for a `Commit`. Unlike the `CommitId`, the `ChangeId`
    /// follows the commit and is not updated when the commit is rewritten.
    pub ChangeId { reverse_hex() }
);

impl ChangeId {
    /// Parses the given "reverse" hex string into a `ChangeId`.
    pub fn try_from_reverse_hex(hex: impl AsRef<[u8]>) -> Option<Self> {
        hex_util::decode_reverse_hex(hex).map(Self)
    }

    /// Returns the hex string representation of this ID, which uses `z-k`
    /// "digits" instead of `0-9a-f`.
    pub fn reverse_hex(&self) -> String {
        hex_util::encode_reverse_hex(&self.0)
    }
}
