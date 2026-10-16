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

//! Logic shared by user interfaces for Jujutsu, such as the CLI and GUIs:
//! snapshotting the working copy before an operation, importing from and
//! exporting to Git, finishing transactions, and updating the working copy
//! afterwards. None of it depends on how the user interacts with the repo;
//! things that happen along the way are reported as data for the caller to
//! present.

#![warn(missing_docs)]
#![deny(unused_must_use)]
#![deny(unsafe_code)]

pub mod revset_util;
