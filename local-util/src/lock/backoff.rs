// Copyright 2020-2026 The Jujutsu Authors
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

use std::time::Duration;

pub struct BackoffIterator {
    next_sleep_secs: f32,
    elapsed_secs: f32,
    max_elapsed_secs: f32,
}

impl BackoffIterator {
    #[cfg_attr(unix, expect(dead_code))]
    pub fn new() -> Self {
        Self::with_max_elapsed(Duration::from_secs(10))
    }

    /// Creates an iterator that stops yielding delays once their sum reaches
    /// `max_elapsed`.
    pub fn with_max_elapsed(max_elapsed: Duration) -> Self {
        Self {
            next_sleep_secs: 0.001,
            elapsed_secs: 0.0,
            max_elapsed_secs: max_elapsed.as_secs_f32(),
        }
    }
}

impl Iterator for BackoffIterator {
    type Item = Duration;

    fn next(&mut self) -> Option<Self::Item> {
        if self.elapsed_secs >= self.max_elapsed_secs {
            None
        } else {
            let current_sleep = self.next_sleep_secs * (rand::random::<f32>() + 0.5);
            self.next_sleep_secs *= 1.5;
            self.elapsed_secs += current_sleep;
            Some(Duration::from_secs_f32(current_sleep))
        }
    }
}
