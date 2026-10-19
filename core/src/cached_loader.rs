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

//! LRU cache that deduplicates concurrent loads of the same key.

use std::hash::Hash;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::Mutex;

use clru::CLruCache;
use futures::FutureExt as _;
use futures::future::BoxFuture;
use futures::future::Shared;
use futures::future::WeakShared;

/// An LRU cache that makes concurrent cache misses for the same key share a
/// single load.
///
/// The first caller for a key starts the load. Callers that arrive while it is
/// in progress wait for its result instead of starting their own. Every
/// waiting caller polls the shared load, so it makes progress as long as any
/// caller is polled, and it is not canceled when the caller that started it is
/// dropped. When the load succeeds, the callers waiting for it store the value
/// in the cache. If it fails, the error is returned only to the caller that
/// started it, and each of the other callers loads the value itself, without
/// sharing the load.
///
/// The load may be polled by any of the callers waiting for it, so it must not
/// depend on the context of the caller that started it, such as task-locals or
/// the presence of a particular async runtime.
///
/// The cache holds only a weak reference to a load in progress, so a load is
/// dropped (canceling it) when all callers waiting for it are dropped. The
/// entry for a load that failed or was dropped stays in the cache until it is
/// evicted or the key is requested again, which starts a new load.
///
/// If the load panics, every caller waiting for it panics too (with "inner
/// future panicked during poll"), not just the caller whose poll panicked.
/// Callers that arrive later join the poisoned load and panic as well, until
/// all callers holding it have been dropped.
pub(crate) struct CachedLoader<K, V> {
    entries: Mutex<CLruCache<K, Entry<V>>>,
}

enum Entry<V> {
    Loaded(V),
    // Weak reference so a load that all callers dropped is freed.
    Loading(WeakShared<BoxFuture<'static, Option<V>>>),
}

/// A load shared by concurrent callers. Its output is `None` if the load
/// failed.
type SharedLoad<V> = Shared<BoxFuture<'static, Option<V>>>;

enum Lookup<V> {
    Loaded(V),
    Loading(SharedLoad<V>),
    Missing,
}

enum StartLoad<V, E, F> {
    Loaded(V),
    // Another caller's load, and the future we would otherwise have shared.
    Joined(SharedLoad<V>, F),
    // Our load, and where it stores its error.
    Started(SharedLoad<V>, Arc<Mutex<Option<E>>>),
}

impl<K, V> CachedLoader<K, V>
where
    K: Clone + Eq + Hash + Send + 'static,
    V: Clone + Send + Sync + 'static,
{
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self {
            entries: Mutex::new(CLruCache::new(capacity)),
        }
    }

    /// Stores `value` in the cache.
    pub fn insert(&self, key: K, value: V) {
        self.entries.lock().unwrap().put(key, Entry::Loaded(value));
    }

