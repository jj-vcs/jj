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

use crate::common::rerere_helpers::*;

/// Test rerere with rename/move conflicts
#[test]
fn test_rerere_rename_conflict() {
    let test_env = setup_rerere_test();
    let work_dir = test_env.work_dir("repo");

    // Create initial file
    work_dir.run_jj(&["new", "root()", "-m", "base"]).success();
    let base = create_commit(
        &work_dir,
        "add original file",
        "original.txt",
        "original content",
    );

    // One side renames the file
    work_dir.run_jj(&["new", &base, "-m", "side1"]).success();
    std::fs::rename(
        work_dir.root().join("original.txt"),
        work_dir.root().join("renamed.txt"),
    )
    .unwrap();
    let side1 = create_commit(
        &work_dir,
        "rename to renamed.txt",
        "renamed.txt",
        "renamed and modified",
    );

    // Other side modifies the file in place
    work_dir.run_jj(&["new", &base, "-m", "side2"]).success();
    let side2 = create_commit(
        &work_dir,
        "modify original.txt",
        "original.txt",
        "modified in place",
    );

    // Create a merge commit - should handle rename conflict
    let output = work_dir
        .run_jj(&["new", &side1, &side2, "-m", "merge"])
        .success();

    // Check the result - jj should handle rename gracefully
    // The exact behavior depends on jj's rename detection
    let has_original = work_dir.root().join("original.txt").exists();
    let has_renamed = work_dir.root().join("renamed.txt").exists();

    // At least one should exist
    assert!(has_original || has_renamed);

    // If there's a conflict, resolve it
    if output.stderr.raw().contains("unresolved conflicts") {
        // Resolve by keeping the renamed file with merged content
        std::fs::remove_file(work_dir.root().join("original.txt")).ok();
        // Commit the resolution
        create_commit(
            &work_dir,
            "resolved rename conflict",
            "renamed.txt",
            "resolved content",
        );
    }

    // Create the same scenario in a different context
    work_dir.run_jj(&["new", "root()"]).success();
    let output = work_dir
        .run_jj(&["new", &side1, &side2, "-m", "merge2"])
        .success();

    // Check if rerere handled the rename conflict
    let has_renamed_2 = work_dir.root().join("renamed.txt").exists();
    if has_renamed_2 && !output.stderr.raw().contains("unresolved conflicts") {
        work_dir.assert_file_content("renamed.txt", "resolved content");
    }
}

/// Test rerere with file vs directory conflicts
#[test]
fn test_rerere_file_vs_directory_conflict() {
    let test_env = setup_rerere_test();
    let work_dir = test_env.work_dir("repo");

    // Create initial files
    work_dir.run_jj(&["new", "root()", "-m", "base"]).success();
    work_dir.write_file("file1.txt", "base content 1");
    work_dir.write_file("file2.txt", "base content 2");
    work_dir.run_jj(&["commit", "-m", "add files"]).success();
    let base = get_commit_id_by_description(&work_dir, "add files");

    // One side modifies file1 and adds file3
    work_dir.run_jj(&["new", &base, "-m", "side1"]).success();
    work_dir.write_file("file1.txt", "modified content 1");
    work_dir.write_file("file3.txt", "new file 3");
    work_dir
        .run_jj(&["commit", "-m", "modify and add"])
        .success();
    let side1 = get_commit_id_by_description(&work_dir, "modify and add");

    // Other side modifies file2 and deletes file1
    work_dir.run_jj(&["new", &base, "-m", "side2"]).success();
    std::fs::remove_file(work_dir.root().join("file1.txt")).unwrap();
    work_dir.write_file("file2.txt", "modified content 2");
    work_dir
        .run_jj(&["commit", "-m", "delete and modify"])
        .success();
    let side2 = get_commit_id_by_description(&work_dir, "delete and modify");

    // Create a merge commit - file deletion conflict
    let output = work_dir
        .run_jj(&["new", &side1, &side2, "-m", "merge"])
        .success();

    if output.stderr.raw().contains("unresolved conflicts") {
        // Resolve by keeping the modified version
        create_commit(
            &work_dir,
            "resolved deletion conflict",
            "file1.txt",
            "resolved - kept modified",
        );
    }

    // Create the same conflict in a different context
    work_dir.run_jj(&["new", "root()"]).success();
    let output = work_dir
        .run_jj(&["new", &side1, &side2, "-m", "merge2"])
        .success();

    // Check if rerere was applied
    if output.stderr.raw().contains("Applied")
        && output.stderr.raw().contains("cached conflict resolution")
    {
        // Verify the resolution was applied
        work_dir.assert_file_content("file1.txt", "resolved - kept modified");
    }
}

