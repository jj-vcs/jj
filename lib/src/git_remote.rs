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

use std::io;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use bstr::ByteSlice as _;
use gix::error::ResultExt as _;
use gix::progress::Count;
use gix::progress::Id;
use gix::progress::MessageLevel;
use gix::progress::NestedProgress;
use gix::progress::Progress;
use gix::progress::StepShared;
use gix::progress::Unit;

/// Handles progress and sideband messages from Git remote operations.
pub trait GitRemoteCallback {
    /// Whether to request progress information.
    fn needs_progress(&self) -> bool;

    /// Progress of local and remote operations.
    fn progress(&mut self, progress: &GitProgress) -> io::Result<()>;

    /// Single-line sideband message received from remote.
    fn remote_sideband(
        &mut self,
        message: &[u8],
        term: Option<GitSidebandLineTerminator>,
    ) -> io::Result<()>;
}

/// Newline character that terminates sideband message line.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum GitSidebandLineTerminator {
    /// CR to remain on the same line.
    Cr = b'\r',
    /// LF to move to the next line.
    Lf = b'\n',
}

impl GitSidebandLineTerminator {
    /// Returns byte representation.
    pub fn as_byte(self) -> u8 {
        self as u8
    }
}

/// Progress of a Git remote operation.
#[derive(Clone, Debug, Default)]
pub struct GitProgress {
    /// `(frac, total)` of "Resolving deltas".
    pub deltas: (u64, u64),
    /// `(frac, total)` of "Receiving objects".
    pub objects: (u64, u64),
    /// `(frac, total)` of remote "Counting objects".
    pub counted_objects: (u64, u64),
    /// `(frac, total)` of remote "Compressing objects".
    pub compressed_objects: (u64, u64),
}

// TODO: maybe let callers print each field separately and remove overall()?
impl GitProgress {
    /// Overall progress normalized to 0 to 1 range.
    pub fn overall(&self) -> f32 {
        if self.total() != 0 {
            self.fraction() as f32 / self.total() as f32
        } else {
            0.0
        }
    }

    fn fraction(&self) -> u64 {
        self.objects.0 + self.deltas.0 + self.counted_objects.0 + self.compressed_objects.0
    }

    fn total(&self) -> u64 {
        self.objects.1 + self.deltas.1 + self.counted_objects.1 + self.compressed_objects.1
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CounterKind {
    Objects,
    Deltas,
}

enum Event {
    Counter(CounterKind, StepShared, Option<usize>),
    Message { is_error: bool, bytes: Vec<u8> },
}

/// Runs a native operation while keeping application callbacks on the caller's
/// thread. Counts are shared atomically; only messages and counter setup use the
/// bounded channel, so transfers and verbose servers cannot queue unlimited UI work.
pub(crate) fn run_with_progress<T: Send>(
    callback: &mut dyn GitRemoteCallback,
    operation: impl FnOnce(RemoteProgress, RemoteSideband, &AtomicBool) -> gix::Result<T> + Send,
) -> gix::Result<T> {
    let needs_progress = callback.needs_progress();
    let interrupt = AtomicBool::new(false);
    let (sender, receiver) = mpsc::sync_channel(16);
    let progress = RemoteProgress::new(sender.clone(), "remote operation".into(), *b"JJRO");
    let sideband = RemoteSideband { sender };
    thread::scope(|scope| {
        let worker = scope.spawn(|| operation(progress, sideband, &interrupt));
        let mut counters: Vec<(CounterKind, StepShared, Option<usize>)> = Vec::new();
        let mut lines = SidebandLines::default();
        let mut callback_error = None;
        let mut next_report = Instant::now();
        loop {
            match receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(Event::Counter(kind, counter, max)) => {
                    counters.retain(|(previous, _, _)| *previous != kind);
                    counters.push((kind, counter, max));
                }
                Ok(Event::Message { is_error, bytes }) if callback_error.is_none() => {
                    callback_error = lines.push(is_error, &bytes, callback).err();
                }
                Ok(Event::Message { .. }) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            if callback_error.is_none() && needs_progress && Instant::now() >= next_report {
                callback_error = callback
                    .progress(&snapshot(&counters, &lines.progress))
                    .err();
                next_report = Instant::now() + Duration::from_millis(50);
            }
            if callback_error.is_some() {
                interrupt.store(true, Ordering::Relaxed);
            }
        }
        if callback_error.is_none() {
            callback_error = lines.finish(callback).err();
        }
        if callback_error.is_none() && needs_progress {
            callback_error = callback
                .progress(&snapshot(&counters, &lines.progress))
                .err();
        }
        let result = worker
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        if let Some(error) = callback_error {
            return Err(error).or_error();
        }
        result
    })
}

fn snapshot(
    counters: &[(CounterKind, StepShared, Option<usize>)],
    remote: &GitProgress,
) -> GitProgress {
    let mut progress = remote.clone();
    for (kind, counter, max) in counters {
        let Some(max) = max else { continue };
        let value = (counter.load(Ordering::Relaxed) as u64, *max as u64);
        *match kind {
            CounterKind::Objects => &mut progress.objects,
            CounterKind::Deltas => &mut progress.deltas,
        } = value;
    }
    progress
}

#[derive(Clone)]
pub(crate) struct RemoteSideband {
    sender: mpsc::SyncSender<Event>,
}

impl RemoteSideband {
    pub(crate) fn send(&self, is_error: bool, bytes: &[u8]) -> io::Result<()> {
        self.sender
            .send(Event::Message {
                is_error,
                bytes: bytes.to_vec(),
            })
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "remote callback stopped"))
    }
}

