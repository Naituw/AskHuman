//! Remembers what each Agent offered, so the phone's "new task" form does not start a CLI every
//! time it opens (spec `ios-task-model` M12).
//!
//! - A listing is trusted for [`CatalogCache::new`]'s `ttl` (30 minutes in the daemon).
//! - Past that it is still served, and a refresh runs in the background (stale while revalidate).
//! - Only one refresh per key runs at a time; everyone who wants the answer waits for that one.
//! - A listing that could not be read keeps the previous answer and is asked again after a short
//!   retry delay; with no previous answer the phone gets an empty catalog for that long.
//!
//! The fetch itself is a blocking closure handed in by the caller (it runs on the blocking pool),
//! so the cache knows nothing about processes or files.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::watch;

use super::AgentCatalog;

/// How long a listing is trusted before it is asked for again.
pub const CATALOG_TTL: Duration = Duration::from_secs(30 * 60);
/// How long a listing that could not be read is left alone before the next attempt.
pub const FAILURE_RETRY: Duration = Duration::from_secs(60);

/// What the cache holds for a key right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Peek {
    Fresh(Arc<AgentCatalog>),
    /// Past its time (or the last attempt to renew it failed): usable, but worth renewing.
    Stale(Arc<AgentCatalog>),
    Missing,
}

#[derive(Default)]
struct Slot {
    value: Option<Stored>,
    /// Set while a refresh runs; flips to `true` when it is done.
    refresh: Option<watch::Receiver<bool>>,
}

struct Stored {
    catalog: Arc<AgentCatalog>,
    good_until: Instant,
}

pub struct CatalogCache {
    ttl: Duration,
    retry: Duration,
    slots: Mutex<HashMap<String, Slot>>,
}

impl CatalogCache {
    pub fn new(ttl: Duration, retry: Duration) -> Arc<Self> {
        Arc::new(Self {
            ttl,
            retry,
            slots: Mutex::new(HashMap::new()),
        })
    }

    pub fn peek(&self, key: &str) -> Peek {
        let slots = self.slots.lock().unwrap();
        match slots.get(key).and_then(|slot| slot.value.as_ref()) {
            Some(stored) if Instant::now() < stored.good_until => {
                Peek::Fresh(stored.catalog.clone())
            }
            Some(stored) => Peek::Stale(stored.catalog.clone()),
            None => Peek::Missing,
        }
    }

    /// The catalog to show now.
    ///
    /// - Fresh: returned as it is.
    /// - Stale: returned as it is, and a refresh starts in the background.
    /// - Missing: a refresh starts and is waited for, up to `wait`. `None` when it is not done by
    ///   then (it keeps running and fills the cache for the next ask).
    pub async fn get<F>(
        self: &Arc<Self>,
        key: &str,
        wait: Duration,
        fetch: F,
    ) -> Option<Arc<AgentCatalog>>
    where
        F: FnOnce() -> Result<AgentCatalog, String> + Send + 'static,
    {
        match self.peek(key) {
            Peek::Fresh(catalog) => Some(catalog),
            Peek::Stale(catalog) => {
                self.begin_refresh(key, fetch);
                Some(catalog)
            }
            Peek::Missing => {
                let done = self.begin_refresh(key, fetch);
                wait_for(done, wait).await;
                self.latest(key)
            }
        }
    }

    /// Like [`get`](Self::get), but the answer must also satisfy `enough` (a model the phone named
    /// is in it, say). When what is cached does not, the Agent is asked again and waited for, up
    /// to `wait`; if that fails or is late, the older answer stands.
    pub async fn get_enough<F, P>(
        self: &Arc<Self>,
        key: &str,
        wait: Duration,
        fetch: F,
        enough: P,
    ) -> Option<Arc<AgentCatalog>>
    where
        F: FnOnce() -> Result<AgentCatalog, String> + Send + 'static,
        P: Fn(&AgentCatalog) -> bool,
    {
        if let Some(catalog) = self.latest(key).filter(|catalog| enough(catalog)) {
            return Some(catalog);
        }
        let done = self.begin_refresh(key, fetch);
        wait_for(done, wait).await;
        self.latest(key)
    }

