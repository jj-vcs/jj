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

//! Resolution cache for automatically reusing conflict resolutions (rerere).

use std::fs;
use std::io;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;

use blake2::Blake2b512;
use blake2::Digest as _;
use bstr::BString;
use tempfile::NamedTempFile;
use tracing::debug;

use crate::backend::BackendError;
use crate::backend::BackendResult;
use crate::conflicts::MaterializedFileConflictValue;
use crate::file_util::persist_content_addressed_temp_file;
use crate::merge::Merge;
use crate::repo_path::RepoPath;

/// Result of a resolution cache operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionCacheResult {
    /// A new conflict preimage was recorded.
    RecordedPreimage,
    /// A resolution was recorded.
    RecordedResolution,
    /// A cached resolution was applied.
    AppliedCachedResolution,
    /// No action was taken.
    NoAction,
}

/// Statistics about resolution cache operations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolutionCacheStats {
    /// Number of new conflict preimages recorded.
    pub recorded_preimages: usize,
    /// Number of resolutions recorded.
    pub recorded_resolutions: usize,
    /// Number of cached resolutions applied.
    pub applied_cached_resolutions: usize,
}

impl ResolutionCacheStats {
    /// Create a new empty statistics instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a resolution cache operation result to the statistics.
    pub fn add(&mut self, result: ResolutionCacheResult) {
        match result {
            ResolutionCacheResult::RecordedPreimage => self.recorded_preimages += 1,
            ResolutionCacheResult::RecordedResolution => self.recorded_resolutions += 1,
            ResolutionCacheResult::AppliedCachedResolution => self.applied_cached_resolutions += 1,
            ResolutionCacheResult::NoAction => {}
        }
    }

    /// Merge statistics from another instance into this one.
    pub fn merge(&mut self, other: &ResolutionCacheStats) {
        self.recorded_preimages += other.recorded_preimages;
        self.recorded_resolutions += other.recorded_resolutions;
        self.applied_cached_resolutions += other.applied_cached_resolutions;
    }

    /// Check if all statistics are zero.
    pub fn is_empty(&self) -> bool {
        self.recorded_preimages == 0
            && self.recorded_resolutions == 0
            && self.applied_cached_resolutions == 0
    }
}

/// Unique identifier for a conflict based on its normalized content.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConflictId([u8; 64]);

impl ConflictId {
    /// Creates a ConflictId from raw bytes.
    pub fn from_bytes(bytes: [u8; 64]) -> Self {
        ConflictId(bytes)
    }

    /// Returns the hex representation of the conflict ID.
    pub fn hex(&self) -> String {
        crate::hex_util::encode_hex(&self.0)
    }
}

/// Cache for storing and retrieving conflict resolutions.
pub struct ResolutionCache {
    store_path: PathBuf,
    enabled: bool,
}

impl ResolutionCache {
    /// Creates a new ResolutionCache with the given store path.
    pub fn new(store_path: PathBuf, enabled: bool) -> Self {
        ResolutionCache {
            store_path,
            enabled,
        }
    }

