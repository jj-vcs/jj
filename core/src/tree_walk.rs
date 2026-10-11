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

//! Utilities for walking trees concurrently.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use std::task::ready;

use futures::FutureExt as _;
use futures::Stream;
use futures::StreamExt as _;
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;

use crate::backend::BackendResult;
use crate::repo_path::RepoPath;
use crate::repo_path::RepoPathBuf;

/// A set of futures keyed by priority. At most `limit` of the futures are
/// polled concurrently. The others are queued, and the ones with the smallest
/// keys are started first when there's capacity. A started future keeps being
/// polled until it completes, even if futures with smaller keys are added
/// later.
pub(crate) struct PrioritizedFutures<'a, K, T> {
    limit: usize,
    running: FuturesUnordered<BoxFuture<'a, (K, T)>>,
    running_keys: BTreeSet<K>,
    queued: BTreeMap<K, BoxFuture<'a, T>>,
}

impl<'a, K, T> PrioritizedFutures<'a, K, T>
where
    K: Ord + Clone + Send + 'a,
    T: 'a,
{
    /// Creates an empty set that polls at most `limit` futures concurrently.
    pub fn new(limit: usize) -> Self {
        assert!(limit > 0, "The limit must be positive");
        Self {
            limit,
            running: FuturesUnordered::new(),
            running_keys: BTreeSet::new(),
            queued: BTreeMap::new(),
        }
    }

    /// Whether there are no futures that haven't completed.
    pub fn is_empty(&self) -> bool {
        self.running.is_empty() && self.queued.is_empty()
    }

    /// Adds a future. Panics if there's already an incomplete future with the
    /// same key.
    pub fn push(&mut self, key: K, future: impl Future<Output = T> + Send + 'a) {
        assert!(!self.running_keys.contains(&key), "Duplicate key");
        let old = self.queued.insert(key, future.boxed());
        assert!(old.is_none(), "Duplicate key");
    }

    /// The smallest key of the futures that haven't completed.
    pub fn first_key(&self) -> Option<&K> {
        let first_running = self.running_keys.first();
        let first_queued = self.queued.keys().next();
        first_running.into_iter().chain(first_queued).min()
    }

    /// Polls the started futures, after first starting queued futures if
    /// there's capacity. Returns the key and output of a completed future, or
    /// `None` if there are no more futures.
    pub fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<(K, T)>> {
        while self.running.len() < self.limit
            && let Some((key, future)) = self.queued.pop_first()
        {
            self.running_keys.insert(key.clone());
            self.running
                .push(future.map(move |output| (key, output)).boxed());
        }
        let next = ready!(self.running.poll_next_unpin(cx));
        if let Some((key, _)) = &next {
            self.running_keys.remove(key);
        }
        Poll::Ready(next)
    }
}

/// Decides what an `OrderedTreeWalk` emits and which paths it schedules work
/// for.
pub(crate) trait TreeWalkVisitor<'a> {
    /// The output of the work scheduled for a path, such as the trees read for
    /// a directory.
    type Work: Send + 'a;
    /// The items emitted by the walk.
    type Item;

    /// Processes the completed work for `path`. Any items emitted and work
    /// scheduled must be for paths below `path`.
    fn visit(
        &self,
        path: &RepoPath,
        work: Self::Work,
        queue: &mut TreeWalkQueue<'a, Self::Work, Self::Item>,
    );
}

/// The pending work and items of an `OrderedTreeWalk`.
pub(crate) struct TreeWalkQueue<'a, W, I> {
    work: PrioritizedFutures<'a, RepoPathBuf, BackendResult<W>>,
    /// Items that may or may not be ready to emit. An item is ready to emit
    /// unless there's pending work for a smaller path.
    items: BTreeMap<RepoPathBuf, BackendResult<I>>,
}