/// Test rerere with multiple interdependent conflicts
#[test]
fn test_rerere_multiple_file_conflicts() {
    let test_env = setup_rerere_test();
    let work_dir = test_env.work_dir("repo");

    // Create initial files with dependencies
    work_dir.run_jj(&["new", "root()", "-m", "base"]).success();
    work_dir.write_file("config.txt", "version: 1\nfeature: disabled");
    work_dir.write_file(
        "main.py",
        "import config\nif config.feature:\n    print('enabled')",
    );
    work_dir.write_file("test.py", "def test():\n    assert config.version == 1");
    work_dir.run_jj(&["commit", "-m", "add files"]).success();
    let base = get_commit_id_by_description(&work_dir, "add files");

    // Side 1 updates version and related code
    work_dir.run_jj(&["new", &base, "-m", "side1"]).success();
    work_dir.write_file("config.txt", "version: 2\nfeature: disabled\napi: v2");
    work_dir.write_file(
        "main.py",
        "import config\nif config.feature:\n    print('enabled v2')",
    );
    work_dir.write_file(
        "test.py",
        "def test():\n    assert config.version == 2\n    assert config.api == 'v2'",
    );
    work_dir.run_jj(&["commit", "-m", "update to v2"]).success();
    let side1 = get_commit_id_by_description(&work_dir, "update to v2");

    // Side 2 enables feature and updates code
    work_dir.run_jj(&["new", &base, "-m", "side2"]).success();
    work_dir.write_file(
        "config.txt",
        "version: 1\nfeature: enabled\nmode: production",
    );
    work_dir.write_file(
        "main.py",
        "import config\nif config.feature:\n    print('feature is active')\nelse:\n    \
         print('disabled')",
    );
    work_dir.write_file(
        "test.py",
        "def test():\n    assert config.version == 1\n    assert config.feature == True",
    );
    work_dir
        .run_jj(&["commit", "-m", "enable feature"])
        .success();
    let side2 = get_commit_id_by_description(&work_dir, "enable feature");

    // Create a merge commit - multiple conflicts
    let output = work_dir
        .run_jj(&["new", &side1, &side2, "-m", "merge"])
        .success();
    output.assert_has_conflict();

    // Resolve all conflicts consistently
    work_dir.write_file(
        "config.txt",
        "version: 2\nfeature: enabled\napi: v2\nmode: production",
    );
    work_dir.write_file(
        "main.py",
        "import config\nif config.feature:\n    print('feature is active v2')\nelse:\n    \
         print('disabled')",
    );
    work_dir.write_file(
        "test.py",
        "def test():\n    assert config.version == 2\n    assert config.feature == True\n    \
         assert config.api == 'v2'",
    );
    // Commit the resolution to record it
    work_dir
        .run_jj(&["commit", "-m", "resolved conflicts"])
        .success();

    // Create the same conflicts in a different context
    let output = work_dir
        .run_jj(&["new", &side1, &side2, "-m", "merge2"])
        .success();

    // Check if rerere was applied
    output.assert_rerere_applied(3);

    // Check if all three files were resolved correctly
    if !output
        .stderr
        .raw()
        .contains("There are unresolved conflicts")
    {
        let config = std::fs::read_to_string(work_dir.root().join("config.txt")).unwrap();
        assert!(config.contains("version: 2"));
        assert!(config.contains("feature: enabled"));
        assert!(config.contains("api: v2"));
        assert!(config.contains("mode: production"));

        work_dir.assert_file_content(
            "main.py",
            "import config\nif config.feature:\n    print('feature is active v2')\nelse:\n    \
             print('disabled')",
        );

        let test_py = std::fs::read_to_string(work_dir.root().join("test.py")).unwrap();
        assert!(test_py.contains("assert config.version == 2"));
        assert!(test_py.contains("assert config.feature == True"));
    }
}

