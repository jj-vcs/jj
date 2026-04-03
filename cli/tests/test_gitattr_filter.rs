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

use testutils::git;

use crate::common::TestEnvironment;

#[test]
fn test_gitattr_filter_update() {
    let filter_name = "fakefilter";
    let test_env = TestEnvironment::default();
    test_env.add_config(indoc::formatdoc!(
        r#"
        git.filter.enabled = true

        [git.filter.drivers.{filter_name}]
        smudge = [{}, "--uppercase"]
        "#,
        toml::Value::String(env!("CARGO_BIN_EXE_fake-gitattr-filter").to_string()),
    ));
    let work_dir = test_env.work_dir("repo");
    git::init(work_dir.root());
    work_dir
        .run_jj(["git", "init", "--git-repo", "."])
        .success();
    let test_file_name = "test-file.txt";
    work_dir.write_file(
        ".gitattributes",
        format!("{test_file_name} filter={filter_name}\n"),
    );
    let file_path = work_dir.root().join(test_file_name);
    std::fs::write(&file_path, "abcdefg\n").unwrap();
    let bookmark_name = "test_change";
    work_dir
        .run_jj(["bookmark", "create", "-r@", bookmark_name])
        .success();
    work_dir.run_jj(["new", "root()"]).success();
    assert!(!std::fs::exists(&file_path).unwrap());
    work_dir.run_jj(["edit", bookmark_name]).success();
    let actual_contents = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(actual_contents, "ABCDEFG\n");
}

#[test]
fn test_gitattr_filter_update_optional_filter_failed() {
    let filter_name = "fakefilter";
    let test_env = TestEnvironment::default();
    test_env.add_config(indoc::formatdoc!(
        r#"
        git.filter.enabled = true

        [git.filter.drivers.{filter_name}]
        smudge = [{}, "--uppercase", "--abort-on-end"]
        required = false
        "#,
        toml::Value::String(env!("CARGO_BIN_EXE_fake-gitattr-filter").to_string()),
    ));
    let work_dir = test_env.work_dir("repo");
    git::init(work_dir.root());
    work_dir
        .run_jj(["git", "init", "--git-repo", "."])
        .success();
    // A unique file name, so that it won't be mixed with other message.
    let test_file_name = "ajo0gW7EYg85fU23cwNxHf0iIkXlL2.txt";
    work_dir.write_file(
        ".gitattributes",
        format!("{test_file_name} filter={filter_name}\n"),
    );
    let file_path = work_dir.root().join(test_file_name);
    let contents = "abcdefg\n";
    std::fs::write(&file_path, contents).unwrap();
    let bookmark_name = "test_change";
    work_dir
        .run_jj(["bookmark", "create", "-r@", bookmark_name])
        .success();
    work_dir.run_jj(["new", "root()"]).success();
    assert!(!std::fs::exists(&file_path).unwrap());
    let stderr = work_dir
        .run_jj(["edit", bookmark_name])
        .success()
        .stderr
        .into_raw();
    assert!(stderr.contains(test_file_name));
    let actual_contents = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(actual_contents, contents);
}

#[test]
fn test_gitattr_filter_update_required_filter_failed() {
    // A unique filter name, so that we can check if it exists in the error message.
    let filter_name = "FMRhnsol9Z25oa741Ny2";
    let test_env = TestEnvironment::default();
    test_env.add_config(indoc::formatdoc!(
        r#"
        git.filter.enabled = true

        [git.filter.drivers.{filter_name}]
        smudge = [{}, "--abort-on-start"]
        required = true
        "#,
        toml::Value::String(env!("CARGO_BIN_EXE_fake-gitattr-filter").to_string()),
    ));
    let work_dir = test_env.work_dir("repo");
    git::init(work_dir.root());
    work_dir
        .run_jj(["git", "init", "--git-repo", "."])
        .success();
    // A unique file name, so that it won't be mixed with other message.
    let test_file_name = "aA6FPp0ELKsHY0RnF9sl.txt";
    work_dir.write_file(
        ".gitattributes",
        format!("{test_file_name} filter={filter_name}\n"),
    );
    let file_path = work_dir.root().join(test_file_name);
    let contents = "abcdefg\n";
    std::fs::write(&file_path, contents).unwrap();
    let bookmark_name = "test_change";
    work_dir
        .run_jj(["bookmark", "create", "-r@", bookmark_name])
        .success();
    work_dir.run_jj(["new", "root()"]).success();
    assert!(!std::fs::exists(&file_path).unwrap());
    let output = work_dir.run_jj(["edit", bookmark_name]);
    assert!(!output.status.success());
    assert!(
        output.stderr.raw().contains(filter_name),
        "Expect the stderr to contain the filter name {filter_name}, but got\n{}",
        output.stderr.raw()
    );
    assert!(
        output.stderr.raw().contains(test_file_name),
        "Expect the stderr to contain the file name {test_file_name}, but got\n{}",
        output.stderr.raw()
    );
}

