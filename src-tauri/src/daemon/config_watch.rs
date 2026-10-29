//! 监听 `~/.askhuman/config.json` 变更（`notify` + 去抖）。
//!
//! 本模块只做「文件变更 → 去抖脉冲」，与 Daemon 内部状态解耦：调用 `spawn()` 得到一个接收端，
//! 每当 config.json 稳定写入后收到一个 `()`。具体「重载/失效 Router/通知 GUI」由 daemon 处理。
//!
//! 监听的是**配置目录**（非单个文件）：配置用「写临时文件 + rename」原子落盘，rename 会替换 inode，
//! 直接 watch 文件在首次替换后即失效；watch 目录再按文件名过滤最稳。daemon.log 等同目录写入被过滤掉。

use std::time::Duration;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

/// Whether a watcher event means config.json **really changed** on disk.
///
/// notify's inotify backend subscribes to `IN_OPEN` and `IN_CLOSE_WRITE` and reports them as
/// `EventKind::Access` (`Access(Open)` / `Access(Close(Write))`). Counting those as changes would
/// let every read of the file — the daemon's own periodic `load_without_secrets()` as well as the
/// re-read inside a reload — trigger the next reload, a read-triggered feedback loop. A real change
/// always comes with a non-`Access` event (`Modify`, `Create`, `Remove` or a rename), so only those
/// count.
pub(crate) fn is_config_change(event: &notify::Event) -> bool {
    let touches_config = event
        .paths
        .iter()
        .any(|p| p.file_name().map(|n| n == "config.json").unwrap_or(false));
    touches_config && !matches!(event.kind, notify::EventKind::Access(_))
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
    use super::is_config_change;
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
}
