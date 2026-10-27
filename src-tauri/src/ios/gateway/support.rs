//! Small building blocks of the gateway: publish rate limiting, message de-duplication and the
//! ordered outbound queue.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::ios::transport::BrokerTransport;
use crate::ios::wire::{
    DEDUP_WINDOW_COUNT, DEDUP_WINDOW_SECS, PUBLISH_BURST, PUBLISH_RATE_PER_SEC,
};

/// Token bucket: `rate` tokens per second up to `burst`.
pub struct TokenBucket {
    rate: f64,
    burst: f64,
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    pub fn new(rate: f64, burst: f64) -> Self {
        Self {
            rate,
            burst,
            tokens: burst,
            last: Instant::now(),
        }
    }

    /// Reserves one token and returns how long the caller has to wait before using it.
    pub fn reserve(&mut self) -> Duration {
        let now = Instant::now();
        self.tokens = (self.tokens + (now - self.last).as_secs_f64() * self.rate).min(self.burst);
        self.last = now;
        self.tokens -= 1.0;
        if self.tokens >= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(-self.tokens / self.rate)
        }
    }
}

/// Remembers recent message ids; a message id seen twice within the window is a duplicate.
#[derive(Default)]
pub struct Dedup {
    order: VecDeque<(String, i64)>,
    seen: HashSet<String>,
}

impl Dedup {
    /// Returns `true` when `id` is new (and records it), `false` for a duplicate.
    pub fn insert(&mut self, id: &str, now_ms: i64) -> bool {
        let cutoff = now_ms - DEDUP_WINDOW_SECS * 1000;
        while let Some((old, ts)) = self.order.front() {
            if *ts < cutoff || self.order.len() > DEDUP_WINDOW_COUNT {
                self.seen.remove(old);
                self.order.pop_front();
            } else {
                break;
            }
        }
        if !self.seen.insert(id.to_string()) {
            return false;
        }
        self.order.push_back((id.to_string(), now_ms));
        true
    }
}

enum Out {
    Publish {
        topic: String,
        payload: Vec<u8>,
        retain: bool,
    },
    Flush(oneshot::Sender<()>),
}

/// Ordered, rate-limited publisher. `send` never blocks; a background task drains the queue.
#[derive(Clone)]
pub struct Outbox {
    tx: UnboundedSender<Out>,
    queued: Arc<AtomicUsize>,
    failed: Arc<AtomicUsize>,
}

impl Outbox {
    pub fn spawn(transport: Arc<dyn BrokerTransport>) -> Self {
        let (tx, mut rx) = unbounded_channel::<Out>();
        let queued = Arc::new(AtomicUsize::new(0));
        let failed = Arc::new(AtomicUsize::new(0));
        let (queued_task, failed_task) = (queued.clone(), failed.clone());
        tokio::spawn(async move {
            let mut bucket = TokenBucket::new(PUBLISH_RATE_PER_SEC, PUBLISH_BURST);
            while let Some(item) = rx.recv().await {
                match item {
                    Out::Publish {
                        topic,
                        payload,
                        retain,
                    } => {
                        let wait = bucket.reserve();
                        if !wait.is_zero() {
                            tokio::time::sleep(wait).await;
                        }
                        if transport.publish(&topic, payload, retain).await.is_err() {
                            failed_task.fetch_add(1, Ordering::Relaxed);
                        }
                        queued_task.fetch_sub(1, Ordering::Relaxed);
                    }
                    Out::Flush(done) => {
                        let _ = done.send(());
                    }
                }
            }
        });
        Self { tx, queued, failed }
    }

    pub fn send(&self, topic: String, payload: Vec<u8>, retain: bool) {
        self.queued.fetch_add(1, Ordering::Relaxed);
        let _ = self.tx.send(Out::Publish {
            topic,
            payload,
            retain,
        });
    }

    /// Waits until everything queued so far has been handed to the transport.
    pub async fn flush(&self) {
        let (done_tx, done_rx) = oneshot::channel();
        if self.tx.send(Out::Flush(done_tx)).is_ok() {
            let _ = tokio::time::timeout(Duration::from_secs(5), done_rx).await;
        }
    }

    pub fn queued(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }

    /// Publishes that the transport refused (not connected, not authorized).
    pub fn failed(&self) -> usize {
        self.failed.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn bucket_allows_a_burst_then_paces_at_the_rate() {
        let mut bucket = TokenBucket::new(6.0, 10.0);
        for _ in 0..10 {
            assert_eq!(bucket.reserve(), Duration::ZERO);
        }
        let wait = bucket.reserve();
        assert!(
            wait > Duration::from_millis(150) && wait < Duration::from_millis(180),
            "{wait:?}"
        );
        let next = bucket.reserve();
        assert!(next > wait);
        tokio::time::advance(Duration::from_secs(5)).await;
        assert_eq!(bucket.reserve(), Duration::ZERO);
    }

    #[test]
    fn dedup_rejects_repeats_within_the_window_and_expires_old_ids() {
        let mut dedup = Dedup::default();
        assert!(dedup.insert("a", 0));
        assert!(!dedup.insert("a", 1_000));
        assert!(dedup.insert("b", 2_000));
        // After the window the id is forgotten.
        assert!(dedup.insert("a", DEDUP_WINDOW_SECS * 1000 + 5_000));
    }

    #[test]
    fn dedup_caps_its_memory() {
        let mut dedup = Dedup::default();
        for i in 0..(DEDUP_WINDOW_COUNT + 100) {
            assert!(dedup.insert(&format!("m{i}"), i as i64));
        }
        assert!(dedup.seen.len() <= DEDUP_WINDOW_COUNT + 1);
        // The very first ids were evicted, the latest are still remembered.
        assert!(dedup.insert("m0", 10_000));
        assert!(!dedup.insert(&format!("m{}", DEDUP_WINDOW_COUNT + 99), 10_001));
    }
}