    /// Records a resolution for a given conflict.
    pub fn record_resolution(
        &self,
        path: &RepoPath,
        conflict: &MaterializedFileConflictValue,
        resolution: &[u8],
    ) -> BackendResult<ResolutionCacheResult> {
        if !self.enabled {
            return Ok(ResolutionCacheResult::NoAction);
        }

        let conflict_id = self.calculate_conflict_id(path, &conflict.contents)?;
        let conflict_dir = self.store_path.join(conflict_id.hex());

        // Check if this is a new conflict (preimage doesn't exist yet)
        let is_new_conflict = !conflict_dir.exists();

        if is_new_conflict {
            // Ensure the parent directory exists
            fs::create_dir_all(&self.store_path).map_err(|e| {
                BackendError::Other(
                    format!("Failed to create resolution cache directory: {e}").into(),
                )
            })?;

            // Create a temporary directory for atomic creation
            let temp_dir = tempfile::Builder::new()
                .prefix(&format!(".tmp-{}-", conflict_id.hex()))
                .tempdir_in(&self.store_path)
                .map_err(|e| {
                    BackendError::Other(format!("Failed to create temp directory: {e}").into())
                })?;

            // Write normalized conflict
            let conflict_path = temp_dir.path().join("conflict");
            let normalized_conflict = self.normalize_conflict(&conflict.contents)?;
            fs::write(&conflict_path, &normalized_conflict).map_err(|e| {
                BackendError::Other(format!("Failed to write conflict file: {e}").into())
            })?;

            // Write resolution
            let resolution_path = temp_dir.path().join("resolution");
            fs::write(&resolution_path, resolution).map_err(|e| {
                BackendError::Other(format!("Failed to write resolution file: {e}").into())
            })?;

            // Atomically rename the temp directory to the final location
            match fs::rename(temp_dir.path(), &conflict_dir) {
                Ok(()) => {
                    // Successfully created the directory
                    let _ = temp_dir.keep();
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    // Another process created it first, update the resolution file only
                    let resolution_path = conflict_dir.join("resolution");
                    let mut temp_file = NamedTempFile::new_in(&conflict_dir).map_err(|e| {
                        BackendError::Other(
                            format!("Failed to create temp file for resolution: {e}").into(),
                        )
                    })?;
                    temp_file.write_all(resolution).map_err(|e| {
                        BackendError::Other(
                            format!("Failed to write resolution data to temp file: {e}").into(),
                        )
                    })?;
                    persist_content_addressed_temp_file(temp_file, &resolution_path).map_err(
                        |e| {
                            BackendError::Other(
                                format!("Failed to persist resolution file: {e}").into(),
                            )
                        },
                    )?;
                }
                Err(e) => {
                    return Err(BackendError::Other(
                        format!("Failed to rename temp directory: {e}").into(),
                    ));
                }
            }
        } else {
            // Directory already exists, check if resolution has changed
            let resolution_path = conflict_dir.join("resolution");

            // Read existing resolution to compare
            if let Ok(existing_resolution) = fs::read(&resolution_path) {
                if existing_resolution == resolution {
                    debug!(path = ?path, conflict_id = %conflict_id.hex(), "Resolution unchanged");
                    return Ok(ResolutionCacheResult::NoAction);
                }
            }

            // Resolution has changed or didn't exist, update it atomically
            let mut temp_file = NamedTempFile::new_in(&conflict_dir).map_err(|e| {
                BackendError::Other(
                    format!("Failed to create temp file for resolution: {e}").into(),
                )
            })?;
            temp_file.write_all(resolution).map_err(|e| {
                BackendError::Other(
                    format!("Failed to write resolution data to temp file: {e}").into(),
                )
            })?;
            persist_content_addressed_temp_file(temp_file, &resolution_path).map_err(|e| {
                BackendError::Other(format!("Failed to persist resolution file: {e}").into())
            })?;
        }

        let result = if is_new_conflict {
            debug!(path = ?path, conflict_id = %conflict_id.hex(), "Recorded conflict preimage");
            ResolutionCacheResult::RecordedPreimage
        } else {
            debug!(path = ?path, conflict_id = %conflict_id.hex(), "Recorded resolution");
            ResolutionCacheResult::RecordedResolution
        };
        Ok(result)
    }

