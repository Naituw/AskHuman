//! 监听 `~/.askhuman/config.json` 变更（`notify` + 去抖）。
//!
//! 本模块只做「文件变更 → 去抖脉冲」，与 Daemon 内部状态解耦：调用 `spawn()` 得到一个接收端，
//! 每当 config.json 稳定写入后收到一个 `()`。具体「重载/失效 Router/通知 GUI」由 daemon 处理。
//!
//! 监听的是**配置目录**（非单个文件）：配置用「写临时文件 + rename」原子落盘，rename 会替换 inode，
//! 直接 watch 文件在首次替换后即失效；watch 目录再按文件名过滤最稳。daemon.log 等同目录写入被过滤掉。

use std::ffi::OsStr;
use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

/// Whether a watcher event means the file called `name` **really changed** on disk.
///
/// notify's inotify backend subscribes to `IN_OPEN` and `IN_CLOSE_WRITE` and reports them as
/// `EventKind::Access` (`Access(Open)` / `Access(Close(Write))`). Counting those as changes would
/// let every read of the file trigger the next reload, and the reload reads the file again: a
/// read-triggered feedback loop. For config.json the daemon's own periodic `load_without_secrets()`
/// starts it; `todos.json` and `history.jsonl` are read by the very window that reloads on the
/// event, so there it would sustain itself. A real change always comes with a non-`Access` event
/// (`Modify`, `Create`, `Remove` or a rename), so only those count.
///
/// Every watcher that watches a directory and filters its events by file name goes through this.
pub(crate) fn is_file_change(event: &notify::Event, name: impl AsRef<OsStr>) -> bool {
    let name = name.as_ref();
    let touches_file = event.paths.iter().any(|p| p.file_name() == Some(name));
    touches_file && !matches!(event.kind, notify::EventKind::Access(_))
}

/// [`is_file_change`] for config.json.
pub(crate) fn is_config_change(event: &notify::Event) -> bool {
    is_file_change(event, "config.json")
}

/// 启动监听，返回「去抖后的变更脉冲」接收端。
pub fn spawn() -> UnboundedReceiver<()> {
    let (raw_tx, mut raw_rx) = unbounded_channel::<()>();

    // The notify watcher runs on a dedicated thread; its callback lets only real writes to
    // config.json through (reads are not changes).
    std::thread::spawn(move || {
        use notify::{RecursiveMode, Watcher};
        let dir = crate::paths::config_dir();
        let _ = std::fs::create_dir_all(&dir);
        let mut watcher =
            match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(ev) = res {
                    if is_config_change(&ev) {
                        let _ = raw_tx.send(());
                    }
                }
            }) {
                Ok(w) => w,
                Err(_) => return,
            };
        if watcher.watch(&dir, RecursiveMode::NonRecursive).is_err() {
            return;
        }
        // 阻塞保活：watcher 持有回调（含 raw_tx），park 住本线程直到进程退出。
        loop {
            std::thread::park();
        }
    });

    // 去抖：首个脉冲后等 300ms 静默，再向上层发一次（合并原子写产生的多个 rename/modify 事件）。
    let (out_tx, out_rx) = unbounded_channel::<()>();
    tokio::spawn(async move {
        while raw_rx.recv().await.is_some() {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(300)) => break,
                    m = raw_rx.recv() => {
                        if m.is_none() {
                            return;
                        }
                    }
                }
            }
            if out_tx.send(()).is_err() {
                break;
            }
        }
    });

    out_rx
}

#[cfg(test)]
mod tests {
    use super::{is_config_change, is_file_change};
    use notify::event::{
        AccessKind, AccessMode, CreateKind, DataChange, Event, EventKind, ModifyKind, RemoveKind,
        RenameMode,
    };
    use std::path::PathBuf;

    fn event(kind: EventKind, name: &str) -> Event {
        Event::new(kind).add_path(PathBuf::from("/tmp/askhuman").join(name))
    }

    #[test]
    fn reads_are_not_config_changes() {
        // inotify reports IN_OPEN as an Access event; an Access event only says that someone read
        // or closed the file.
        assert!(!is_config_change(&event(
            EventKind::Access(AccessKind::Open(AccessMode::Any)),
            "config.json"
        )));
        assert!(!is_config_change(&event(
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
            "config.json"
        )));
    }

    #[test]
    fn writes_to_config_are_changes() {
        assert!(is_config_change(&event(
            EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            "config.json"
        )));
        // An atomic save writes a temp file and renames it over config.json; a directory watch
        // sees the rename's `To` event.
        assert!(is_config_change(&event(
            EventKind::Modify(ModifyKind::Name(RenameMode::To)),
            "config.json"
        )));
        assert!(is_config_change(&event(
            EventKind::Create(CreateKind::File),
            "config.json"
        )));
        assert!(is_config_change(&event(
            EventKind::Remove(RemoveKind::File),
            "config.json"
        )));
    }

    #[test]
    fn other_files_are_ignored() {
        assert!(!is_config_change(&event(
            EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            "daemon.log"
        )));
        assert!(!is_config_change(&event(
            EventKind::Access(AccessKind::Open(AccessMode::Any)),
            "agents.json"
        )));
    }

    /// The other files watched by name: each is re-read by the code that reacts to its events.
    const WATCHED: [&str; 3] = ["todos.json", "history.jsonl", "daemon.json"];

    #[test]
    fn reads_and_closes_of_the_other_watched_files_are_not_changes() {
        for name in WATCHED {
            for kind in [
                EventKind::Access(AccessKind::Open(AccessMode::Any)),
                EventKind::Access(AccessKind::Close(AccessMode::Read)),
                // A close after writing follows the write's own Modify event.
                EventKind::Access(AccessKind::Close(AccessMode::Write)),
            ] {
                assert!(!is_file_change(&event(kind, name), name), "{name}");
            }
        }
    }

    #[test]
    fn writes_to_the_other_watched_files_are_changes() {
        for name in WATCHED {
            for kind in [
                EventKind::Modify(ModifyKind::Data(DataChange::Any)),
                EventKind::Modify(ModifyKind::Name(RenameMode::To)),
                EventKind::Create(CreateKind::File),
                EventKind::Remove(RemoveKind::File),
            ] {
                assert!(is_file_change(&event(kind, name), name), "{name}");
            }
        }
    }

    #[test]
    fn a_file_is_only_matched_by_its_own_name() {
        let write = EventKind::Modify(ModifyKind::Data(DataChange::Any));
        assert!(!is_file_change(
            &event(write, "todos.json"),
            "history.jsonl"
        ));
        // The temp file of an atomic save sits next to the target with a longer name.
        assert!(!is_file_change(
            &event(EventKind::Create(CreateKind::File), "todos.json.tmp-0b9d"),
            "todos.json"
        ));
        assert!(!is_file_change(
            &event(write, "old-todos.json"),
            "todos.json"
        ));
    }
}
