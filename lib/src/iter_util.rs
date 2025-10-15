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

//! Iterator helpers.

/// Returns `Ok(true)` if any element satisfies the fallible predicate,
/// `Ok(false)` if none do. Returns `Err` on the first error encountered.
pub fn fallible_any<T, E>(
    iter: impl IntoIterator<Item = T>,
    mut predicate: impl FnMut(T) -> Result<bool, E>,
) -> Result<bool, E> {
    for item in iter {
        if predicate(item)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Returns `Ok(Some(item))` for the first element where the predicate returns
/// `Ok(true)`, `Ok(None)` if no element satisfies it, or `Err` on the first
/// error.
pub fn fallible_find<T, E>(
    iter: impl IntoIterator<Item = T>,
    mut predicate: impl FnMut(&T) -> Result<bool, E>,
) -> Result<Option<T>, E> {
    for item in iter {
        if predicate(&item)? {
            return Ok(Some(item));
        }
    }
    Ok(None)
}

/// Returns `Ok(Some(index))` for the first element where the predicate returns
/// `Ok(true)`, `Ok(None)` if no element satisfies it, or `Err` on the first
/// error.
pub fn fallible_position<T, E>(
    iter: impl IntoIterator<Item = T>,
    mut predicate: impl FnMut(T) -> Result<bool, E>,
) -> Result<Option<usize>, E> {
    for (index, item) in iter.into_iter().enumerate() {
        if predicate(item)? {
            return Ok(Some(index));
        }
    }
    Ok(None)
}