    /// Retrieves a resolution for a given conflict if one exists.
    pub fn get_resolution(
        &self,
        path: &RepoPath,
        conflict: &MaterializedFileConflictValue,
    ) -> BackendResult<Option<Vec<u8>>> {
        if !self.enabled {
            return Ok(None);
        }

        let conflict_id = self.calculate_conflict_id(path, &conflict.contents)?;
        let resolution_path = self.store_path.join(conflict_id.hex()).join("resolution");

        match fs::read(&resolution_path) {
            Ok(resolution) => {
                debug!(path = ?path, conflict_id = %conflict_id.hex(), "Found cached resolution");
                // Update mtime to keep the resolution from being garbage collected
                if let Ok(file) = fs::OpenOptions::new().write(true).open(&resolution_path) {
                    if let Err(e) = file.set_modified(std::time::SystemTime::now()) {
                        tracing::debug!("Failed to update resolution mtime: {e}");
                    }
                }
                Ok(Some(resolution))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(BackendError::Other(
                format!("Failed to read resolution file: {e}").into(),
            )),
        }
    }

    /// Retrieves a resolution for a given conflict content if one exists.
    pub fn get_resolution_for_content(
        &self,
        path: &RepoPath,
        conflict_content: &Merge<BString>,
    ) -> BackendResult<Option<Vec<u8>>> {
        if !self.enabled {
            return Ok(None);
        }

        let conflict_id = self.calculate_conflict_id(path, conflict_content)?;
        let resolution_path = self.store_path.join(conflict_id.hex()).join("resolution");

        match fs::read(&resolution_path) {
            Ok(resolution) => {
                // Update mtime to keep the resolution from being garbage collected
                if let Ok(file) = fs::OpenOptions::new().write(true).open(&resolution_path) {
                    if let Err(e) = file.set_modified(std::time::SystemTime::now()) {
                        tracing::debug!("Failed to update resolution mtime: {e}");
                    }
                }
                Ok(Some(resolution))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(BackendError::Other(
                format!("Failed to read resolution file: {e}").into(),
            )),
        }
    }

    /// Calculates a unique ID for a conflict based on its normalized content.
    fn calculate_conflict_id(
        &self,
        _path: &RepoPath,
        conflict_content: &Merge<BString>,
    ) -> BackendResult<ConflictId> {
        let normalized = self.normalize_conflict(conflict_content)?;
        let mut hasher = Blake2b512::new();
        hasher.update(&normalized);
        let hash = hasher.finalize();
        let mut id_bytes = [0u8; 64];
        id_bytes.copy_from_slice(&hash);
        Ok(ConflictId(id_bytes))
    }

    /// Normalizes a conflict for consistent hashing.
    /// This strips context-specific information and sorts conflict sides.
    fn normalize_conflict(&self, conflict_content: &Merge<BString>) -> BackendResult<Vec<u8>> {
        let mut normalized = Vec::new();

        // Write the number of sides
        writeln!(&mut normalized, "CONFLICT:{}", conflict_content.num_sides()).map_err(|e| {
            BackendError::Other(format!("Failed to write conflict header: {e}").into())
        })?;

        // Sort the conflict sides by content for consistent hashing
        let mut sides: Vec<_> = conflict_content.iter().collect();
        sides.sort_by_key(|content| content.as_slice());

        for content in sides {
            writeln!(&mut normalized, "SIDE_START").map_err(|e| {
                BackendError::Other(format!("Failed to write side marker: {e}").into())
            })?;
            normalized.write_all(content).map_err(|e| {
                BackendError::Other(format!("Failed to write side content: {e}").into())
            })?;
            writeln!(&mut normalized, "SIDE_END").map_err(|e| {
                BackendError::Other(format!("Failed to write side end marker: {e}").into())
            })?;
        }

        Ok(normalized)
    }

    /// Clears all cached resolutions.
    pub fn clear(&self) -> io::Result<()> {
        if Path::new(&self.store_path).exists() {
            fs::remove_dir_all(&self.store_path)?;
        }
        Ok(())
    }

    /// Checks if the resolution cache is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Performs garbage collection on the resolution cache.
    /// Removes resolutions older than the given threshold.
    pub fn gc(&self, keep_newer: std::time::SystemTime) -> io::Result<()> {
        if !self.enabled || !Path::new(&self.store_path).exists() {
            return Ok(());
        }

        let mut removed_count = 0;
        let entries = fs::read_dir(&self.store_path)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }

            // Check the modification time of the resolution file
            let resolution_path = path.join("resolution");
            if let Ok(metadata) = fs::metadata(&resolution_path) {
                if let Ok(mtime) = metadata.modified() {
                    if mtime < keep_newer {
                        // Remove the entire conflict directory
                        fs::remove_dir_all(&path)?;
                        removed_count += 1;
                    }
                }
            }
        }

        tracing::info!("Removed {} old resolution cache entries", removed_count);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo_path::RepoPathBuf;

    #[test]
    fn test_conflict_normalization() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let cache = ResolutionCache::new(temp_dir.path().to_path_buf(), true);

        // Test 1: Basic consistency - same conflict should produce same normalization
        let path = RepoPathBuf::from_internal_string("test.txt").unwrap();
        let conflict_content = Merge::from_vec(vec![
            b"base content\n".to_vec().into(),
            b"left content\n".to_vec().into(),
            b"right content\n".to_vec().into(),
        ]);

        let normalized1 = cache.normalize_conflict(&conflict_content).unwrap();
        let normalized2 = cache.normalize_conflict(&conflict_content).unwrap();
        assert_eq!(normalized1, normalized2);

        // Test 2: Order independence - different order of sides should produce same
        // hash
        let conflict_reordered = Merge::from_vec(vec![
            b"base content\n".to_vec().into(),
            b"right content\n".to_vec().into(), // swapped with left
            b"left content\n".to_vec().into(),
        ]);

        let id1 = cache
            .calculate_conflict_id(path.as_ref(), &conflict_content)
            .unwrap();
        let id2 = cache
            .calculate_conflict_id(path.as_ref(), &conflict_reordered)
            .unwrap();
        assert_eq!(
            id1.hex(),
            id2.hex(),
            "Reordered conflicts should have same ID"
        );

        // Test 3: Different conflicts should produce different IDs
        let different_conflict = Merge::from_vec(vec![
            b"base content\n".to_vec().into(),
            b"completely different left\n".to_vec().into(),
            b"completely different right\n".to_vec().into(),
        ]);

        let id3 = cache
            .calculate_conflict_id(path.as_ref(), &different_conflict)
            .unwrap();
        assert_ne!(
            id1.hex(),
            id3.hex(),
            "Different conflicts should have different IDs"
        );

        // Test 4: Multi-way conflicts
        // Note: Merge stores values in alternating add/remove pattern
        // For a 3-way merge, we have 5 elements total
        let multiway_conflict = Merge::from_vec(vec![
            b"base".to_vec().into(),
            b"change1".to_vec().into(),
            b"change2".to_vec().into(),
            b"change3".to_vec().into(),
            b"change4".to_vec().into(),
        ]);

        let normalized_multiway = cache.normalize_conflict(&multiway_conflict).unwrap();
        // With 5 elements, num_sides = 5/2 + 1 = 3
        let normalized_str = std::str::from_utf8(&normalized_multiway).unwrap();
        assert!(
            normalized_str.starts_with("CONFLICT:3\n"),
            "Expected normalized conflict to start with 'CONFLICT:3\\n', but got: {normalized_str}"
        );

        // Also verify we have all 5 sides in the normalized output
        assert!(normalized_str.contains("base"));
        assert!(normalized_str.contains("change1"));
        assert!(normalized_str.contains("change2"));
        assert!(normalized_str.contains("change3"));
        assert!(normalized_str.contains("change4"));

        // Test 5: Conflicts with empty sides
        let conflict_with_empty = Merge::from_vec(vec![
            b"".to_vec().into(),
            b"left content".to_vec().into(),
            b"".to_vec().into(),
        ]);

        let normalized_empty = cache.normalize_conflict(&conflict_with_empty).unwrap();
        // Should handle empty sides gracefully
        let normalized_str = std::str::from_utf8(&normalized_empty).unwrap();
        assert!(normalized_str.contains("SIDE_START\nSIDE_END"));

        // Test 6: Path independence - same conflict in different files should have same
        // ID
        let different_path = RepoPathBuf::from_internal_string("other/file.txt").unwrap();
        let id_path1 = cache
            .calculate_conflict_id(path.as_ref(), &conflict_content)
            .unwrap();
        let id_path2 = cache
            .calculate_conflict_id(different_path.as_ref(), &conflict_content)
            .unwrap();
        assert_eq!(
            id_path1.hex(),
            id_path2.hex(),
            "Same conflict in different files should have same ID"
        );
    }