    /// The stored catalog, however old.
    fn latest(&self, key: &str) -> Option<Arc<AgentCatalog>> {
        let slots = self.slots.lock().unwrap();
        slots
            .get(key)
            .and_then(|slot| slot.value.as_ref())
            .map(|stored| stored.catalog.clone())
    }

    /// Starts a refresh of `key` unless one is already running; either way the returned receiver
    /// flips to `true` once that refresh is over.
    fn begin_refresh<F>(self: &Arc<Self>, key: &str, fetch: F) -> watch::Receiver<bool>
    where
        F: FnOnce() -> Result<AgentCatalog, String> + Send + 'static,
    {
        let (sender, receiver) = {
            let mut slots = self.slots.lock().unwrap();
            let slot = slots.entry(key.to_string()).or_default();
            if let Some(running) = &slot.refresh {
                return running.clone();
            }
            let (sender, receiver) = watch::channel(false);
            slot.refresh = Some(receiver.clone());
            (sender, receiver)
        };
        let cache = self.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || {
            let outcome = catch_unwind(AssertUnwindSafe(fetch))
                .unwrap_or_else(|_| Err("the catalog fetch panicked".to_string()));
            cache.finish_refresh(&key, outcome);
            let _ = sender.send(true);
        });
        receiver
    }

    fn finish_refresh(&self, key: &str, outcome: Result<AgentCatalog, String>) {
        let mut slots = self.slots.lock().unwrap();
        let slot = slots.entry(key.to_string()).or_default();
        slot.refresh = None;
        let now = Instant::now();
        match (outcome, &mut slot.value) {
            (Ok(catalog), value) => {
                *value = Some(Stored {
                    catalog: Arc::new(catalog),
                    good_until: now + self.ttl,
                });
            }
            // The old answer stays; it is only asked for again after the retry delay.
            (Err(_), Some(stored)) => stored.good_until = now + self.retry,
            (Err(_), value @ None) => {
                *value = Some(Stored {
                    catalog: Arc::new(AgentCatalog::default()),
                    good_until: now + self.retry,
                });
            }
        }
    }
}