#[derive(Clone)]
pub(crate) struct RemoteProgress {
    sender: mpsc::SyncSender<Event>,
    name: String,
    id: Id,
    counter: StepShared,
    max: Option<usize>,
    unit: Option<Unit>,
    kind: Option<CounterKind>,
}

impl RemoteProgress {
    fn new(sender: mpsc::SyncSender<Event>, name: String, id: Id) -> Self {
        let kind = match &id {
            b"IWIO" | b"JJRO" => Some(CounterKind::Objects),
            b"IWRO" => Some(CounterKind::Deltas),
            _ => None,
        };
        Self {
            sender,
            name,
            id,
            counter: Arc::new(AtomicUsize::new(0)),
            max: None,
            unit: None,
            kind,
        }
    }

    fn register_counter(&self) {
        if let Some(kind) = self.kind {
            drop(
                self.sender
                    .send(Event::Counter(kind, self.counter.clone(), self.max)),
            );
        }
    }
}

impl Count for RemoteProgress {
    fn set(&self, step: usize) {
        self.counter.store(step, Ordering::Relaxed);
    }
    fn step(&self) -> usize {
        self.counter.load(Ordering::Relaxed)
    }
    fn inc_by(&self, step: usize) {
        self.counter.fetch_add(step, Ordering::Relaxed);
    }
    fn counter(&self) -> StepShared {
        self.counter.clone()
    }
}

impl Progress for RemoteProgress {
    fn init(&mut self, max: Option<usize>, unit: Option<Unit>) {
        self.max = max;
        self.unit = unit;
        self.counter = Arc::new(AtomicUsize::new(0));
        self.register_counter();
    }

    fn unit(&self) -> Option<Unit> {
        self.unit.clone()
    }
    fn max(&self) -> Option<usize> {
        self.max
    }
    fn set_max(&mut self, max: Option<usize>) -> Option<usize> {
        let previous = std::mem::replace(&mut self.max, max);
        self.register_counter();
        previous
    }
    fn set_name(&mut self, name: String) {
        self.name = name;
    }
    fn name(&self) -> Option<String> {
        Some(self.name.clone())
    }
    fn id(&self) -> Id {
        self.id
    }
    fn message(&self, _level: MessageLevel, _message: String) {}
}

impl NestedProgress for RemoteProgress {
    type SubProgress = Self;
    fn add_child(&mut self, name: impl Into<String>) -> Self {
        self.add_child_with_id(name, gix::progress::UNKNOWN)
    }
    fn add_child_with_id(&mut self, name: impl Into<String>, id: Id) -> Self {
        Self::new(self.sender.clone(), name.into(), id)
    }
}

#[derive(Default)]
struct SidebandLines {
    messages: Vec<u8>,
    errors: Vec<u8>,
    progress: GitProgress,
}

impl SidebandLines {
    fn push(
        &mut self,
        is_error: bool,
        bytes: &[u8],
        callback: &mut dyn GitRemoteCallback,
    ) -> io::Result<()> {
        let buffer = if is_error {
            &mut self.errors
        } else {
            &mut self.messages
        };
        buffer.extend_from_slice(bytes);
        Self::emit(buffer, is_error, false, &mut self.progress, callback)
    }

    fn emit(
        buffer: &mut Vec<u8>,
        is_error: bool,
        finished: bool,
        progress: &mut GitProgress,
        callback: &mut dyn GitRemoteCallback,
    ) -> io::Result<()> {
        while let Some(end) = buffer.iter().position(|byte| matches!(byte, b'\n' | b'\r')) {
            // A CR at a packet boundary may be the first half of CRLF.
            if buffer[end] == b'\r' && end + 1 == buffer.len() && !finished {
                break;
            }
            let (term, consumed) = match &buffer[end..] {
                [b'\r', b'\n', ..] => (GitSidebandLineTerminator::Lf, 2),
                [b'\n', ..] => (GitSidebandLineTerminator::Lf, 1),
                _ => (GitSidebandLineTerminator::Cr, 1),
            };
            if is_error || !update_remote_progress(&buffer[..end], progress) {
                callback.remote_sideband(&buffer[..end], Some(term))?;
            }
            buffer.drain(..end + consumed);
        }
        if finished && !buffer.is_empty() {
            if is_error || !update_remote_progress(buffer, progress) {
                callback.remote_sideband(buffer, None)?;
            }
            buffer.clear();
        }
        Ok(())
    }

