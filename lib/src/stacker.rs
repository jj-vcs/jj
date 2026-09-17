// Copyright 2020 The Jujutsu Authors
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

// TODO: add module docs
#![expect(missing_docs)]

use stacker::remaining_stack;
use thiserror::Error;

// The minimum amount of bytes left on the stack (20k).
// Determined empirically.
// TBD: do we want this to be global or should it be specified at each call-site?
const RED_ZONE: usize = 20 * 1024;

#[derive(Debug, Error)]
pub enum StackerError {
    #[error("Red zone reached (less than {0} bytes left on the stack).")]
    RedZoneReached(usize),
    #[error("Unable to query remaining stack.")]
    StackSizeUnknown,
}

/// Checks whether sufficient amount of space is still left on the stack.
/// Errors if the remaining stack is too small or if the information could
/// not be obtained.
pub fn check_stack() -> Result<(), StackerError> {
    if let Some(left) = remaining_stack() {
        if left > RED_ZONE {
            Ok(())
        } else {
            Err(StackerError::RedZoneReached(RED_ZONE))
        }
    } else {
        Err(StackerError::StackSizeUnknown)
    }
}
