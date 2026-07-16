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

//! Contains helpers and macros to make it easier to work with binary Object
//! types, with a newtype wrapper [`id_type`] and the [`ObjectId`] trait.

pub use jj_core::object_id::HexPrefix;
pub use jj_core::object_id::ObjectId;
pub use jj_core::object_id::PrefixResolution;
pub use jj_core::object_id::id_type;
pub use jj_core::object_id::impl_id_type;
