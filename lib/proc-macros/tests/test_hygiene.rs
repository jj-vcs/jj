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

#[test]
fn no_direct_digest_references_in_macro_output() {
    for (filename, source) in [
        ("lib.rs", include_str!("../src/lib.rs")),
        ("content_hash.rs", include_str!("../src/content_hash.rs")),
    ] {
        // The proc-macro output must not reference `digest::` directly, as
        // that would
        // 1) require downstream crates to add `digest` as a direct dependency
        // 2) cause compilation errors if downstream crates use a *different version* of
        //    `digest` than jj-lib.
        assert!(
            !source.contains("digest::"),
            "{filename} references `digest::` directly; use \
             `::jj_lib::content_hash::DigestUpdate` instead"
        );
    }
}
