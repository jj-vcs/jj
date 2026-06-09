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

//! Garbage collection for the repository.

use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;

use futures::StreamExt as _;
use futures::pin_mut;
use tempfile::NamedTempFile;

use crate::backend::MillisSinceEpoch;
use crate::file_util::persist_temp_file;
use crate::op_store::OpStoreError;
use crate::op_walk;
use crate::operation::Operation;

/// Configuration for garbage collection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GcSettings {
    /// How often to run automatic garbage collection, in days.
    /// Set to 0 to disable automatic garbage collection.
    pub frequency_days: i64,
    /// Minimum number of operations to keep when garbage collecting.
    pub operation_min_count: i64,
    /// Number of days after which operations are considered obsolete
    /// and can be garbage collected.
    pub operation_expiry_days: i64,
}

impl GcSettings {
    /// Returns the operation expiry as a [`Duration`].
    pub fn operation_expiry(&self) -> Duration {
        Duration::from_secs(self.operation_expiry_days as u64 * 86400)
    }

    /// Returns the cutoff time for operation expiry. Operations with
    /// end timestamps before this time are considered expired.
    pub fn operation_expiry_cutoff(&self) -> SystemTime {
        SystemTime::now() - self.operation_expiry()
    }
}

const GC_LAST_RUN_FILE: &str = "gc_last_run";

fn last_gc_run_path(repo_path: &Path) -> PathBuf {
    repo_path.join(GC_LAST_RUN_FILE)
}

/// Reads the last GC run time from the repo directory.
/// Returns `None` if the file doesn't exist or is invalid.
pub fn read_last_gc_time(repo_path: &Path) -> Option<SystemTime> {
    let path = last_gc_run_path(repo_path);
    let bytes = std::fs::read(&path).ok()?;
    let millis: i64 = std::str::from_utf8(&bytes).ok()?.trim().parse().ok()?;

    SystemTime::UNIX_EPOCH.checked_add(Duration::from_millis(millis as u64))
}

/// Writes the current time as the last GC run time to the repo directory.
pub fn write_last_gc_time(repo_path: &Path) -> std::io::Result<()> {
    let path = last_gc_run_path(repo_path);
    let parent = path.parent().unwrap();
    let temp_file = NamedTempFile::new_in(parent)?;

    let now = SystemTime::now();
    let millis = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    temp_file
        .as_file()
        .write_all(millis.to_string().as_bytes())?;

    persist_temp_file(temp_file, &path)?;

    Ok(())
}

/// Checks whether automatic garbage collection should be run based on the
/// configured frequency and the last time GC was run.
pub fn should_run_auto_gc(repo_path: &Path, settings: &GcSettings) -> bool {
    if settings.frequency_days == 0 {
        return false;
    }

    let frequency = Duration::from_secs(settings.frequency_days as u64 * 86400);
    let last_gc_time = read_last_gc_time(repo_path);

    match last_gc_time {
        None => true,
        Some(last_time) => {
            let now = SystemTime::now();
            now.duration_since(last_time).unwrap_or_default() >= frequency
        }
    }
}

/// Finds the operation that should be used as the cutoff point for abandonment
/// during garbage collection.
///
/// Walks the operation history from the head backwards. The first operation
/// with index (distance from head) greater than `operation_min_count` whose
/// TTL has expired (end time is older than `expiry_cutoff`) becomes the
/// discard point. All operations from root up to and including this
/// operation would be abandoned.
pub async fn find_discard_op(
    head_ops: &[Operation],
    operation_min_count: i64,
    expiry_cutoff: SystemTime,
) -> Result<Option<Operation>, OpStoreError> {
    let expiry_cutoff_millis = expiry_cutoff
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let expiry_cutoff = MillisSinceEpoch(expiry_cutoff_millis);

    let mut count: i64 = 0;
    let stream = op_walk::walk_ancestors(head_ops);
    pin_mut!(stream);

    while let Some(op) = stream.next().await {
        let op = op?;
        count += 1;
        if count > operation_min_count && op.metadata().time.end.timestamp <= expiry_cutoff {
            return Ok(Some(op));
        }
    }

    Ok(None)
}