    /// Removes all values from the cache. Loads that are in progress still
    /// store their results when they finish.
    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
    }

    /// Returns the cached value for `key`, the result of a concurrent load of
    /// it, or the result of the future returned by `load`.
    ///
    /// `load` is called without holding any lock, so it may use this cache.
    pub async fn get_or_load<E, F>(&self, key: &K, load: impl FnOnce() -> F) -> Result<V, E>
    where
        E: Send + 'static,
        F: Future<Output = Result<V, E>> + Send + 'static,
    {
        match self.lookup(key) {
            Lookup::Loaded(value) => Ok(value),
            Lookup::Loading(shared_load) => self.wait(key, shared_load, load).await,
            Lookup::Missing => match self.start_load(key, load()) {
                StartLoad::Loaded(value) => Ok(value),
                StartLoad::Joined(shared_load, future) => {
                    self.wait(key, shared_load, || future).await
                }
                StartLoad::Started(shared_load, error_slot) => match shared_load.await {
                    Some(value) => {
                        self.store_loaded(key, &value);
                        Ok(value)
                    }
                    None => Err(error_slot.lock().unwrap().take().unwrap()),
                },
            },
        }
    }

    fn lookup(&self, key: &K) -> Lookup<V> {
        lookup_entry(&mut self.entries.lock().unwrap(), key)
    }

    /// Makes `future` the shared load of `key`, unless another caller started
    /// loading it after we looked it up.
    fn start_load<E, F>(&self, key: &K, future: F) -> StartLoad<V, E, F>
    where
        E: Send + 'static,
        F: Future<Output = Result<V, E>> + Send + 'static,
    {
        let mut entries = self.entries.lock().unwrap();
        match lookup_entry(&mut entries, key) {
            Lookup::Loaded(value) => return StartLoad::Loaded(value),
            Lookup::Loading(shared_load) => return StartLoad::Joined(shared_load, future),
            Lookup::Missing => {}
        }
        let error_slot = Arc::new(Mutex::new(None));
        let shared_load = {
            let error_slot = error_slot.clone();
            async move {
                match future.await {
                    Ok(value) => Some(value),
                    Err(err) => {
                        *error_slot.lock().unwrap() = Some(err);
                        None
                    }
                }
            }
            .boxed()
            .shared()
        };
        entries.put(
            key.clone(),
            Entry::Loading(shared_load.downgrade().unwrap()),
        );
        StartLoad::Started(shared_load, error_slot)
    }

    /// Waits for another caller's load, or calls `load` if that load fails.
    async fn wait<E, F>(
        &self,
        key: &K,
        shared_load: SharedLoad<V>,
        load: impl FnOnce() -> F,
    ) -> Result<V, E>
    where
        F: Future<Output = Result<V, E>>,
    {
        let value = match shared_load.await {
            Some(value) => value,
            // The error was returned to the caller that started the load. We
            // don't share the new load, since the other callers would then
            // wait for each other's loads one after another if they keep
            // failing.
            None => load().await?,
        };
        self.store_loaded(key, &value);
        Ok(value)
    }

    /// Stores the result of a load in the cache, replacing the entry for the
    /// load.
    fn store_loaded(&self, key: &K, value: &V) {
        let mut entries = self.entries.lock().unwrap();
        match entries.get_mut(key) {
            Some(Entry::Loaded(_)) => {}
            Some(entry) => *entry = Entry::Loaded(value.clone()),
            // The entry was evicted while the load was in progress.
            None => {
                entries.put(key.clone(), Entry::Loaded(value.clone()));
            }
        }
    }
}

/// Returns the cached value for `key` or the load of it that may still succeed.
fn lookup_entry<K: Eq + Hash, V: Clone>(
    entries: &mut CLruCache<K, Entry<V>>,
    key: &K,
) -> Lookup<V> {
    match entries.get(key) {
        Some(Entry::Loaded(value)) => Lookup::Loaded(value.clone()),
        Some(Entry::Loading(weak)) => live_load(weak).map_or(Lookup::Missing, Lookup::Loading),
        None => Lookup::Missing,
    }
}

/// Returns the load if it may still succeed, i.e. if it is still referenced
/// and has not failed.
fn live_load<V: Clone>(weak: &WeakShared<BoxFuture<'static, Option<V>>>) -> Option<SharedLoad<V>> {
    weak.upgrade()
        .filter(|shared_load| !matches!(shared_load.peek(), Some(None)))
}

#[cfg(test)]
mod tests {
    use std::pin::pin;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;
    use std::task::Context;
    use std::task::Poll;

    use futures::channel::oneshot;
    use futures::future;
    use futures::poll;
    use futures::task::noop_waker_ref;
    use pollster::FutureExt as _;

    use super::*;

    const CAPACITY: NonZeroUsize = NonZeroUsize::new(10).unwrap();

    fn cached_value<K: Eq + Hash, V: Clone>(loader: &CachedLoader<K, V>, key: &K) -> Option<V> {
        match loader.entries.lock().unwrap().peek(key) {
            Some(Entry::Loaded(value)) => Some(value.clone()),
            Some(Entry::Loading(_)) | None => None,
        }
    }

    #[test]
    fn test_concurrent_calls_share_load() {
        let loader = CachedLoader::<u32, String>::new(CAPACITY);
        let calls = Arc::new(AtomicUsize::new(0));
        let (sender, receiver) = oneshot::channel::<()>();
        let receiver = receiver.shared();
        let load = || {
            let calls = calls.clone();
            let receiver = receiver.clone();
            async move {
                calls.fetch_add(1, Ordering::Relaxed);
                receiver.await.unwrap();
                Ok::<_, ()>("value".to_owned())
            }
        };
        let mut first = pin!(loader.get_or_load(&1, load));
        let mut second = pin!(loader.get_or_load(&1, load));
        async {
            assert!(poll!(&mut first).is_pending());
            assert!(poll!(&mut second).is_pending());
            sender.send(()).unwrap();
            assert_eq!(first.await, Ok("value".to_owned()));
            assert_eq!(second.await, Ok("value".to_owned()));
            // Later calls get the value from the cache.
            assert_eq!(loader.get_or_load(&1, load).await, Ok("value".to_owned()));
        }
        .block_on();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(cached_value(&loader, &1), Some("value".to_owned()));
    }