impl<'a, W, I> TreeWalkQueue<'a, W, I>
where
    W: 'a,
{
    /// Creates an empty queue that runs at most `max_concurrent_work`
    /// scheduled futures concurrently.
    pub fn new(max_concurrent_work: usize) -> Self {
        Self {
            work: PrioritizedFutures::new(max_concurrent_work),
            items: BTreeMap::new(),
        }
    }

    /// Adds an item to emit at `path`. There must not already be an item for
    /// the same path.
    pub fn emit(&mut self, path: RepoPathBuf, item: I) {
        let old = self.items.insert(path, Ok(item));
        debug_assert!(old.is_none(), "Duplicate item");
    }

    /// Schedules work for `path`. Paths earlier in the walk are prioritized.
    /// If the work fails, the error is emitted at `path`.
    pub fn schedule(
        &mut self,
        path: RepoPathBuf,
        work: impl Future<Output = BackendResult<W>> + Send + 'a,
    ) {
        self.work.push(path, work);
    }

    /// Whether the first item can be emitted, i.e. no pending work can result
    /// in an earlier item.
    fn first_item_ready(&self) -> bool {
        match (self.items.keys().next(), self.work.first_key()) {
            (Some(item_path), Some(work_path)) => item_path <= work_path,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }
}

/// Stream that walks trees concurrently and emits items in path order. This
/// means that items in a directory are emitted before items in later sibling
/// directories, and that an item for a directory is emitted before items
/// within it.
///
/// Work is scheduled for paths in order, so the walk makes progress on early
/// paths without first having to read all directories at each level.
pub(crate) struct OrderedTreeWalk<'a, V: TreeWalkVisitor<'a>> {
    visitor: V,
    queue: TreeWalkQueue<'a, V::Work, V::Item>,
    /// The maximum number of items in the queue. We stop polling scheduled
    /// work while we're at the limit. However, we may temporarily go over the
    /// limit because a visitor can emit any number of items, and because items
    /// may be waiting for work for earlier paths.
    max_queued_items: usize,
}

impl<'a, V: TreeWalkVisitor<'a>> OrderedTreeWalk<'a, V> {
    /// Creates a walk that starts with the given queue.
    pub fn new(visitor: V, queue: TreeWalkQueue<'a, V::Work, V::Item>) -> Self {
        Self {
            visitor,
            queue,
            max_queued_items: 10000,
        }
    }
}

