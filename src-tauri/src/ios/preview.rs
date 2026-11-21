//! Downscaled previews of image attachments (docs/specs/ios-attachment-traffic.md §6.3–§6.5).
//!
//! A phone that opens a picture asks for the preview first (`asset.get` with `variant: preview`) and for the original
//! only when the user wants the detail. The preview is made when the request arrives, on a blocking thread, and kept
//! in a small cache in memory. Nothing is written to disk.
//!
//! This module owns the rules and the bookkeeping: which files get a preview, the cache, the requests that wait for
//! the same preview, and how many previews are made at once. The pixels are the job of the `ah-preview` crate, which is
//! compiled with the optimization level that job needs (AT20; see the profile overrides in `Cargo.toml`).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use tokio::sync::Semaphore;

use super::crypto::sha256_hex;
use super::views::{mime_for, AssetSource};
use super::wire::{AssetGet, MAX_ASSET_BYTES};

/// Longest edge of a preview in pixels (AT2). Raising it makes every preview bigger: 2048 costs about 27% more bytes.
pub const PREVIEW_MAX_EDGE: u32 = 1600;
/// AVIF quality of a preview (AT2). 85 costs about 14% more bytes.
pub const PREVIEW_QUALITY: f32 = 76.0;
/// rav1e speed: 6 is the speed the sizes and the times of the spec were measured with.
pub const PREVIEW_SPEED: u8 = 6;
/// A file smaller than this is sent as it is: below it a preview saves little (the files under 400 KiB are about 1.6% of
/// the bytes of the pictures that were measured) and the original is the better picture (AT10).
pub const PREVIEW_MIN_ORIGINAL_BYTES: u64 = 400 * 1024;
/// Pictures with more pixels than this are not decoded for a preview (a picture bomb is a few kilobytes of PNG). The
/// original is sent instead.
pub const PREVIEW_MAX_SOURCE_PIXELS: u64 = 50_000_000;

/// How many previews the cache holds and how many bytes all of them may add up to; the oldest go first. A preview is
/// about 75 KB, so this is generous: a viewer asks for the page that is open and its two neighbours.
const CACHE_MAX_ENTRIES: usize = 32;
const CACHE_MAX_BYTES: usize = 32 * 1024 * 1024;
/// How many previews are made at the same time. The viewer asks for three pages at once, and each preview spreads over
/// all cores by itself (the measurements of three at a time are in §10.3).
const MAX_CONCURRENT: usize = 3;
/// What a remembered "this file gets no preview" counts for in the cache's byte budget.
const VERDICT_BYTES: usize = 256;

/// `asset.get` and `asset.chunk` `variant` of a preview.
pub const VARIANT_PREVIEW: &str = "preview";
/// `asset.chunk` `variant` of a file that is sent as it is, when the phone asked for another one.
pub const VARIANT_ORIGINAL: &str = "original";

/// Whether the file of a request or of the history can have a preview, by what is known without opening it (AT10): a
/// PNG, JPEG or WebP of at least [`PREVIEW_MIN_ORIGINAL_BYTES`] that a phone may download. Whether a preview comes out
/// is only known once the picture is decoded.
pub fn eligible(name: &str, size: u64) -> bool {
    matches!(mime_for(name), "image/png" | "image/jpeg" | "image/webp")
        && (PREVIEW_MIN_ORIGINAL_BYTES..=MAX_ASSET_BYTES).contains(&size)
}

/// Makes the preview of a picture file. The product's is [`AvifMaker`]; a test host swaps in one it can count, slow
/// down and script.
pub trait PreviewMaker: Send + Sync + 'static {
    fn make(&self, original: &[u8]) -> Result<ah_preview::Preview, ah_preview::Skip>;
}

/// The product's maker: AVIF, long edge [`PREVIEW_MAX_EDGE`], quality [`PREVIEW_QUALITY`].
pub struct AvifMaker;

impl PreviewMaker for AvifMaker {
    fn make(&self, original: &[u8]) -> Result<ah_preview::Preview, ah_preview::Skip> {
        ah_preview::make_preview(
            original,
            &ah_preview::PreviewParams {
                max_edge: PREVIEW_MAX_EDGE,
                quality: PREVIEW_QUALITY,
                speed: PREVIEW_SPEED,
                max_source_pixels: PREVIEW_MAX_SOURCE_PIXELS,
            },
        )
    }
}

