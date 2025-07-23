use crate::common::rerere_helpers::*;

/// Test to verify if jj status triggers rerere recording
#[test]
fn test_rerere_recording_with_status() {
    let test_env = setup_rerere_test();
    let work_dir = test_env.work_dir("repo");

    // Create a simple conflict
    // Create base
    work_dir.run_jj(&["new", "root()", "-m", "base"]).success();
    let base = create_commit(&work_dir, "base", "file.txt", "base\n");

    // Create side1
    work_dir.run_jj(&["new", &base, "-m", "side1"]).success();
    let side1 = create_commit(&work_dir, "side1", "file.txt", "branch1\n");

    // Create side2
    work_dir.run_jj(&["new", &base, "-m", "side2"]).success();
    let side2 = create_commit(&work_dir, "side2", "file.txt", "branch2\n");

    // Create merge
    let merge_output = work_dir
        .run_jj(&["new", &side1, &side2, "-m", "merge"])
        .success();

    // Verify we have a conflict
    merge_output.assert_has_conflict();

    // Check the conflict markers
    let content = std::fs::read_to_string(work_dir.root().join("file.txt")).unwrap();
    assert!(content.contains("<<<<<<"));
    assert!(content.contains(">>>>>>"));

    // Resolve the conflict
    work_dir.write_file("file.txt", "resolved\n");

    // Run status - this should trigger snapshot and recording
    let status_output = work_dir.run_jj(["status"]).success();

    // Check if resolution was recorded
    let cache_dir = work_dir.root().join(".jj/repo/resolution_cache");

    // First, let's see what status says
    println!("Status output after resolving:\n{status_output}");

    // The cache directory might not exist until we commit
    // Let's commit and check
    work_dir
        .run_jj(["commit", "-m", "resolved conflict"])
        .success();

    // Now check if cache exists
    if cache_dir.exists() {
        let entries: Vec<_> = std::fs::read_dir(&cache_dir).unwrap().collect();
        println!("Cache has {} entries", entries.len());
        assert!(
            !entries.is_empty(),
            "Resolution cache should have entries after commit"
        );
    } else {
        panic!("Resolution cache directory doesn't exist even after commit");
    }

    // Create another similar conflict to test if rerere applies
    // Use the same file path but different branches
    work_dir.run_jj(["new", &base, "-m", "branch3"]).success();
    let branch3 = create_commit(&work_dir, "commit branch3", "file.txt", "branch1\n");

    work_dir.run_jj(["new", &base, "-m", "branch4"]).success();
    let branch4 = create_commit(&work_dir, "commit branch4", "file.txt", "branch2\n");

    // Create another merge - rerere should apply
    let merge2_output = work_dir
        .run_jj(["new", &branch3, &branch4, "-m", "merge2"])
        .success();

    // Check if rerere applied the cached resolution
    merge2_output.assert_rerere_applied(1);
    println!("SUCCESS: Rerere applied cached resolution!");
}