    #[test]
    fn test_inserted_value_is_returned() {
        let loader = CachedLoader::<u32, u32>::new(CAPACITY);
        loader.insert(1, 1);
        let result = loader
            .get_or_load(&1, || async { Ok::<_, ()>(2) })
            .block_on();
        assert_eq!(result, Ok(1));
    }

    #[test]
    fn test_value_inserted_before_load_starts_is_returned() {
        let loader = CachedLoader::<u32, u32>::new(CAPACITY);
        let result = loader
            .get_or_load(&1, || {
                // Another caller stores the value after we looked it up but
                // before we start loading it.
                loader.insert(1, 1);
                async { Ok::<_, ()>(2) }
            })
            .block_on();
        assert_eq!(result, Ok(1));
    }

    #[test]
    fn test_load_started_before_ours_is_joined() {
        let loader = CachedLoader::<u32, u32>::new(CAPACITY);
        let (sender, receiver) = oneshot::channel::<()>();
        let mut second = pin!(loader.get_or_load(&1, || async {
            receiver.await.unwrap();
            Ok::<_, ()>(2)
        }));
        let first = loader.get_or_load(&1, || {
            // Another caller starts loading the value after we looked it up
            // but before we start loading it.
            let mut cx = Context::from_waker(noop_waker_ref());
            assert!(second.as_mut().poll(&mut cx).is_pending());
            async { Ok::<_, ()>(1) }
        });
        async {
            let mut first = pin!(first);
            assert!(poll!(&mut first).is_pending());
            sender.send(()).unwrap();
            assert_eq!(first.await, Ok(2));
        }
        .block_on();
        assert_eq!(second.block_on(), Ok(2));
        assert_eq!(cached_value(&loader, &1), Some(2));
    }