/// A file as it was when its preview was made: its path, size and modification time. A file that is changed or replaced
/// is another key, so a stale preview is never served for it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PreviewKey {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
}

impl PreviewKey {
    /// `None` when the file is not there (any more) or is not a regular file.
    pub fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        meta.is_file().then(|| Self {
            path: path.to_path_buf(),
            size: meta.len(),
            modified: meta.modified().ok(),
        })
    }
}

/// A finished preview: the AVIF bytes and their SHA-256, which the first `asset.chunk` carries.
#[derive(Debug)]
pub(crate) struct PreviewBytes {
    pub avif: Vec<u8>,
    pub sha256: String,
}

/// What became of one file.
#[derive(Debug)]
pub(crate) enum Outcome {
    Preview(Arc<PreviewBytes>),
    /// The file gets no preview, whatever the reason (transparent, animated, not decodable, too big to decode, a color
    /// profile that cannot be converted, or a preview that is not smaller): it is served as it is.
    Original,
}

/// The result for one file, with the hash of the bytes it was made from.
#[derive(Debug)]
pub(crate) struct Made {
    /// SHA-256 of the original that was read. A request records the hash of the file as it was when the request was
    /// built; a preview made from other bytes is not the preview of that request's picture (AT19).
    pub original_sha256: String,
    pub outcome: Outcome,
}

impl Made {
    /// The preview, if there is one and it was made from the file `source` describes.
    pub fn preview_for(&self, source: &AssetSource) -> Option<&Arc<PreviewBytes>> {
        let Outcome::Preview(preview) = &self.outcome else {
            return None;
        };
        match &source.sha256 {
            Some(expected) if *expected != self.original_sha256 => None,
            _ => Some(preview),
        }
    }

    fn cache_bytes(&self) -> usize {
        match &self.outcome {
            Outcome::Preview(preview) => preview.avif.len() + 64,
            Outcome::Original => VERDICT_BYTES,
        }
    }
}

/// What making the preview of one file came to.
pub(crate) enum Generated {
    Made(Arc<Made>),
    /// The file could not be read.
    Unreadable,
    /// The file is bigger than a phone may download.
    TooLarge,
}

/// A request that waits for a preview that is being made.
pub(crate) struct Waiter {
    pub phone_id: String,
    pub get: AssetGet,
    pub source: AssetSource,
}

/// The limits of [`Previews`]; the product's values are the constants above.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PreviewLimits {
    pub entries: usize,
    pub bytes: usize,
    pub concurrent: usize,
}

impl Default for PreviewLimits {
    fn default() -> Self {
        Self {
            entries: CACHE_MAX_ENTRIES,
            bytes: CACHE_MAX_BYTES,
            concurrent: MAX_CONCURRENT,
        }
    }
}

/// The previews of one gateway: the cache, the requests waiting for a preview and the cap on previews made at once.
/// Owned by the gateway actor, which is the only one to touch the maps; the work itself runs elsewhere
/// ([`Previews::generation`]).
pub(crate) struct Previews {
    maker: Arc<dyn PreviewMaker>,
    permits: Arc<Semaphore>,
    limits: PreviewLimits,
    cache: HashMap<PreviewKey, Arc<Made>>,
    /// The keys of `cache`, the oldest first.
    order: VecDeque<PreviewKey>,
    cached_bytes: usize,
    /// The requests that wait for a preview, by file. A key is here for as long as its preview is being made.
    pending: HashMap<PreviewKey, Vec<Waiter>>,
}

impl Previews {
    pub fn new(maker: Arc<dyn PreviewMaker>) -> Self {
        Self::with_limits(maker, PreviewLimits::default())
    }

    pub fn with_limits(maker: Arc<dyn PreviewMaker>, limits: PreviewLimits) -> Self {
        Self {
            maker,
            permits: Arc::new(Semaphore::new(limits.concurrent.max(1))),
            limits,
            cache: HashMap::new(),
            order: VecDeque::new(),
            cached_bytes: 0,
            pending: HashMap::new(),
        }
    }

    /// The remembered result for `key`, if there is one.
    pub fn lookup(&self, key: &PreviewKey) -> Option<Arc<Made>> {
        self.cache.get(key).cloned()
    }

