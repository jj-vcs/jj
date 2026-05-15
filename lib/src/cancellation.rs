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

//! Thread-safe global cancellation support.

use std::sync::atomic::{AtomicBool, Ordering};

static CANCELED: AtomicBool = AtomicBool::new(false);

/// Signal that a cancellation has been requested (e.g. from a SIGINT handler).
pub fn request_cancellation() {
    CANCELED.store(true, Ordering::SeqCst);
}

/// Check if cancellation has been requested.
pub fn is_canceled() -> bool {
    CANCELED.load(Ordering::SeqCst)
}

/// Reset the cancellation status. Mainly used to prevent cross-test contamination.
pub fn reset_cancellation() {
    CANCELED.store(false, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cancellation_flow() {
        reset_cancellation();
        assert!(!is_canceled(), "should not be canceled initially");

        request_cancellation();
        assert!(is_canceled(), "should be canceled after request");

        reset_cancellation();
        assert!(!is_canceled(), "should not be canceled after reset");
    }
}
