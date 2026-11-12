//! A session's earlier exchanges for the phone, Mac side (docs/specs/ios-session-history.md §6.2), and the
//! pictures in them (docs/specs/ios-history-pictures.md §6).
//!
//! Reads the reply history (`history.rs`) of one exact native Agent session, the same
//! definition of "a real session" as `reply-history.md` F1, and projects every entry with the pure
//! `ios::history_view`. The read is blocking file IO, so it runs in `spawn_blocking`. This is also the
//! only place that looks at the picture files themselves ([`DiskPictures`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::history::{HistoryEntry, SendsPage};
use crate::ios::history_view::{exchange_of, exchange_with_pictures, find_picture, PictureProbe};
use crate::ios::views::{history_thumbnail, mime_for, split_agent_key, AssetSource};
use crate::ios::wire::{self, SESSION_HISTORY_PAGE_MAX};

/// Whether the history is switched on. Only the general settings are needed, so the keychain is not read.
fn history_enabled() -> bool {
    crate::config::AppConfig::load_without_secrets()
        .general
        .history_limit
        > 0
}

/// The page for one `history.session.get`. An unreadable key, a session without records and a
/// history that is switched off all give an empty page, never an error.
pub(super) async fn session_history(get: &wire::SessionHistoryGet) -> wire::SessionHistory {
    let get = get.clone();
    let fallback = wire::SessionHistory::empty(&get.req_id, &get.agent_key);
    tokio::task::spawn_blocking(move || {
        page_with(
            &get,
            history_enabled(),
            crate::history::sends_for_session_before,
            &DiskPictures,
        )
    })
    .await
    .unwrap_or(fallback)
}

/// The answer for one `history.exchange.get`: the recorded exchange with its pictures, or none when the
/// Mac has no such record or the history is switched off.
pub(super) async fn history_exchange(get: &wire::HistoryExchangeGet) -> wire::HistoryExchange {
    let get = get.clone();
    let fallback = wire::HistoryExchange::empty(&get.req_id, &get.id);
    tokio::task::spawn_blocking(move || {
        exchange_with(
            &get,
            history_enabled(),
            crate::history::entry_by_id,
            &DiskPictures,
        )
    })
    .await
    .unwrap_or(fallback)
}

/// The file behind the picture the phone asked for with `asset.get`, found by the id the Mac gave it in an
/// exchange (docs/specs/ios-history-pictures.md HP7). `None` for an id that was never given out, a history
/// that is switched off, and a file that is not there any more.
pub(super) async fn history_picture(asset_id: &str) -> Option<AssetSource> {
    let asset_id = asset_id.to_string();
    tokio::task::spawn_blocking(move || {
        picture_source(
            &asset_id,
            history_enabled(),
            || crate::history::load(None, true),
            &DiskPictures,
        )
    })
    .await
    .ok()
    .flatten()
}

/// `read` fetches the exchanges older than a cursor: `(agent kind, session id, before_ms, before_id,
/// limit)`. The daemon passes the history file's reader; the tests pass a script. The pictures are only
/// looked at when the phone asked for them.
fn page_with(
    get: &wire::SessionHistoryGet,
    enabled: bool,
    read: impl FnOnce(&str, &str, i64, Option<&str>, usize) -> SendsPage,
    probe: &dyn PictureProbe,
) -> wire::SessionHistory {
    let mut page = wire::SessionHistory::empty(&get.req_id, &get.agent_key);
    let Some((kind, session_id)) = split_agent_key(&get.agent_key) else {
        return page;
    };
    if !enabled {
        return page;
    }
    let limit = (get.limit as usize).clamp(1, SESSION_HISTORY_PAGE_MAX);
    let found = read(
        kind.as_str(),
        session_id,
        get.before_ms,
        get.before_id.as_deref(),
        limit,
    );
    page.items = found
        .entries
        .iter()
        .map(|entry| {
            if get.pictures {
                exchange_with_pictures(entry, probe)
            } else {
                exchange_of(entry)
            }
        })
        .collect();
    page.more = found.more;
    page
}

/// One exchange by its id, `find` being the history's reader.
fn exchange_with(
    get: &wire::HistoryExchangeGet,
    enabled: bool,
    find: impl FnOnce(&str) -> Option<HistoryEntry>,
    probe: &dyn PictureProbe,
) -> wire::HistoryExchange {
    let mut answer = wire::HistoryExchange::empty(&get.req_id, &get.id);
    if enabled {
        answer.item = find(&get.id).map(|entry| exchange_with_pictures(&entry, probe));
    }
    answer
}

