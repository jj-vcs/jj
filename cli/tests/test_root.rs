// Copyright 2024 The Jujutsu Authors
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

use std::path::Path;

use test_case::test_case;
use testutils::TestRepoBackend;
use testutils::TestResult;
use testutils::TestWorkspace;

use crate::common::TestEnvironment;

#[test_case(TestRepoBackend::Simple ; "simple backend")]
#[test_case(TestRepoBackend::Git ; "git backend")]
fn test_root(backend: TestRepoBackend) -> TestResult {
    let test_env = TestEnvironment::default();
    let test_workspace = TestWorkspace::init_with_backend(backend);
    let root = test_workspace.workspace.workspace_root();
    let subdir = root.join("subdir");
    std::fs::create_dir(&subdir)?;
    let output = test_env.run_jj_in(&subdir, ["root"]).success();
    assert_eq!(
        output.stdout.raw(),
        &[root.to_str().unwrap(), "\n"].concat()
    );
    Ok(())
}

#[test]
fn test_root_outside_a_repo() {
    let test_env = TestEnvironment::default();
    let output = test_env.run_jj_in(Path::new("/"), ["root"]);
    insta::assert_snapshot!(output, @r#"
    ------- stderr -------
    Error: There is no jj repo in "."
    [EOF]
    [exit status: 1]
    "#);
}

#[test_case(TestRepoBackend::Simple ; "simple backend")]
#[test_case(TestRepoBackend::Git ; "git backend")]
fn test_root_inside_a_nested_repo(backend: TestRepoBackend) -> TestResult {
    let test_env = TestEnvironment::default();
    let test_workspace = TestWorkspace::init_with_backend(backend);
    let root = test_workspace.workspace.workspace_root();
    let subdir = root.join("subdir");
    std::fs::create_dir_all(subdir.join(".git"))?;

    let mut insta_settings = insta::Settings::clone_current();
    insta_settings.add_filter(&regex::escape(&root.to_string_lossy()), "<root>");
    let _guard = insta_settings.bind_to_scope();

    let output = test_env.run_jj_in(&subdir, ["root"]);
    insta::assert_snapshot!(output, @r#"
        ------- stderr -------
        Error: There is no jj repo in "."
        Hint: It looks like this is a git repo. You can create a jj repo backed by it by running this:
        jj git init
        Hint: Or, you may have intended to run this command inside "<root>"
        [EOF]
        [exit status: 1]
    "#);

    Ok(())
}