    #[test]
    fn test_record_and_retrieve_resolution() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let cache = ResolutionCache::new(temp_dir.path().to_path_buf(), true);

        let path = RepoPathBuf::from_internal_string("test.txt").unwrap();
        let conflict_content = Merge::from_vec(vec![
            b"base".to_vec().into(),
            b"left".to_vec().into(),
            b"right".to_vec().into(),
        ]);

        let conflict = MaterializedFileConflictValue {
            unsimplified_ids: Merge::from_vec(vec![None, None, None]),
            ids: Merge::from_vec(vec![None, None, None]),
            contents: conflict_content.clone(),
            executable: Some(false),
            copy_id: None,
        };

        let resolution = b"resolved content";

        // Record resolution
        let result = cache
            .record_resolution(path.as_ref(), &conflict, resolution)
            .unwrap();
        // First time recording should record both preimage and resolution
        assert_eq!(result, ResolutionCacheResult::RecordedPreimage);

        // Retrieve resolution
        let retrieved = cache.get_resolution(path.as_ref(), &conflict).unwrap();
        assert_eq!(retrieved, Some(resolution.to_vec()));

        // Also test retrieval with just content
        let retrieved2 = cache
            .get_resolution_for_content(path.as_ref(), &conflict_content)
            .unwrap();
        assert_eq!(retrieved2, Some(resolution.to_vec()));
    }

    #[test]
    fn test_disabled_cache() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let cache = ResolutionCache::new(temp_dir.path().to_path_buf(), false);

        let path = RepoPathBuf::from_internal_string("test.txt").unwrap();
        let conflict_content = Merge::from_vec(vec![
            b"base".to_vec().into(),
            b"left".to_vec().into(),
            b"right".to_vec().into(),
        ]);

        let conflict = MaterializedFileConflictValue {
            unsimplified_ids: Merge::from_vec(vec![None, None, None]),
            ids: Merge::from_vec(vec![None, None, None]),
            contents: conflict_content,
            executable: Some(false),
            copy_id: None,
        };

        let resolution = b"resolved content";

        // Should not record when disabled
        let result = cache
            .record_resolution(path.as_ref(), &conflict, resolution)
            .unwrap();
        assert_eq!(result, ResolutionCacheResult::NoAction);

        // Should not retrieve when disabled
        let retrieved = cache.get_resolution(path.as_ref(), &conflict).unwrap();
        assert_eq!(retrieved, None);
    }

    #[test]
    fn test_garbage_collection() {
        use std::thread;
        use std::time::Duration;

        let temp_dir = tempfile::TempDir::new().unwrap();
        let cache = ResolutionCache::new(temp_dir.path().to_path_buf(), true);

        let path = RepoPathBuf::from_internal_string("test.txt").unwrap();
        let conflict_content = Merge::from_vec(vec![
            b"base".to_vec().into(),
            b"left".to_vec().into(),
            b"right".to_vec().into(),
        ]);

        let conflict = MaterializedFileConflictValue {
            unsimplified_ids: Merge::from_vec(vec![None, None, None]),
            ids: Merge::from_vec(vec![None, None, None]),
            contents: conflict_content.clone(),
            executable: Some(false),
            copy_id: None,
        };

        let resolution = b"resolved content";

        // Record a resolution
        let result = cache
            .record_resolution(path.as_ref(), &conflict, resolution)
            .unwrap();
        assert_eq!(result, ResolutionCacheResult::RecordedPreimage);

        // Verify it exists
        let retrieved = cache.get_resolution(path.as_ref(), &conflict).unwrap();
        assert_eq!(retrieved, Some(resolution.to_vec()));

        // GC with a threshold in the past - should keep the resolution (it's newer than
        // threshold)
        let past_time = std::time::SystemTime::now() - Duration::from_secs(3600);
        cache.gc(past_time).unwrap();

        // Resolution should still exist
        let retrieved = cache.get_resolution(path.as_ref(), &conflict).unwrap();
        assert_eq!(retrieved, Some(resolution.to_vec()));

        // Wait a bit to ensure the file times are different
        thread::sleep(Duration::from_millis(10));

        // GC with future time - should remove the resolution (it's older than
        // threshold)
        let future_time = std::time::SystemTime::now() + Duration::from_secs(1);
        cache.gc(future_time).unwrap();

        // Resolution should be gone
        let retrieved = cache.get_resolution(path.as_ref(), &conflict).unwrap();
        assert_eq!(retrieved, None);

        // Test that using a resolution updates its mtime and prevents GC
        let result = cache
            .record_resolution(path.as_ref(), &conflict, resolution)
            .unwrap();
        assert_eq!(result, ResolutionCacheResult::RecordedPreimage);

        // Wait to ensure time passes
        thread::sleep(Duration::from_millis(50));

        // Set up a GC threshold that would remove the resolution based on original
        // mtime
        let gc_threshold = std::time::SystemTime::now() - Duration::from_millis(25);

        // Use the resolution (should update mtime)
        let retrieved = cache.get_resolution(path.as_ref(), &conflict).unwrap();
        assert_eq!(retrieved, Some(resolution.to_vec()));

        // Run GC with threshold that would have removed old file
        cache.gc(gc_threshold).unwrap();

        // Resolution should still exist because mtime was updated
        let retrieved = cache.get_resolution(path.as_ref(), &conflict).unwrap();
        assert_eq!(retrieved, Some(resolution.to_vec()));
    }

    #[test]
    fn test_cross_platform_path_handling() {
        let test_dir = tempfile::tempdir().unwrap();
        let cache = ResolutionCache::new(test_dir.path().to_path_buf(), true);

        let resolution = b"resolved content";

        // Test various path formats that might appear on different platforms
        let test_paths = [
            "dir/file.txt",
            "dir\\file.txt", // Windows-style
            "./dir/file.txt",
            ".\\dir\\file.txt",
            "nested/dir/file.txt",
            "nested\\dir\\file.txt",
        ];

        // Record resolutions with different path formats
        for (i, path_str) in test_paths.iter().enumerate() {
            let path = RepoPathBuf::from_internal_string(path_str.to_string()).unwrap();

            // Make each conflict slightly different to avoid hash collisions
            let conflict_content = Merge::from_vec(vec![
                b"base content\n".to_vec().into(),
                format!("left content {i}\n").as_bytes().to_vec().into(),
                b"right content\n".to_vec().into(),
            ]);

            let conflict = MaterializedFileConflictValue {
                unsimplified_ids: Merge::from_vec(vec![None, None, None]),
                ids: Merge::from_vec(vec![None, None, None]),
                contents: conflict_content,
                executable: Some(false),
                copy_id: None,
            };

            let result = cache
                .record_resolution(path.as_ref(), &conflict, resolution)
                .unwrap();
            assert_eq!(result, ResolutionCacheResult::RecordedPreimage);
        }

        // Verify resolutions can be retrieved
        for (i, path_str) in test_paths.iter().enumerate() {
            let path = RepoPathBuf::from_internal_string(path_str.to_string()).unwrap();

            // Recreate the same conflict variant
            let conflict_content = Merge::from_vec(vec![
                b"base content\n".to_vec().into(),
                format!("left content {i}\n").as_bytes().to_vec().into(),
                b"right content\n".to_vec().into(),
            ]);

            let conflict = MaterializedFileConflictValue {
                unsimplified_ids: Merge::from_vec(vec![None, None, None]),
                ids: Merge::from_vec(vec![None, None, None]),
                contents: conflict_content,
                executable: Some(false),
                copy_id: None,
            };

            let retrieved = cache.get_resolution(path.as_ref(), &conflict).unwrap();
            assert_eq!(retrieved, Some(resolution.to_vec()));
        }

        // Test Unicode paths
        let unicode_paths = [
            "ファイル.txt", // Japanese
            "文件.txt",     // Chinese
            "файл.txt",     // Russian
            "αρχείο.txt",   // Greek
        ];

        for (i, path_str) in unicode_paths.iter().enumerate() {
            let path = RepoPathBuf::from_internal_string(path_str.to_string()).unwrap();

            // Make each conflict unique to avoid reusing cached preimages
            let conflict_content = Merge::from_vec(vec![
                b"base unicode\n".to_vec().into(),
                format!("left unicode {i}\n").as_bytes().to_vec().into(),
                b"right unicode\n".to_vec().into(),
            ]);

            let conflict = MaterializedFileConflictValue {
                unsimplified_ids: Merge::from_vec(vec![None, None, None]),
                ids: Merge::from_vec(vec![None, None, None]),
                contents: conflict_content,
                executable: Some(false),
                copy_id: None,
            };

            let result = cache
                .record_resolution(path.as_ref(), &conflict, resolution)
                .unwrap();
            assert_eq!(result, ResolutionCacheResult::RecordedPreimage);

            let retrieved = cache.get_resolution(path.as_ref(), &conflict).unwrap();
            assert_eq!(retrieved, Some(resolution.to_vec()));
        }

        // Test paths with spaces and special characters
        let special_paths = [
            "my file.txt",
            "file with spaces.txt",
            "file-with-dashes.txt",
            "file_with_underscores.txt",
            "file.multiple.dots.txt",
        ];

        for (i, path_str) in special_paths.iter().enumerate() {
            let path = RepoPathBuf::from_internal_string(path_str.to_string()).unwrap();

            // Make each conflict unique to avoid reusing cached preimages
            let conflict_content = Merge::from_vec(vec![
                b"base special\n".to_vec().into(),
                format!("left special {i}\n").as_bytes().to_vec().into(),
                b"right special\n".to_vec().into(),
            ]);

            let conflict = MaterializedFileConflictValue {
                unsimplified_ids: Merge::from_vec(vec![None, None, None]),
                ids: Merge::from_vec(vec![None, None, None]),
                contents: conflict_content,
                executable: Some(false),
                copy_id: None,
            };

            let result = cache
                .record_resolution(path.as_ref(), &conflict, resolution)
                .unwrap();
            assert_eq!(result, ResolutionCacheResult::RecordedPreimage);

            let retrieved = cache.get_resolution(path.as_ref(), &conflict).unwrap();
            assert_eq!(retrieved, Some(resolution.to_vec()));
        }
    }
}