    #[test]
    fn test_different_keys_are_independent() {
        let loader = CachedLoader::<u32, u32>::new(CAPACITY);
        let calls = Arc::new(AtomicUsize::new(0));
        let load = |value| {
            let calls = calls.clone();
            move || async move {
                calls.fetch_add(1, Ordering::Relaxed);
                Ok::<_, ()>(value)
            }
        };
        let (first, second) = future::join(
            loader.get_or_load(&1, load(1)),
            loader.get_or_load(&2, load(2)),
        )
        .block_on();
        assert_eq!(first, Ok(1));
        assert_eq!(second, Ok(2));
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_error_is_not_shared() {
        let loader = CachedLoader::<u32, u32>::new(CAPACITY);
        let (sender, receiver) = oneshot::channel::<()>();
        let mut first = pin!(loader.get_or_load(&1, || async {
            receiver.await.unwrap();
            Err("failed")
        }));
        let mut second = pin!(loader.get_or_load(&1, || async { Ok::<_, &str>(2) }));
        async {
            assert!(poll!(&mut first).is_pending());
            assert!(poll!(&mut second).is_pending());
            sender.send(()).unwrap();
            assert_eq!(first.await, Err("failed"));
            // The second caller loads the value itself.
            assert_eq!(second.await, Ok(2));
        }
        .block_on();
        assert_eq!(cached_value(&loader, &1), Some(2));
    }

    #[test]
    fn test_callers_retry_failed_load_concurrently() {
        let loader = CachedLoader::<u32, u32>::new(CAPACITY);
        let (sender, receiver) = oneshot::channel::<()>();
        let retries = Arc::new(AtomicUsize::new(0));
        let retry = || {
            let retries = retries.clone();
            async move {
                retries.fetch_add(1, Ordering::Relaxed);
                future::pending::<Result<u32, &str>>().await
            }
        };
        let mut first = pin!(loader.get_or_load(&1, || async {
            receiver.await.unwrap();
            Err("failed")
        }));
        let mut second = pin!(loader.get_or_load(&1, retry));
        let mut third = pin!(loader.get_or_load(&1, retry));
        async {
            assert!(poll!(&mut first).is_pending());
            assert!(poll!(&mut second).is_pending());
            assert!(poll!(&mut third).is_pending());
            sender.send(()).unwrap();
            assert_eq!(first.await, Err("failed"));
            // Neither caller waits for the other's retry.
            assert!(poll!(&mut second).is_pending());
            assert!(poll!(&mut third).is_pending());
        }
        .block_on();
        assert_eq!(retries.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_load_can_use_cache() {
        let loader = CachedLoader::<u32, u32>::new(CAPACITY);
        let result = loader
            .get_or_load(&1, || {
                loader.insert(2, 2);
                async { Ok::<_, ()>(1) }
            })
            .block_on();
        assert_eq!(result, Ok(1));
        assert_eq!(cached_value(&loader, &2), Some(2));
    }

    #[test]
    fn test_waiter_drives_load() {
        let loader = CachedLoader::<u32, u32>::new(CAPACITY);
        let (sender, receiver) = oneshot::channel::<()>();
        let mut first = pin!(loader.get_or_load(&1, || async {
            receiver.await.unwrap();
            Ok::<_, ()>(1)
        }));
        let second = loader.get_or_load(&1, || async { Ok::<_, ()>(2) });
        async {
            assert!(poll!(&mut first).is_pending());
            sender.send(()).unwrap();
            // The first caller is not polled again, but the second caller
            // completes the load.
            assert_eq!(second.await, Ok(1));
            assert_eq!(first.await, Ok(1));
        }
        .block_on();
        assert_eq!(cached_value(&loader, &1), Some(1));
    }

    #[test]
    fn test_dropped_caller_does_not_cancel_load() {
        let loader = CachedLoader::<u32, u32>::new(CAPACITY);
        let (sender, receiver) = oneshot::channel::<()>();
        let mut second = pin!(loader.get_or_load(&1, || async { Ok::<_, ()>(2) }));
        async {
            {
                let mut first = pin!(loader.get_or_load(&1, || async {
                    receiver.await.unwrap();
                    Ok::<_, ()>(1)
                }));
                assert!(poll!(&mut first).is_pending());
                assert!(poll!(&mut second).is_pending());
            }
            sender.send(()).unwrap();
            assert_eq!(poll!(&mut second), Poll::Ready(Ok(1)));
        }
        .block_on();
        assert_eq!(cached_value(&loader, &1), Some(1));
    }

    #[test]
    fn test_load_stores_value_after_clear() {
        let loader = CachedLoader::<u32, u32>::new(CAPACITY);
        let (sender, receiver) = oneshot::channel::<()>();
        let mut first = pin!(loader.get_or_load(&1, || async {
            receiver.await.unwrap();
            Ok::<_, ()>(1)
        }));
        async {
            assert!(poll!(&mut first).is_pending());
            loader.clear();
            sender.send(()).unwrap();
            assert_eq!(first.await, Ok(1));
        }
        .block_on();
        assert_eq!(cached_value(&loader, &1), Some(1));
    }

    #[test]
    fn test_load_stores_value_after_eviction() {
        let loader = CachedLoader::<u32, u32>::new(NonZeroUsize::new(1).unwrap());
        let (sender, receiver) = oneshot::channel::<()>();
        let mut first = pin!(loader.get_or_load(&1, || async {
            receiver.await.unwrap();
            Ok::<_, ()>(1)
        }));
        async {
            assert!(poll!(&mut first).is_pending());
            // Evicts the entry for the load.
            loader.insert(2, 2);
            sender.send(()).unwrap();
            assert_eq!(first.await, Ok(1));
        }
        .block_on();
        assert_eq!(cached_value(&loader, &1), Some(1));
        assert_eq!(cached_value(&loader, &2), None);
    }

    #[test]
    fn test_caller_does_not_join_evicted_load() {
        let loader = CachedLoader::<u32, u32>::new(NonZeroUsize::new(1).unwrap());
        let mut first = pin!(loader.get_or_load(&1, future::pending::<Result<u32, ()>>));
        async {
            assert!(poll!(&mut first).is_pending());
            // Evicts the entry for the load.
            loader.insert(2, 2);
            // The second caller can't find the load, so it starts its own.
            let second = loader.get_or_load(&1, || async { Ok::<_, ()>(3) });
            assert_eq!(second.await, Ok(3));
        }
        .block_on();
        assert_eq!(cached_value(&loader, &1), Some(3));
    }

    #[test]
    fn test_load_dropped_by_all_callers_is_restarted() {
        let loader = CachedLoader::<u32, u32>::new(CAPACITY);
        {
            let mut first = pin!(loader.get_or_load(&1, future::pending::<Result<u32, ()>>));
            async { assert!(poll!(&mut first).is_pending()) }.block_on();
        }
        let second = loader.get_or_load(&1, || async { Ok::<_, ()>(2) });
        assert_eq!(second.block_on(), Ok(2));
        assert_eq!(cached_value(&loader, &1), Some(2));
    }
}
