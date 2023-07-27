// Copyright 2022 The Jujutsu Authors
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

use insta::assert_snapshot;
use regex::Regex;

use crate::common::TestEnvironment;

pub mod common;

#[test]
fn test_debug_revset() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let workspace_path = test_env.env_root().join("repo");

    let stdout = test_env.jj_cmd_success(&workspace_path, &["debug", "revset", "root"]);
    insta::with_settings!({filters => vec![
        (r"(?m)(^    .*\n)+", "    ..\n"),
    ]}, {
        assert_snapshot!(stdout, @r###"
        -- Parsed:
        CommitRef(
            ..
        )

        -- Optimized:
        CommitRef(
            ..
        )

        -- Resolved:
        Commits(
            ..
        )

        -- Evaluated:
        RevsetImpl {
            ..
        }

        -- Commit IDs:
        0000000000000000000000000000000000000000
        "###);
    });
}

#[test]
fn test_debug_index() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let workspace_path = test_env.env_root().join("repo");
    let stdout = test_env.jj_cmd_success(&workspace_path, &["debug", "index"]);
    assert_snapshot!(filter_index_stats(&stdout), @r###"
    Number of commits: 2
    Number of merges: 0
    Max generation number: 1
    Number of heads: 1
    Number of changes: 2
    Stats per level:
      Level 0:
        Number of commits: 2
        Name: [hash]
    "###
    );
}

#[test]
fn test_debug_reindex() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let workspace_path = test_env.env_root().join("repo");
    test_env.jj_cmd_success(&workspace_path, &["new"]);
    test_env.jj_cmd_success(&workspace_path, &["new"]);
    let stdout = test_env.jj_cmd_success(&workspace_path, &["debug", "index"]);
    assert_snapshot!(filter_index_stats(&stdout), @r###"
    Number of commits: 4
    Number of merges: 0
    Max generation number: 3
    Number of heads: 1
    Number of changes: 4
    Stats per level:
      Level 0:
        Number of commits: 3
        Name: [hash]
      Level 1:
        Number of commits: 1
        Name: [hash]
    "###
    );
    let stdout = test_env.jj_cmd_success(&workspace_path, &["debug", "reindex"]);
    assert_snapshot!(stdout, @r###"
    Finished indexing 4 commits.
    "###);
    let stdout = test_env.jj_cmd_success(&workspace_path, &["debug", "index"]);
    assert_snapshot!(filter_index_stats(&stdout), @r###"
    Number of commits: 4
    Number of merges: 0
    Max generation number: 3
    Number of heads: 1
    Number of changes: 4
    Stats per level:
      Level 0:
        Number of commits: 4
        Name: [hash]
    "###
    );
}

#[test]
fn test_debug_operation_id() {
    let test_env = TestEnvironment::default();
    test_env.jj_cmd_success(test_env.env_root(), &["init", "repo", "--git"]);
    let workspace_path = test_env.env_root().join("repo");
    let stdout =
        test_env.jj_cmd_success(&workspace_path, &["debug", "operation", "--display", "id"]);
    assert_snapshot!(filter_index_stats(&stdout), @r###"
    19b8089fc78b7c49171f3c8934248be6f89f52311005e961cab5780f9f138b142456d77b27d223d7ee84d21d8c30c4a80100eaf6735b548b1acd0da688f94c80
    "###
    );
}

fn filter_index_stats(text: &str) -> String {
    let regex = Regex::new(r"    Name: [0-9a-z]+").unwrap();
    regex.replace_all(text, "    Name: [hash]").to_string()
}