    /// Registers `waiter` for the preview of `key`. Returns `true` when that preview is already being made, so the
    /// caller has nothing to start; `false` when the caller is the first and must start it.
    pub fn join(&mut self, key: PreviewKey, waiter: Waiter) -> bool {
        match self.pending.entry(key) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                entry.get_mut().push(waiter);
                true
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(vec![waiter]);
                false
            }
        }
    }

    /// The preview of `key` is done: remembers it (when there is something to remember) and hands back everyone who
    /// waited for it.
    pub fn finish(&mut self, key: &PreviewKey, made: Option<Arc<Made>>) -> Vec<Waiter> {
        let waiters = self.pending.remove(key).unwrap_or_default();
        if let Some(made) = made {
            self.remember(key.clone(), made);
        }
        waiters
    }

    fn remember(&mut self, key: PreviewKey, made: Arc<Made>) {
        self.forget(&key);
        self.cached_bytes += made.cache_bytes();
        self.order.push_back(key.clone());
        self.cache.insert(key, made);
        while self.order.len() > self.limits.entries || self.cached_bytes > self.limits.bytes {
            // The entry just added goes too when it alone is over the budget.
            let Some(oldest) = self.order.front().cloned() else {
                break;
            };
            self.forget(&oldest);
        }
    }

    fn forget(&mut self, key: &PreviewKey) {
        if let Some(made) = self.cache.remove(key) {
            self.cached_bytes -= made.cache_bytes();
            self.order.retain(|k| k != key);
        }
    }

    /// The work of making the preview of the file at `path`, to be run in the background. It waits for its turn when
    /// [`PreviewLimits::concurrent`] previews are being made already, and reads the file and does the pixels on a
    /// blocking thread, so the gateway's other messages are not held up.
    pub fn generation(
        &self,
        path: PathBuf,
    ) -> impl std::future::Future<Output = Generated> + Send + 'static {
        let maker = self.maker.clone();
        let permits = self.permits.clone();
        async move {
            // The semaphore is never closed.
            let Ok(_turn) = permits.acquire_owned().await else {
                return Generated::Unreadable;
            };
            tokio::task::spawn_blocking(move || generate(maker.as_ref(), &path))
                .await
                .unwrap_or(Generated::Unreadable)
        }
    }

    #[cfg(test)]
    pub fn cached(&self) -> usize {
        self.cache.len()
    }

    #[cfg(test)]
    pub fn waiting(&self) -> usize {
        self.pending.values().map(Vec::len).sum()
    }
}