    fn finish(&mut self, callback: &mut dyn GitRemoteCallback) -> io::Result<()> {
        Self::emit(
            &mut self.messages,
            false,
            true,
            &mut self.progress,
            callback,
        )?;
        Self::emit(&mut self.errors, true, true, &mut self.progress, callback)
    }
}

fn update_remote_progress(line: &[u8], progress: &mut GitProgress) -> bool {
    let Some(parsed) = gix::protocol::RemoteProgress::from_bytes(line) else {
        return false;
    };
    let counter = match parsed.action.as_bytes() {
        b"Counting objects" | b"Enumerating objects" => &mut progress.counted_objects,
        b"Compressing objects" => &mut progress.compressed_objects,
        b"Receiving objects" | b"Writing objects" => &mut progress.objects,
        b"Resolving deltas" => &mut progress.deltas,
        _ => return false,
    };
    if let (Some(step), Some(max)) = (parsed.step, parsed.max) {
        *counter = (step as u64, max as u64);
    }
    true
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    struct Callback {
        thread: thread::ThreadId,
        messages: Rc<RefCell<Vec<Vec<u8>>>>,
        progress: Vec<GitProgress>,
        fail: bool,
    }

    impl GitRemoteCallback for Callback {
        fn needs_progress(&self) -> bool {
            true
        }
        fn progress(&mut self, progress: &GitProgress) -> io::Result<()> {
            assert_eq!(thread::current().id(), self.thread);
            self.progress.push(progress.clone());
            Ok(())
        }
        fn remote_sideband(
            &mut self,
            bytes: &[u8],
            term: Option<GitSidebandLineTerminator>,
        ) -> io::Result<()> {
            assert_eq!(thread::current().id(), self.thread);
            if self.fail {
                return Err(io::Error::other("callback failed"));
            }
            let mut bytes = bytes.to_vec();
            if let Some(term) = term {
                bytes.push(term.as_byte());
            }
            self.messages.borrow_mut().push(bytes);
            Ok(())
        }
    }

    #[test]
    fn callbacks_stay_on_the_caller_thread_and_preserve_sideband_fragments() -> gix::Result<()> {
        let caller = thread::current().id();
        let mut callback = Callback {
            thread: caller,
            messages: Rc::default(),
            progress: Vec::new(),
            fail: false,
        };
        run_with_progress(&mut callback, move |mut progress, sideband, _| {
            assert_ne!(thread::current().id(), caller);
            let mut objects = progress.add_child_with_id("objects", *b"IWIO");
            objects.init(Some(3), None);
            objects.counter().fetch_add(3, Ordering::Relaxed);
            sideband.send(false, b"first ").or_error()?;
            sideband.send(false, b"line\nsecond").or_error()?;
            Ok(())
        })?;
        assert_eq!(
            *callback.messages.borrow(),
            [b"first line\n".to_vec(), b"second".to_vec()]
        );
        assert_eq!(
            callback.progress.last().expect("final progress").objects,
            (3, 3)
        );
        Ok(())
    }

    #[test]
    fn callback_errors_interrupt_the_operation_and_are_returned() {
        let observed = Arc::new(AtomicBool::new(false));
        let mut callback = Callback {
            thread: thread::current().id(),
            messages: Rc::default(),
            progress: Vec::new(),
            fail: true,
        };
        let result = run_with_progress(&mut callback, |_, sideband, interrupt| {
            sideband.send(false, b"stop\n").or_error()?;
            let deadline = Instant::now() + Duration::from_secs(5);
            while !interrupt.load(Ordering::Relaxed) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
            observed.store(interrupt.load(Ordering::Relaxed), Ordering::Relaxed);
            Ok(())
        });
        assert!(observed.load(Ordering::Relaxed));
        assert_eq!(
            result.expect_err("callback error").to_string(),
            "callback failed"
        );
    }

    #[test]
    fn mixed_sideband_packets_preserve_messages_and_filter_only_progress() -> gix::Result<()> {
        let mut callback = Callback {
            thread: thread::current().id(),
            messages: Rc::default(),
            progress: Vec::new(),
            fail: false,
        };
        run_with_progress(&mut callback, |_, sideband, _| {
            sideband
                .send(
                    false,
                    b"Counting objects: 100% (3/3), done.\r\nmessage \xff\0\r",
                )
                .or_error()?;
            sideband
                .send(
                    false,
                    b"\ncustom: 12\nCompressing objects: 50% (1/2)\rfinal",
                )
                .or_error()?;
            sideband.send(true, b"Receiving objects: 42\n").or_error()?;
            Ok(())
        })?;
        assert_eq!(
            *callback.messages.borrow(),
            [
                b"message \xff\0\n".to_vec(),
                b"custom: 12\n".to_vec(),
                b"Receiving objects: 42\n".to_vec(),
                b"final".to_vec(),
            ]
        );
        let progress = callback.progress.last().expect("final progress");
        assert_eq!(progress.counted_objects, (3, 3));
        assert_eq!(progress.compressed_objects, (1, 2));
        Ok(())
    }
}