/// The picture `asset_id` stands for, as the phone may fetch it, from `entries` (the history's reader).
fn picture_source(
    asset_id: &str,
    enabled: bool,
    entries: impl FnOnce() -> Vec<HistoryEntry>,
    probe: &dyn PictureProbe,
) -> Option<AssetSource> {
    if !enabled {
        return None;
    }
    let entries = entries();
    let found = find_picture(&entries, asset_id)?;
    let size = probe.size(&found.path)?;
    Some(AssetSource {
        asset_id: asset_id.to_string(),
        mime: mime_for(&found.name).to_string(),
        name: found.name,
        path: found.path,
        size,
    })
}

// ---------------------------------------------------------------- the files

/// What identifies one version of a picture file: its path, its size and when it was modified.
type ThumbKey = (PathBuf, u64, u128);

/// How many thumbnails are kept: about 6 KiB each, so a few megabytes at most.
const THUMB_CACHE_MAX: usize = 256;

/// The thumbnails already made. A page carries the same pictures again each time the phone opens a
/// session or a record, and decoding a screenshot takes a few milliseconds, so they are kept. An entry
/// of `None` is a file that cannot be decoded, which is not worth trying again either.
#[derive(Default)]
struct ThumbCache {
    made: HashMap<ThumbKey, Option<Vec<u8>>>,
}

impl ThumbCache {
    fn get(&self, key: &ThumbKey) -> Option<Option<Vec<u8>>> {
        self.made.get(key).cloned()
    }

    /// Remembers a thumbnail. When the cache is full it starts over: the pictures a phone looks at
    /// together are few, so keeping the most recent ones is not worth bookkeeping.
    fn put(&mut self, key: ThumbKey, thumb: Option<Vec<u8>>) {
        if self.made.len() >= THUMB_CACHE_MAX && !self.made.contains_key(&key) {
            self.made.clear();
        }
        self.made.insert(key, thumb);
    }
}

fn thumb_cache() -> &'static Mutex<ThumbCache> {
    static CACHE: OnceLock<Mutex<ThumbCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(ThumbCache::default()))
}

/// The pictures as they are on this Mac's disk.
pub(super) struct DiskPictures;

fn regular_file(path: &Path) -> Option<std::fs::Metadata> {
    std::fs::metadata(path).ok().filter(|meta| meta.is_file())
}

impl PictureProbe for DiskPictures {
    fn size(&self, path: &Path) -> Option<u64> {
        regular_file(path).map(|meta| meta.len())
    }