/// Reads the file, hashes it and makes its preview. Blocking.
fn generate(maker: &dyn PreviewMaker, path: &Path) -> Generated {
    let original = match std::fs::read(path) {
        Ok(bytes) if bytes.len() as u64 <= MAX_ASSET_BYTES => bytes,
        Ok(_) => return Generated::TooLarge,
        Err(_) => return Generated::Unreadable,
    };
    let original_sha256 = sha256_hex(&original);
    let outcome = match maker.make(&original) {
        Ok(preview) => Outcome::Preview(Arc::new(PreviewBytes {
            sha256: sha256_hex(&preview.avif),
            avif: preview.avif,
        })),
        Err(_) => Outcome::Original,
    };
    Generated::Made(Arc::new(Made {
        original_sha256,
        outcome,
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    /// A maker that returns `len` bytes of 7 and counts, and how many calls overlap at most.
    struct Counting {
        len: usize,
        calls: AtomicUsize,
        running: AtomicUsize,
        most_running: AtomicUsize,
        takes: Duration,
    }

    impl Counting {
        fn new(len: usize, takes: Duration) -> Arc<Self> {
            Arc::new(Self {
                len,
                calls: AtomicUsize::new(0),
                running: AtomicUsize::new(0),
                most_running: AtomicUsize::new(0),
                takes,
            })
        }
    }

    impl PreviewMaker for Counting {
        fn make(&self, _original: &[u8]) -> Result<ah_preview::Preview, ah_preview::Skip> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
            self.most_running.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(self.takes);
            self.running.fetch_sub(1, Ordering::SeqCst);
            Ok(ah_preview::Preview {
                avif: vec![7; self.len],
                width: 1,
                height: 1,
            })
        }
    }

    fn key(name: &str) -> PreviewKey {
        PreviewKey {
            path: PathBuf::from(name),
            size: 1,
            modified: None,
        }
    }

    fn made(len: usize, original_sha256: &str) -> Arc<Made> {
        Arc::new(Made {
            original_sha256: original_sha256.into(),
            outcome: Outcome::Preview(Arc::new(PreviewBytes {
                avif: vec![0; len],
                sha256: String::new(),
            })),
        })
    }

    fn waiter(phone: &str) -> Waiter {
        Waiter {
            phone_id: phone.into(),
            get: AssetGet {
                asset_id: "a".into(),
                from: 0,
                count: 1,
                variant: Some(VARIANT_PREVIEW.into()),
            },
            source: AssetSource {
                asset_id: "a".into(),
                path: PathBuf::from("a.png"),
                name: "a.png".into(),
                mime: "image/png".into(),
                size: 500_000,
                sha256: None,
            },
        }
    }

    fn previews(limits: PreviewLimits) -> Previews {
        Previews::with_limits(Counting::new(10, Duration::ZERO), limits)
    }

    /// The values the sizes and the times of the spec were measured with (AT2, AT10, AT12). Changing one changes what
    /// the phone downloads or what the Mac spends, so it is a decision, not an edit.
    #[test]
    fn the_limits_are_the_ones_the_spec_was_measured_with() {
        assert_eq!(PREVIEW_MAX_EDGE, 1600);
        assert_eq!(PREVIEW_QUALITY, 76.0);
        assert_eq!(PREVIEW_SPEED, 6);
        assert_eq!(PREVIEW_MIN_ORIGINAL_BYTES, 400 * 1024);
        let limits = PreviewLimits::default();
        assert_eq!(limits.entries, 32);
        assert_eq!(limits.bytes, 32 * 1024 * 1024);
        assert_eq!(limits.concurrent, 3);
    }

    #[test]
    fn only_png_jpeg_and_webp_files_of_enough_size_are_eligible() {
        let big = PREVIEW_MIN_ORIGINAL_BYTES;
        for name in ["a.png", "a.PNG", "photo.jpg", "photo.jpeg", "x.webp"] {
            assert!(eligible(name, big), "{name}");
        }
        for name in [
            "a.gif",
            "a.heic",
            "a.bmp",
            "a.svg",
            "a.pdf",
            "notes.txt",
            "noext",
        ] {
            assert!(!eligible(name, big), "{name}");
        }
        assert!(!eligible("a.png", big - 1));
        assert!(eligible("a.png", MAX_ASSET_BYTES));
        assert!(!eligible("a.png", MAX_ASSET_BYTES + 1));
    }

    #[test]
    fn a_remembered_result_is_found_by_the_state_of_the_file() {
        let mut previews = previews(PreviewLimits::default());
        previews.finish(&key("a"), Some(made(100, "h")));
        assert!(previews.lookup(&key("a")).is_some());
        assert!(previews.lookup(&key("b")).is_none());
        let mut changed = key("a");
        changed.size = 2;
        assert!(previews.lookup(&changed).is_none());
    }

    #[test]
    fn the_oldest_results_go_first_when_there_are_too_many() {
        let mut previews = previews(PreviewLimits {
            entries: 3,
            ..PreviewLimits::default()
        });
        for name in ["a", "b", "c", "d"] {
            previews.finish(&key(name), Some(made(10, "h")));
        }
        assert_eq!(previews.cached(), 3);
        assert!(previews.lookup(&key("a")).is_none());
        assert!(previews.lookup(&key("d")).is_some());
    }

    #[test]
    fn the_oldest_results_go_first_when_they_add_up_to_too_many_bytes() {
        let mut previews = previews(PreviewLimits {
            bytes: 800,
            ..PreviewLimits::default()
        });
        for name in ["a", "b", "c"] {
            previews.finish(&key(name), Some(made(400, "h")));
        }
        // Each result counts a little more than its 400 bytes, so two of them are over 800.
        assert_eq!(previews.cached(), 1);
        assert!(previews.lookup(&key("c")).is_some());
        assert!(previews.cached_bytes <= 800);
    }

    #[test]
    fn a_result_that_alone_is_over_the_byte_budget_is_not_kept() {
        let mut previews = previews(PreviewLimits {
            bytes: 100,
            ..PreviewLimits::default()
        });
        previews.finish(&key("a"), Some(made(500, "h")));
        assert_eq!(previews.cached(), 0);
        assert_eq!(previews.cached_bytes, 0);
    }

    #[test]
    fn remembering_a_key_again_replaces_it_without_counting_it_twice() {
        let mut previews = previews(PreviewLimits::default());
        previews.finish(&key("a"), Some(made(100, "h1")));
        previews.finish(&key("a"), Some(made(300, "h2")));
        assert_eq!(previews.cached(), 1);
        assert_eq!(previews.cached_bytes, 300 + 64);
        assert_eq!(previews.lookup(&key("a")).unwrap().original_sha256, "h2");
    }

    #[test]
    fn the_first_request_starts_the_work_and_the_others_wait_for_it() {
        let mut previews = previews(PreviewLimits::default());
        assert!(!previews.join(key("a"), waiter("p1")));
        assert!(previews.join(key("a"), waiter("p2")));
        assert!(!previews.join(key("b"), waiter("p3")));
        assert_eq!(previews.waiting(), 3);
        let waiters = previews.finish(&key("a"), None);
        assert_eq!(
            waiters
                .iter()
                .map(|w| w.phone_id.as_str())
                .collect::<Vec<_>>(),
            ["p1", "p2"]
        );
        assert_eq!(previews.waiting(), 1);
        // The work of "a" is over, so the next request for it starts it again.
        assert!(!previews.join(key("a"), waiter("p4")));
    }

    #[test]
    fn a_preview_is_for_a_request_only_when_the_file_is_the_one_it_was_built_from() {
        let result = made(10, "aa");
        let mut source = waiter("p").source;
        assert!(
            result.preview_for(&source).is_some(),
            "no recorded hash: the file as it is"
        );
        source.sha256 = Some("aa".into());
        assert!(result.preview_for(&source).is_some());
        source.sha256 = Some("bb".into());
        assert!(result.preview_for(&source).is_none());
        let skipped = Made {
            original_sha256: "aa".into(),
            outcome: Outcome::Original,
        };
        source.sha256 = None;
        assert!(skipped.preview_for(&source).is_none());
    }

    #[tokio::test]
    async fn generation_reads_hashes_and_makes_the_preview_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        std::fs::write(&path, b"original bytes").unwrap();
        let maker = Counting::new(20, Duration::ZERO);
        let previews = Previews::new(maker.clone());
        let Generated::Made(made) = previews.generation(path).await else {
            panic!("expected a result");
        };
        assert_eq!(made.original_sha256, sha256_hex(b"original bytes"));
        let Outcome::Preview(preview) = &made.outcome else {
            panic!("expected a preview");
        };
        assert_eq!(preview.avif, vec![7; 20]);
        assert_eq!(preview.sha256, sha256_hex(&preview.avif));
        assert_eq!(maker.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn generation_of_a_missing_or_oversized_file_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let previews = Previews::new(Counting::new(1, Duration::ZERO));
        assert!(matches!(
            previews.generation(dir.path().join("gone.png")).await,
            Generated::Unreadable
        ));
        let big = dir.path().join("big.png");
        std::fs::write(&big, vec![0u8; MAX_ASSET_BYTES as usize + 1]).unwrap();
        assert!(matches!(
            previews.generation(big).await,
            Generated::TooLarge
        ));
    }

    #[tokio::test]
    async fn no_more_previews_are_made_at_once_than_the_limit_allows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        std::fs::write(&path, b"x").unwrap();
        let maker = Counting::new(1, Duration::from_millis(40));
        let previews = Previews::with_limits(
            maker.clone(),
            PreviewLimits {
                concurrent: 2,
                ..PreviewLimits::default()
            },
        );
        let all: Vec<_> = (0..6)
            .map(|_| tokio::spawn(previews.generation(path.clone())))
            .collect();
        for handle in all {
            assert!(matches!(handle.await.unwrap(), Generated::Made(_)));
        }
        assert_eq!(maker.calls.load(Ordering::SeqCst), 6);
        assert_eq!(maker.most_running.load(Ordering::SeqCst), 2);
    }
}