#[test]
fn test_gitattr_filter_snapshot() {
    let filter_name = "fakefilter";
    let test_env = TestEnvironment::default();
    test_env.add_config(indoc::formatdoc!(
        r#"
        git.filter.enabled = true

        [git.filter.drivers.{filter_name}]
        clean = [{}, "--uppercase"]
        "#,
        toml::Value::String(env!("CARGO_BIN_EXE_fake-gitattr-filter").to_string()),
    ));
    let work_dir = test_env.work_dir("repo");
    git::init(work_dir.root());
    work_dir
        .run_jj(["git", "init", "--git-repo", "."])
        .success();
    let test_file_name = "test-file.txt";
    work_dir.write_file(
        ".gitattributes",
        format!("{test_file_name} filter={filter_name}\n"),
    );
    let file_path = work_dir.root().join(test_file_name);
    std::fs::write(&file_path, "abcdefg\n").unwrap();
    let bookmark_name = "test_change";
    work_dir
        .run_jj(["bookmark", "create", "-r@", bookmark_name])
        .success();
    work_dir.run_jj(["new", "root()"]).success();
    assert!(!std::fs::exists(&file_path).unwrap());
    work_dir.run_jj(["edit", bookmark_name]).success();
    let actual_contents = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(actual_contents, "ABCDEFG\n");
}

#[test]
fn test_gitattr_filter_snapshot_optional_filter_failed() {
    let filter_name = "fakefilter";
    let test_env = TestEnvironment::default();
    test_env.add_config(indoc::formatdoc!(
        r#"
        git.filter.enabled = true

        [git.filter.drivers.{filter_name}]
        clean = [{}, "--uppercase", "--abort-on-end"]
        required = false
        "#,
        toml::Value::String(env!("CARGO_BIN_EXE_fake-gitattr-filter").to_string()),
    ));
    let work_dir = test_env.work_dir("repo");
    git::init(work_dir.root());
    work_dir
        .run_jj(["git", "init", "--git-repo", "."])
        .success();
    // A unique file name, so that it won't be mixed with other message.
    let test_file_name = "X5x40ZX4o5Z9lAFX1i9Q.txt";
    work_dir.write_file(
        ".gitattributes",
        format!("{test_file_name} filter={filter_name}\n"),
    );
    let file_path = work_dir.root().join(test_file_name);
    let contents = "abcdefg\n";
    std::fs::write(&file_path, contents).unwrap();
    let bookmark_name = "test_change";
    let stderr = work_dir
        .run_jj(["bookmark", "create", "-r@", bookmark_name])
        .success()
        .stderr
        .into_raw();
    assert!(
        stderr.contains(test_file_name),
        "Expect stderr to include the file name {test_file_name}, but got\n{stderr}"
    );
    work_dir.run_jj(["new", "root()"]).success();
    assert!(!std::fs::exists(&file_path).unwrap());
    work_dir.run_jj(["edit", bookmark_name]).success();
    let actual_contents = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(actual_contents, contents);
}

#[test]
fn test_gitattr_filter_snapshot_required_filter_failed() {
    // A unique filter name, so that we can check if it exists in the error message.
    let filter_name = "kfkPEnd65BLHEaSXhE09";
    let test_env = TestEnvironment::default();
    test_env.add_config(indoc::formatdoc!(
        r#"
        git.filter.enabled = true

        [git.filter.drivers.{filter_name}]
        clean = [{}, "--abort-on-start"]
        required = true
        "#,
        toml::Value::String(env!("CARGO_BIN_EXE_fake-gitattr-filter").to_string()),
    ));
    let work_dir = test_env.work_dir("repo");
    git::init(work_dir.root());
    work_dir
        .run_jj(["git", "init", "--git-repo", "."])
        .success();
    // A unique file name, so that it won't be mixed with other message.
    let test_file_name = "3WC00zfLMNRxrAwQA6hy.txt";
    work_dir.write_file(
        ".gitattributes",
        format!("{test_file_name} filter={filter_name}\n"),
    );
    let file_path = work_dir.root().join(test_file_name);
    let contents = "abcdefg\n";
    std::fs::write(&file_path, contents).unwrap();
    let output = work_dir.run_jj(["log"]);
    assert!(!output.status.success());
    let stderr = output.stderr.raw();
    assert!(
        stderr.contains(test_file_name),
        "Expect stderr to include the file name {test_file_name}, but got\n{stderr}"
    );
    assert!(
        stderr.contains(test_file_name),
        "Expect stderr to include the filter name {filter_name}, but got\n{stderr}"
    );
}