async fn wait_for(mut done: watch::Receiver<bool>, wait: Duration) {
    let _ = tokio::time::timeout(wait, done.wait_for(|finished| *finished)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::model_catalog::ModelEntry;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const LONG: Duration = Duration::from_secs(60);

    fn catalog(ids: &[&str]) -> AgentCatalog {
        AgentCatalog {
            models: ids.iter().map(|id| ModelEntry::plain(id, id)).collect(),
            ..AgentCatalog::default()
        }
    }

    fn ids(catalog: &AgentCatalog) -> Vec<&str> {
        catalog.models.iter().map(|m| m.id.as_str()).collect()
    }

    /// A fetch that counts how often it ran and gives `answer` after `delay`.
    fn fetch(
        runs: &Arc<AtomicUsize>,
        delay: Duration,
        answer: Result<AgentCatalog, String>,
    ) -> impl FnOnce() -> Result<AgentCatalog, String> + Send + 'static {
        let runs = runs.clone();
        move || {
            runs.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(delay);
            answer
        }
    }

    #[tokio::test]
    async fn the_first_ask_fetches_and_the_next_one_reuses_the_answer() {
        let cache = CatalogCache::new(LONG, LONG);
        let runs = Arc::new(AtomicUsize::new(0));
        let first = cache
            .get(
                "k",
                LONG,
                fetch(&runs, Duration::ZERO, Ok(catalog(&["model-a"]))),
            )
            .await
            .unwrap();
        assert_eq!(ids(&first), ["model-a"]);
        let second = cache
            .get(
                "k",
                LONG,
                fetch(&runs, Duration::ZERO, Ok(catalog(&["other"]))),
            )
            .await
            .unwrap();
        assert_eq!(ids(&second), ["model-a"]);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert!(matches!(cache.peek("k"), Peek::Fresh(_)));
        assert_eq!(cache.peek("other-key"), Peek::Missing);
    }

    #[tokio::test]
    async fn keys_do_not_share_answers() {
        let cache = CatalogCache::new(LONG, LONG);
        let runs = Arc::new(AtomicUsize::new(0));
        let a = cache
            .get("a", LONG, fetch(&runs, Duration::ZERO, Ok(catalog(&["x"]))))
            .await
            .unwrap();
        let b = cache
            .get("b", LONG, fetch(&runs, Duration::ZERO, Ok(catalog(&["y"]))))
            .await
            .unwrap();
        assert_eq!((ids(&a), ids(&b)), (vec!["x"], vec!["y"]));
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_slow_fetch_is_not_waited_for_past_the_limit_but_still_fills_the_cache() {
        let cache = CatalogCache::new(LONG, LONG);
        let runs = Arc::new(AtomicUsize::new(0));
        let late = cache
            .get(
                "k",
                Duration::from_millis(20),
                fetch(&runs, Duration::from_millis(300), Ok(catalog(&["model-a"]))),
            )
            .await;
        assert!(late.is_none());
        assert_eq!(cache.peek("k"), Peek::Missing);
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(matches!(cache.peek("k"), Peek::Fresh(_)));
        // The next ask is served from what the late fetch left behind.
        let next = cache
            .get(
                "k",
                LONG,
                fetch(&runs, Duration::ZERO, Ok(catalog(&["again"]))),
            )
            .await
            .unwrap();
        assert_eq!(ids(&next), ["model-a"]);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn concurrent_asks_share_one_fetch() {
        let cache = CatalogCache::new(LONG, LONG);
        let runs = Arc::new(AtomicUsize::new(0));
        let asks = (0..5).map(|_| {
            let cache = cache.clone();
            let slow = fetch(&runs, Duration::from_millis(100), Ok(catalog(&["model-a"])));
            async move { cache.get("k", LONG, slow).await }
        });
        let answers = futures_util::future::join_all(asks).await;
        assert!(answers
            .iter()
            .all(|a| ids(a.as_ref().unwrap()) == ["model-a"]));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_stale_answer_is_served_at_once_and_renewed_in_the_background() {
        let cache = CatalogCache::new(Duration::from_millis(30), LONG);
        let runs = Arc::new(AtomicUsize::new(0));
        cache
            .get(
                "k",
                LONG,
                fetch(&runs, Duration::ZERO, Ok(catalog(&["old"]))),
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(matches!(cache.peek("k"), Peek::Stale(_)));
        let served = cache
            .get(
                "k",
                LONG,
                fetch(&runs, Duration::from_millis(50), Ok(catalog(&["new"]))),
            )
            .await
            .unwrap();
        assert_eq!(ids(&served), ["old"]);
        tokio::time::sleep(Duration::from_millis(300)).await;
        // The renewed answer is in place (with a ttl this short it is already stale again).
        match cache.peek("k") {
            Peek::Fresh(now) | Peek::Stale(now) => assert_eq!(ids(&now), ["new"]),
            Peek::Missing => panic!("the answer vanished"),
        }
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_failed_renewal_keeps_the_old_answer_and_waits_before_trying_again() {
        let cache = CatalogCache::new(Duration::from_millis(20), Duration::from_millis(200));
        let runs = Arc::new(AtomicUsize::new(0));
        cache
            .get(
                "k",
                LONG,
                fetch(&runs, Duration::ZERO, Ok(catalog(&["old"]))),
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(40)).await;
        cache
            .get(
                "k",
                LONG,
                fetch(&runs, Duration::ZERO, Err("no network".into())),
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        // Kept, and not worth asking for again yet.
        match cache.peek("k") {
            Peek::Fresh(kept) => assert_eq!(ids(&kept), ["old"]),
            other => panic!("expected the old answer to stay fresh, got {other:?}"),
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(matches!(cache.peek("k"), Peek::Stale(_)));
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_failure_with_nothing_before_it_is_an_empty_catalog_for_a_while() {
        let cache = CatalogCache::new(LONG, LONG);
        let runs = Arc::new(AtomicUsize::new(0));
        let first = cache
            .get(
                "k",
                LONG,
                fetch(&runs, Duration::ZERO, Err("not signed in".into())),
            )
            .await
            .unwrap();
        assert!(first.models.is_empty());
        // Not asked again within the retry window.
        let again = cache
            .get(
                "k",
                LONG,
                fetch(&runs, Duration::ZERO, Ok(catalog(&["model-a"]))),
            )
            .await
            .unwrap();
        assert!(again.models.is_empty());
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_panicking_fetch_is_a_failure_not_a_crash() {
        let cache = CatalogCache::new(LONG, LONG);
        let answer = cache
            .get("k", LONG, || -> Result<AgentCatalog, String> {
                panic!("boom")
            })
            .await
            .unwrap();
        assert!(answer.models.is_empty());
        // The slot is usable afterwards.
        assert!(matches!(cache.peek("k"), Peek::Fresh(_)));
    }

    #[tokio::test]
    async fn an_ask_that_needs_more_than_the_cache_has_asks_the_agent_again() {
        let cache = CatalogCache::new(LONG, LONG);
        let runs = Arc::new(AtomicUsize::new(0));
        cache
            .get(
                "k",
                LONG,
                fetch(&runs, Duration::ZERO, Ok(catalog(&["model-a"]))),
            )
            .await
            .unwrap();
        // Already good enough: no second fetch.
        let kept = cache
            .get_enough(
                "k",
                LONG,
                fetch(&runs, Duration::ZERO, Ok(catalog(&["unused"]))),
                |c| c.model("model-a").is_some(),
            )
            .await
            .unwrap();
        assert_eq!(ids(&kept), ["model-a"]);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        // A model that appeared since: the Agent is asked again.
        let renewed = cache
            .get_enough(
                "k",
                LONG,
                fetch(&runs, Duration::ZERO, Ok(catalog(&["model-a", "model-b"]))),
                |c| c.model("model-b").is_some(),
            )
            .await
            .unwrap();
        assert_eq!(ids(&renewed), ["model-a", "model-b"]);
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn when_asking_again_fails_the_older_answer_stands() {
        let cache = CatalogCache::new(LONG, LONG);
        let runs = Arc::new(AtomicUsize::new(0));
        cache
            .get(
                "k",
                LONG,
                fetch(&runs, Duration::ZERO, Ok(catalog(&["model-a"]))),
            )
            .await
            .unwrap();
        let answer = cache
            .get_enough(
                "k",
                LONG,
                fetch(&runs, Duration::ZERO, Err("offline".into())),
                |c| c.model("model-b").is_some(),
            )
            .await
            .unwrap();
        assert_eq!(ids(&answer), ["model-a"]);
        // Too slow counts the same.
        let answer = cache
            .get_enough(
                "k",
                Duration::from_millis(10),
                fetch(
                    &runs,
                    Duration::from_millis(200),
                    Ok(catalog(&["model-a", "model-b"])),
                ),
                |c| c.model("model-b").is_some(),
            )
            .await
            .unwrap();
        assert_eq!(ids(&answer), ["model-a"]);
    }

    #[tokio::test]
    async fn an_ask_that_needs_more_than_an_empty_cache_has_waits_for_the_first_fetch() {
        let cache = CatalogCache::new(LONG, LONG);
        let runs = Arc::new(AtomicUsize::new(0));
        let answer = cache
            .get_enough(
                "k",
                LONG,
                fetch(&runs, Duration::from_millis(30), Ok(catalog(&["model-a"]))),
                |c| c.model("model-a").is_some(),
            )
            .await
            .unwrap();
        assert_eq!(ids(&answer), ["model-a"]);
        let nothing = CatalogCache::new(LONG, LONG)
            .get_enough(
                "k",
                Duration::from_millis(10),
                fetch(&runs, Duration::from_millis(200), Ok(catalog(&["model-a"]))),
                |_| true,
            )
            .await;
        assert!(nothing.is_none());
    }
}