    fn thumbnail(&self, path: &Path) -> Option<Vec<u8>> {
        let meta = regular_file(path)?;
        let modified = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let key = (path.to_path_buf(), meta.len(), modified);
        if let Some(hit) = thumb_cache().lock().ok()?.get(&key) {
            return hit;
        }
        // Decoding happens outside the lock: two pages asking for the same picture at once make it twice
        // rather than one waiting for the other.
        let made = history_thumbnail(path);
        if let Ok(mut cache) = thumb_cache().lock() {
            cache.put(key, made.clone());
        }
        made
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::history::HistoryEntry;
    use crate::ios::history_view::{picture_id, PictureSlot};
    use crate::models::{ChannelAction, FileAttachment, MessagePrompt, Question};

    fn get(key: &str, limit: u32) -> wire::SessionHistoryGet {
        wire::SessionHistoryGet {
            req_id: "7".into(),
            agent_key: key.into(),
            before_ms: 1_000,
            before_id: Some("cursor-id".into()),
            limit,
            pictures: false,
        }
    }

    fn entry(id: &str, at: i64) -> HistoryEntry {
        HistoryEntry {
            id: id.into(),
            timestamp_ms: at,
            project: "/work".into(),
            source: "Codex".into(),
            agent_kind: Some("codex".into()),
            agent_session_id: Some("s:1".into()),
            mcp_instance_id: None,
            channel: "popup".into(),
            action: ChannelAction::Send,
            is_markdown: false,
            message: MessagePrompt::new("hello".into(), Vec::new()),
            questions: vec![Question::new("ok?".into(), Vec::new())],
            answers: Vec::new(),
            annotations: Vec::new(),
        }
    }

    /// An entry whose message carries two images and a PDF, and whose answer carries one image.
    fn entry_with_files(id: &str) -> HistoryEntry {
        let mut recorded = entry(id, 500);
        recorded.message.files = ["a.png", "plan.pdf", "b.png"]
            .iter()
            .map(|name| FileAttachment {
                path: format!("/work/{name}"),
                name: (*name).into(),
                size: 1,
                is_image: name.ends_with(".png"),
            })
            .collect();
        recorded.answers = vec![crate::history::HistoryAnswer {
            images: vec!["/tmp/reply.png".into()],
            ..Default::default()
        }];
        recorded
    }

    /// A probe over a made-up disk: the files that exist, with their size. It records every file it was
    /// asked a thumbnail of, and says every thumbnail is the same few bytes.
    #[derive(Default)]
    struct Disk {
        files: HashMap<String, u64>,
        thumbnails: RefCell<Vec<String>>,
    }

    impl Disk {
        fn with(files: &[(&str, u64)]) -> Self {
            Self {
                files: files.iter().map(|(p, s)| (p.to_string(), *s)).collect(),
                ..Default::default()
            }
        }
    }

    impl PictureProbe for Disk {
        fn size(&self, path: &Path) -> Option<u64> {
            self.files.get(path.to_str().unwrap()).copied()
        }
        fn thumbnail(&self, path: &Path) -> Option<Vec<u8>> {
            self.thumbnails
                .borrow_mut()
                .push(path.to_str().unwrap().to_string());
            Some(b"jpeg".to_vec())
        }
    }

    type Call = (String, String, i64, Option<String>, usize);

    /// Runs `page_with` against a script and returns the page with the calls the reader saw.
    fn run(
        get: &wire::SessionHistoryGet,
        enabled: bool,
        found: SendsPage,
    ) -> (wire::SessionHistory, Vec<Call>) {
        run_on(get, enabled, found, &Disk::default())
    }

    fn run_on(
        get: &wire::SessionHistoryGet,
        enabled: bool,
        found: SendsPage,
        probe: &dyn PictureProbe,
    ) -> (wire::SessionHistory, Vec<Call>) {
        let calls = RefCell::new(Vec::new());
        let page = page_with(
            get,
            enabled,
            |kind, session, ms, id, limit| {
                calls.borrow_mut().push((
                    kind.to_string(),
                    session.to_string(),
                    ms,
                    id.map(str::to_string),
                    limit,
                ));
                found
            },
            probe,
        );
        (page, calls.into_inner())
    }

    fn found(ids: &[&str], more: bool) -> SendsPage {
        SendsPage {
            entries: ids
                .iter()
                .enumerate()
                .map(|(index, id)| entry(id, 500 - index as i64))
                .collect(),
            more,
        }
    }

    #[test]
    fn the_cursor_and_the_exact_session_reach_the_reader_and_the_page_echoes_the_request() {
        // The session id keeps its own colons.
        let (page, calls) = run(&get("codex:s:1", 8), true, found(&["b", "a"], true));
        assert_eq!(
            calls,
            [(
                "codex".to_string(),
                "s:1".to_string(),
                1_000,
                Some("cursor-id".to_string()),
                8
            )]
        );
        assert_eq!(page.req_id, "7");
        assert_eq!(page.agent_key, "codex:s:1");
        assert_eq!(
            page.items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert!(page.more);
        assert_eq!(page.items[0].message, "hello");
        assert_eq!(page.items[0].qs[0].text, "ok?");
    }

    #[test]
    fn the_limit_is_kept_within_range() {
        let (_, calls) = run(&get("codex:s:1", 0), true, found(&[], false));
        assert_eq!(calls[0].4, 1);
        let (_, calls) = run(&get("codex:s:1", 500), true, found(&[], false));
        assert_eq!(calls[0].4, SESSION_HISTORY_PAGE_MAX);
    }

    #[test]
    fn a_key_that_is_not_a_session_gives_an_empty_page_and_reads_nothing() {
        for key in ["", "codex", "codex:", ":s1", "nope:s1", "mcp:instance"] {
            let (page, calls) = run(&get(key, 8), true, found(&["a"], true));
            assert!(calls.is_empty(), "{key} was read");
            assert!(page.items.is_empty() && !page.more, "{key} is not empty");
            assert_eq!(page.req_id, "7");
            assert_eq!(page.agent_key, key);
        }
    }

    #[test]
    fn a_switched_off_history_gives_an_empty_page_and_reads_nothing() {
        let (page, calls) = run(&get("codex:s1", 8), false, found(&["a"], true));
        assert!(calls.is_empty());
        assert!(page.items.is_empty() && !page.more);
    }

    // ------------------------------------------------------------ pictures

    fn page_of(entries: Vec<HistoryEntry>) -> SendsPage {
        SendsPage {
            entries,
            more: false,
        }
    }

    #[test]
    fn a_page_carries_pictures_only_when_the_phone_asked_for_them() {
        let disk = Disk::with(&[
            ("/work/a.png", 10),
            ("/work/b.png", 20),
            ("/tmp/reply.png", 5),
        ]);
        let mut request = get("codex:s:1", 8);

        // Not asked for: the page is the plain one and the files are not even looked at.
        let (plain, _) = run_on(&request, true, page_of(vec![entry_with_files("x")]), &disk);
        assert!(plain.items[0].pictures.is_empty());
        assert!(plain.items[0].qs[0].pictures.is_empty());
        assert_eq!(plain.items[0].attachments, 3, "still counted");
        assert!(disk.thumbnails.borrow().is_empty());

        request.pictures = true;
        let (rich, _) = run_on(&request, true, page_of(vec![entry_with_files("x")]), &disk);
        let item = &rich.items[0];
        assert_eq!(
            item.pictures
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["a.png", "b.png"]
        );
        assert_eq!(item.pictures[0].size, 10);
        assert!(item.pictures[0].thumb.is_some());
        assert_eq!(item.qs[0].pictures[0].name, "reply.png");
        assert_eq!(item.attachments, 3);
        assert_eq!(item.qs[0].images, 1);
        assert_eq!(
            item.pictures[0].asset_id,
            picture_id("x", PictureSlot::Message(0))
        );
    }

    #[test]
    fn a_missing_file_makes_a_gone_picture_not_a_missing_one() {
        let disk = Disk::with(&[("/work/a.png", 10)]);
        let mut request = get("codex:s:1", 8);
        request.pictures = true;
        let (page, _) = run_on(&request, true, page_of(vec![entry_with_files("x")]), &disk);
        let item = &page.items[0];
        assert!(!item.pictures[0].gone);
        assert!(item.pictures[1].gone, "b.png is not on the disk");
        assert!(item.qs[0].pictures[0].gone, "neither is the reply image");
        assert_eq!(item.pictures.len(), 2, "the picture is still listed");
    }

    fn exchange_get(id: &str) -> wire::HistoryExchangeGet {
        wire::HistoryExchangeGet {
            req_id: "x9".into(),
            id: id.into(),
        }
    }

    #[test]
    fn an_exchange_is_found_by_its_id_with_its_pictures_and_echoes_the_request() {
        let disk = Disk::with(&[
            ("/work/a.png", 10),
            ("/work/b.png", 20),
            ("/tmp/reply.png", 5),
        ]);
        let asked = RefCell::new(Vec::new());
        let answer = exchange_with(
            &exchange_get("req-7"),
            true,
            |id| {
                asked.borrow_mut().push(id.to_string());
                Some(entry_with_files(id))
            },
            &disk,
        );
        assert_eq!(asked.into_inner(), ["req-7"]);
        assert_eq!(
            (answer.req_id.as_str(), answer.id.as_str()),
            ("x9", "req-7")
        );
        let item = answer.item.expect("the record");
        assert_eq!(item.id, "req-7");
        assert_eq!(item.pictures.len(), 2);
        assert_eq!(item.qs[0].pictures.len(), 1);
    }

    #[test]
    fn an_exchange_the_mac_does_not_have_or_may_not_show_has_no_item() {
        let disk = Disk::default();
        let none = exchange_with(&exchange_get("gone"), true, |_| None, &disk);
        assert!(none.item.is_none());
        assert_eq!((none.req_id.as_str(), none.id.as_str()), ("x9", "gone"));

        let asked = RefCell::new(false);
        let off = exchange_with(
            &exchange_get("req-7"),
            false,
            |id| {
                *asked.borrow_mut() = true;
                Some(entry_with_files(id))
            },
            &disk,
        );
        assert!(off.item.is_none());
        assert!(!asked.into_inner(), "a switched-off history is not read");
    }

    #[test]
    fn a_picture_id_resolves_to_its_file_with_name_type_and_current_size() {
        let disk = Disk::with(&[("/work/b.png", 777), ("/tmp/reply.png", 5)]);
        let recorded = entry_with_files("req-7");
        let id = picture_id("req-7", PictureSlot::Message(2));
        let source = picture_source(&id, true, || vec![recorded.clone()], &disk).expect("found");
        assert_eq!(source.asset_id, id);
        assert_eq!(source.path, PathBuf::from("/work/b.png"));
        assert_eq!(
            (source.name.as_str(), source.mime.as_str(), source.size),
            ("b.png", "image/png", 777)
        );

        let reply = picture_id(
            "req-7",
            PictureSlot::Reply {
                question: 0,
                index: 0,
            },
        );
        let source = picture_source(&reply, true, || vec![recorded.clone()], &disk).unwrap();
        assert_eq!((source.name.as_str(), source.size), ("reply.png", 5));
    }

    #[test]
    fn an_id_that_was_not_given_out_or_whose_file_is_gone_does_not_resolve() {
        let disk = Disk::with(&[("/work/b.png", 777)]);
        let recorded = entry_with_files("req-7");
        let resolve =
            |id: &str, enabled: bool| picture_source(id, enabled, || vec![recorded.clone()], &disk);
        // a.png is in the history but no longer on the disk.
        assert!(resolve(&picture_id("req-7", PictureSlot::Message(0)), true).is_none());
        // The PDF is not a picture; a stranger's id and garbage are nobody's.
        assert!(resolve(&picture_id("req-7", PictureSlot::Message(1)), true).is_none());
        assert!(resolve(&picture_id("req-8", PictureSlot::Message(2)), true).is_none());
        assert!(resolve("nonsense", true).is_none());
        // Switched off: even a good id is not served, and the history is not read.
        let good = picture_id("req-7", PictureSlot::Message(2));
        assert!(resolve(&good, true).is_some());
        let read = RefCell::new(false);
        assert!(picture_source(
            &good,
            false,
            || {
                *read.borrow_mut() = true;
                vec![recorded.clone()]
            },
            &disk
        )
        .is_none());
        assert!(!read.into_inner());
    }

    // ------------------------------------------------------------ the disk

    fn png(path: &Path, width: u32, height: u32) {
        image::RgbImage::from_fn(width, height, |x, y| {
            if (x / 30 + y / 30) % 5 == 0 {
                image::Rgb([20, 90, 220])
            } else {
                image::Rgb([245, 245, 250])
            }
        })
        .save(path)
        .unwrap();
    }

    #[test]
    fn the_disk_knows_regular_files_only() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.png");
        png(&file, 40, 40);
        let disk = DiskPictures;
        assert_eq!(
            disk.size(&file),
            Some(std::fs::metadata(&file).unwrap().len())
        );
        assert_eq!(disk.size(dir.path()), None, "a directory is not a picture");
        assert_eq!(disk.size(&dir.path().join("missing.png")), None);
        assert!(disk.thumbnail(dir.path()).is_none());
        assert!(disk.thumbnail(&dir.path().join("missing.png")).is_none());
    }

    #[test]
    fn a_thumbnail_is_made_once_and_made_again_when_the_file_changes() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("shot.png");
        png(&file, 400, 300);
        let disk = DiskPictures;
        let first = disk.thumbnail(&file).expect("a thumbnail");
        assert_eq!(&first[..2], &[0xff, 0xd8], "JPEG");
        assert!(first.len() <= wire::HISTORY_THUMB_MAX_BYTES);
        assert_eq!(disk.thumbnail(&file), Some(first.clone()), "cached");

        // A different picture under the same name is not served the old thumbnail.
        png(&file, 90, 300);
        let second = disk.thumbnail(&file).expect("a thumbnail");
        assert_ne!(second, first);

        // A file that is no picture has none, and asking again changes nothing.
        let fake = dir.path().join("fake.png");
        std::fs::write(&fake, b"not a picture").unwrap();
        assert!(disk.thumbnail(&fake).is_none());
        assert!(disk.thumbnail(&fake).is_none());
    }

    #[test]
    fn the_thumbnail_cache_starts_over_when_it_is_full() {
        let key = |n: usize| -> ThumbKey { (PathBuf::from(format!("/p/{n}.png")), n as u64, 0) };
        let mut cache = ThumbCache::default();
        for n in 0..THUMB_CACHE_MAX {
            cache.put(key(n), Some(vec![n as u8]));
        }
        assert_eq!(cache.get(&key(0)), Some(Some(vec![0])));
        // Replacing a key does not count as growth.
        cache.put(key(1), Some(vec![9]));
        assert_eq!(cache.made.len(), THUMB_CACHE_MAX);
        assert_eq!(cache.get(&key(1)), Some(Some(vec![9])));
        // A new key in a full cache clears it first, and the new one is kept.
        cache.put(key(THUMB_CACHE_MAX), None);
        assert_eq!(cache.made.len(), 1);
        assert_eq!(cache.get(&key(THUMB_CACHE_MAX)), Some(None));
        assert_eq!(cache.get(&key(0)), None);
    }
}