impl<'a, V> Stream for OrderedTreeWalk<'a, V>
where
    V: TreeWalkVisitor<'a> + Unpin,
{
    type Item = (RepoPathBuf, BackendResult<V::Item>);

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            let ready = this.queue.first_item_ready();
            // If we can't emit an item, we must poll the scheduled work. Otherwise, nothing
            // would wake us up when the work completes.
            if (!ready || this.queue.items.len() < this.max_queued_items)
                && let Poll::Ready(Some((path, result))) = this.queue.work.poll_next(cx)
            {
                match result {
                    Ok(work) => this.visitor.visit(&path, work, &mut this.queue),
                    Err(err) => {
                        this.queue.items.insert(path, Err(err));
                    }
                }
                continue;
            }
            return if ready {
                Poll::Ready(this.queue.items.pop_first())
            } else if this.queue.work.is_empty() {
                Poll::Ready(None)
            } else {
                Poll::Pending
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;

    use futures::channel::oneshot;
    use futures::task::noop_waker_ref;

    use super::*;
    use crate::backend::BackendError;
    use crate::repo_path::RepoPathComponent;

    /// Returns a future that records `key` in `started` when first polled and
    /// then completes when the returned sender is used.
    fn controlled_future(
        key: u32,
        started: &Arc<Mutex<BTreeSet<u32>>>,
    ) -> (oneshot::Sender<()>, impl Future<Output = u32> + use<>) {
        let (sender, receiver) = oneshot::channel();
        let started = started.clone();
        let future = async move {
            started.lock().unwrap().insert(key);
            receiver.await.unwrap();
            key
        };
        (sender, future)
    }

    fn poll_next(futures: &mut PrioritizedFutures<'_, u32, u32>) -> Poll<Option<(u32, u32)>> {
        futures.poll_next(&mut Context::from_waker(noop_waker_ref()))
    }

    #[test]
    fn test_empty() {
        let mut futures = PrioritizedFutures::<u32, u32>::new(2);
        assert!(futures.is_empty());
        assert_eq!(futures.first_key(), None);
        assert_eq!(poll_next(&mut futures), Poll::Ready(None));
    }

    #[test]
    fn test_limit_and_priority() {
        let started = Arc::new(Mutex::new(BTreeSet::new()));
        let mut futures = PrioritizedFutures::new(2);
        let mut senders = BTreeMap::new();
        for key in [5, 3, 4, 1] {
            let (sender, future) = controlled_future(key, &started);
            senders.insert(key, sender);
            futures.push(key, future);
        }
        assert_eq!(futures.first_key(), Some(&1));

        // Only the two futures with the smallest keys are started
        assert_eq!(poll_next(&mut futures), Poll::Pending);
        assert_eq!(*started.lock().unwrap(), BTreeSet::from([1, 3]));

        // A future with a smaller key is queued, not started
        let (sender, future) = controlled_future(2, &started);
        senders.insert(2, sender);
        futures.push(2, future);
        assert_eq!(poll_next(&mut futures), Poll::Pending);
        assert_eq!(*started.lock().unwrap(), BTreeSet::from([1, 3]));

        // When a started future completes, the queued future with the smallest key is
        // started
        senders.remove(&3).unwrap().send(()).unwrap();
        assert_eq!(poll_next(&mut futures), Poll::Ready(Some((3, 3))));
        assert_eq!(futures.first_key(), Some(&1));
        assert_eq!(poll_next(&mut futures), Poll::Pending);
        assert_eq!(*started.lock().unwrap(), BTreeSet::from([1, 2, 3]));

        // The first key considers both running and queued futures
        senders.remove(&1).unwrap().send(()).unwrap();
        assert_eq!(poll_next(&mut futures), Poll::Ready(Some((1, 1))));
        assert_eq!(futures.first_key(), Some(&2));
        senders.remove(&2).unwrap().send(()).unwrap();
        assert_eq!(poll_next(&mut futures), Poll::Ready(Some((2, 2))));
        assert_eq!(futures.first_key(), Some(&4));

        senders.remove(&4).unwrap().send(()).unwrap();
        assert_eq!(poll_next(&mut futures), Poll::Ready(Some((4, 4))));
        senders.remove(&5).unwrap().send(()).unwrap();
        assert_eq!(poll_next(&mut futures), Poll::Ready(Some((5, 5))));
        assert_eq!(poll_next(&mut futures), Poll::Ready(None));
        assert!(futures.is_empty());
        assert_eq!(*started.lock().unwrap(), BTreeSet::from([1, 2, 3, 4, 5]));
    }

    type DirResult = BackendResult<Vec<TestEntry>>;

    /// An entry in a directory walked by `TestVisitor`.
    #[derive(Debug)]
    enum TestEntry {
        File(&'static str),
        /// A directory whose entries are read from the receiver.
        Dir(&'static str, oneshot::Receiver<DirResult>),
    }

    /// Returns a directory entry whose entries are provided by sending them to
    /// the returned sender.
    fn dir(name: &'static str) -> (oneshot::Sender<DirResult>, TestEntry) {
        let (sender, receiver) = oneshot::channel();
        (sender, TestEntry::Dir(name, receiver))
    }

    /// Emits an item for each file and schedules reading each directory.
    /// Records the paths it visits.
    struct TestVisitor {
        visited: Arc<Mutex<Vec<String>>>,
    }

    impl TreeWalkVisitor<'static> for TestVisitor {
        type Work = Vec<TestEntry>;
        type Item = ();

        fn visit(
            &self,
            dir: &RepoPath,
            entries: Self::Work,
            queue: &mut TreeWalkQueue<'static, Self::Work, Self::Item>,
        ) {
            self.visited
                .lock()
                .unwrap()
                .push(dir.as_internal_file_string().to_owned());
            for entry in entries {
                match entry {
                    TestEntry::File(name) => {
                        queue.emit(dir.join(RepoPathComponent::new(name).unwrap()), ());
                    }
                    TestEntry::Dir(name, receiver) => {
                        let path = dir.join(RepoPathComponent::new(name).unwrap());
                        queue.schedule(path, async { receiver.await.unwrap() });
                    }
                }
            }
        }
    }

    /// Returns a walk of a root directory whose entries are provided by
    /// sending them to the returned sender, and the list of visited paths.
    fn new_walk(
        max_concurrent_work: usize,
    ) -> (
        oneshot::Sender<DirResult>,
        OrderedTreeWalk<'static, TestVisitor>,
        Arc<Mutex<Vec<String>>>,
    ) {
        let (sender, receiver) = oneshot::channel();
        let mut queue = TreeWalkQueue::new(max_concurrent_work);
        queue.schedule(RepoPathBuf::root(), async { receiver.await.unwrap() });
        let visited = Arc::new(Mutex::new(vec![]));
        let visitor = TestVisitor {
            visited: visited.clone(),
        };
        (sender, OrderedTreeWalk::new(visitor, queue), visited)
    }

    fn poll_walk(
        walk: &mut OrderedTreeWalk<'static, TestVisitor>,
    ) -> Poll<Option<(String, Result<(), String>)>> {
        walk.poll_next_unpin(&mut Context::from_waker(noop_waker_ref()))
            .map(|item| {
                item.map(|(path, result)| {
                    let path = path.as_internal_file_string().to_owned();
                    (path, result.map_err(|err| err.to_string()))
                })
            })
    }

    fn ok_item(path: &str) -> Poll<Option<(String, Result<(), String>)>> {
        Poll::Ready(Some((path.to_owned(), Ok(()))))
    }

    #[test]
    fn test_walk_out_of_order_completion() {
        let (root, mut walk, visited) = new_walk(2);
        assert_eq!(poll_walk(&mut walk), Poll::Pending);

        let (b, b_entry) = dir("b");
        let (c, c_entry) = dir("c");
        root.send(Ok(vec![
            TestEntry::File("a"),
            b_entry,
            TestEntry::File("b.txt"),
            c_entry,
            TestEntry::File("d"),
        ]))
        .unwrap();
        // Items before pending work are emitted
        assert_eq!(poll_walk(&mut walk), ok_item("a"));
        assert_eq!(poll_walk(&mut walk), Poll::Pending);

        // Items from a later directory wait for the work for earlier directories
        c.send(Ok(vec![TestEntry::File("x")])).unwrap();
        assert_eq!(poll_walk(&mut walk), Poll::Pending);
        assert_eq!(*visited.lock().unwrap(), ["", "c"]);

        // Items within a directory are emitted before later siblings of the directory
        b.send(Ok(vec![TestEntry::File("y")])).unwrap();
        assert_eq!(poll_walk(&mut walk), ok_item("b/y"));
        assert_eq!(poll_walk(&mut walk), ok_item("b.txt"));
        assert_eq!(poll_walk(&mut walk), ok_item("c/x"));
        assert_eq!(poll_walk(&mut walk), ok_item("d"));
        assert_eq!(poll_walk(&mut walk), Poll::Ready(None));
        assert_eq!(*visited.lock().unwrap(), ["", "c", "b"]);
    }

    #[test]
    fn test_walk_error() {
        let (root, mut walk, _visited) = new_walk(2);
        let (b, b_entry) = dir("b");
        root.send(Ok(vec![
            TestEntry::File("a"),
            b_entry,
            TestEntry::File("c"),
        ]))
        .unwrap();
        b.send(Err(BackendError::Other("read failed".into())))
            .unwrap();

        // The error is emitted in order at the path of the failed work
        assert_eq!(poll_walk(&mut walk), ok_item("a"));
        assert_eq!(
            poll_walk(&mut walk),
            Poll::Ready(Some(("b".to_owned(), Err("read failed".to_owned()))))
        );
        assert_eq!(poll_walk(&mut walk), ok_item("c"));
        assert_eq!(poll_walk(&mut walk), Poll::Ready(None));
    }

    #[test]
    fn test_walk_back_pressure() {
        let (root, mut walk, visited) = new_walk(2);
        walk.max_queued_items = 2;
        let (d, d_entry) = dir("d");
        root.send(Ok(vec![
            TestEntry::File("a"),
            TestEntry::File("b"),
            TestEntry::File("c"),
            d_entry,
        ]))
        .unwrap();
        d.send(Ok(vec![TestEntry::File("x")])).unwrap();

        // The work for "d" has completed, but it's not visited while the queue is
        // full
        assert_eq!(poll_walk(&mut walk), ok_item("a"));
        assert_eq!(*visited.lock().unwrap(), [""]);
        assert_eq!(poll_walk(&mut walk), ok_item("b"));
        assert_eq!(*visited.lock().unwrap(), [""]);

        // Once the queue is below the limit, the work is visited
        assert_eq!(poll_walk(&mut walk), ok_item("c"));
        assert_eq!(*visited.lock().unwrap(), ["", "d"]);
        assert_eq!(poll_walk(&mut walk), ok_item("d/x"));
        assert_eq!(poll_walk(&mut walk), Poll::Ready(None));
    }

    #[test]
    fn test_walk_back_pressure_with_blocked_items() {
        let (root, mut walk, _visited) = new_walk(2);
        walk.max_queued_items = 2;
        let (a, a_entry) = dir("a");
        root.send(Ok(vec![
            a_entry,
            TestEntry::File("b"),
            TestEntry::File("c"),
            TestEntry::File("d"),
        ]))
        .unwrap();
        assert_eq!(poll_walk(&mut walk), Poll::Pending);

        // The queue is full, but no item can be emitted until the work for "a"
        // completes, so that work must still be polled
        a.send(Ok(vec![TestEntry::File("x")])).unwrap();
        assert_eq!(poll_walk(&mut walk), ok_item("a/x"));
        assert_eq!(poll_walk(&mut walk), ok_item("b"));
        assert_eq!(poll_walk(&mut walk), ok_item("c"));
        assert_eq!(poll_walk(&mut walk), ok_item("d"));
        assert_eq!(poll_walk(&mut walk), Poll::Ready(None));
    }
}
