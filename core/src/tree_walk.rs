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

#![cfg_attr(not(test), expect(dead_code))]

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::task::Context;
use std::task::Poll;
use std::task::ready;

use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;

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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;

    use futures::channel::oneshot;
    use futures::task::noop_waker_ref;

    use super::*;

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
}