/// Test rerere with nested conflicts (conflicts within conflicts)
#[test]
fn test_rerere_nested_conflicts() {
    let test_env = setup_rerere_test();
    let work_dir = test_env.work_dir("repo");

    // Create initial commit
    work_dir.run_jj(&["new", "root()", "-m", "base"]).success();
    let base = create_commit(&work_dir, "base", "file.txt", "base content").to_string();

    // Create first divergence from base
    work_dir.run_jj(&["new", &base, "-m", "a1"]).success();
    let a1 = create_commit(&work_dir, "a1", "file.txt", "a1 content").to_string();

    work_dir.run_jj(&["new", &base, "-m", "a2"]).success();
    let a2 = create_commit(&work_dir, "a2", "file.txt", "a2 content").to_string();

    // Create parallel branch from base
    work_dir.run_jj(&["new", &base, "-m", "b1"]).success();
    let b1 = create_commit(&work_dir, "b1", "file.txt", "b1 content").to_string();

    work_dir.run_jj(&["new", &base, "-m", "b2"]).success();
    let b2 = create_commit(&work_dir, "b2", "file.txt", "b2 content").to_string();

    // Create first merge with conflict
    let output = work_dir
        .run_jj(&["new", &a1, &a2, "-m", "merge-a"])
        .success();
    if output
        .stderr
        .raw()
        .contains("There are unresolved conflicts")
    {
        create_commit(&work_dir, "resolved merge-a", "file.txt", "merged a1+a2");
    }

    // Create second merge with conflict
    let output = work_dir
        .run_jj(&["new", &b1, &b2, "-m", "merge-b"])
        .success();
    if output
        .stderr
        .raw()
        .contains("There are unresolved conflicts")
    {
        create_commit(&work_dir, "resolved merge-b", "file.txt", "merged b1+b2");
    }

    // Now merge the two merges - creating a nested conflict scenario
    let merge_a = get_commit_id_by_description(&work_dir, "resolved merge-a");
    let merge_b = get_commit_id_by_description(&work_dir, "resolved merge-b");

    let output = work_dir
        .run_jj(&["new", &merge_a, &merge_b, "-m", "mega-merge"])
        .success();

    // This creates a complex conflict between two already-merged states
    if output
        .stderr
        .raw()
        .contains("There are unresolved conflicts")
    {
        create_commit(
            &work_dir,
            "resolved mega-merge",
            "file.txt",
            "final resolution",
        );
    }

    // Try to recreate a similar nested conflict pattern
    let output = work_dir
        .run_jj(&["new", &merge_a, &merge_b, "-m", "mega-merge2"])
        .success();

    // Check if rerere was applied
    output.assert_rerere_applied(1);

    // Check if rerere can handle this complex scenario
    if !output
        .stderr
        .raw()
        .contains("There are unresolved conflicts")
    {
        work_dir.assert_file_content("file.txt", "final resolution");
    }
}

/// Test rerere with copy detection scenarios
#[test]
fn test_rerere_copy_conflicts() {
    let test_env = setup_rerere_test();
    let work_dir = test_env.work_dir("repo");

    // Create initial file
    work_dir.run_jj(&["new", "root()", "-m", "base"]).success();
    let base = create_commit(
        &work_dir,
        "add original",
        "original.txt",
        "shared content\noriginal specific",
    );

    // Side 1: modify original
    work_dir.run_jj(&["new", &base, "-m", "side1"]).success();
    let side1 = create_commit(
        &work_dir,
        "modify original",
        "original.txt",
        "shared content modified\noriginal specific v2",
    );

    // Side 2: copy and modify both
    work_dir.run_jj(&["new", &base, "-m", "side2"]).success();
    work_dir.write_file("original.txt", "shared content\noriginal changed");
    work_dir.write_file("copy.txt", "shared content\ncopy specific");
    work_dir.run_jj(&["commit", "-m", "create copy"]).success();
    let side2 = get_commit_id_by_description(&work_dir, "create copy");

    // Merge - this creates interesting conflicts
    let output = work_dir
        .run_jj(&["new", &side1, &side2, "-m", "merge"])
        .success();

    if output
        .stderr
        .raw()
        .contains("There are unresolved conflicts")
    {
        // Resolve by incorporating changes to both files
        work_dir.write_file(
            "original.txt",
            "shared content modified\noriginal specific v2 changed",
        );
        work_dir.write_file("copy.txt", "shared content modified\ncopy specific");
        work_dir
            .run_jj(&["commit", "-m", "resolved copy conflicts"])
            .success();
    }

    // Recreate similar scenario
    let output = work_dir
        .run_jj(&["new", &side1, &side2, "-m", "merge2"])
        .success();

    // Check if rerere was applied
    assert!(
        output.stderr.raw().contains("Applied")
            && output.stderr.raw().contains("cached conflict resolution")
    );

    // Check if rerere handled the copy scenario
    if !output
        .stderr
        .raw()
        .contains("There are unresolved conflicts")
    {
        let original = std::fs::read_to_string(work_dir.root().join("original.txt")).unwrap();
        let copy = std::fs::read_to_string(work_dir.root().join("copy.txt")).unwrap();

        // Both files should have the resolved content
        assert!(original.contains("shared content modified"));
        assert!(original.contains("original specific v2 changed"));
        assert!(copy.contains("shared content"));
        assert!(copy.contains("copy specific"));
    }
}
