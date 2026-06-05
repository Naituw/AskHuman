//! 语音输入桥（macOS 26 SpeechAnalyzer）。
//!
//! Swift 侧 `@objc(AHSpeechBridge)` 封装真正的识别（SpeechAnalyzer/SpeechTranscriber）。
//! 这里用 objc2 实例化该类、用 block2 把回调闭包(捕获 AppHandle)设进去，
//! 闭包内将结果转发为 Tauri 事件供前端消费。
//!
//! 线程模型：start/stop/flush 由命令线程调用；Swift 内部在自身音频/识别线程回调，
//! 闭包内只做 `app.emit`（线程安全）。会话状态(含非 Send 的 Retained/RcBlock)只在
//! 加锁后短暂访问，并以 `unsafe impl Send` 显式断言（与原 macos_speech 一致）。

use std::sync::Mutex;

use block2::RcBlock;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::NSString;
use tauri::{AppHandle, Emitter};

/// 已最终化的文本片段（插入到当前光标处）。
const EVENT_COMMITTED: &str = "speech-committed";
/// 当前实时片段（就地替换显示）。
const EVENT_VOLATILE: &str = "speech-volatile";
/// 输入电平峰值（0~1，录音动效）。
const EVENT_LEVEL: &str = "speech-level";
/// 人类可读状态（识别语言/下载/聆听中…）。
const EVENT_STATUS: &str = "speech-status";
/// 错误文案（不支持/未授权/下载失败等）。
const EVENT_ERROR: &str = "speech-error";
/// 会话结束（前端复位录音 UI）。
const EVENT_STOPPED: &str = "speech-stopped";
/// 识别已就绪、真正进入实时录制（前端从 loading 切到高亮）。
const EVENT_READY: &str = "speech-ready";

/// 一次语音会话：保活桥对象与各回调 block（Swift 已 retain，但一并持有更稳妥）。
struct SpeechSession {
    bridge: Retained<AnyObject>,
    _committed: RcBlock<dyn Fn(*mut NSString)>,
    _volatile: RcBlock<dyn Fn(*mut NSString)>,
    _level: RcBlock<dyn Fn(f32)>,
    _status: RcBlock<dyn Fn(*mut NSString)>,
    _error: RcBlock<dyn Fn(*mut NSString)>,
    _stopped: RcBlock<dyn Fn()>,
    _ready: RcBlock<dyn Fn()>,
}

// 仅在加锁后短暂访问；Swift 侧回调不回穿到此结构。显式断言可跨线程持有。
unsafe impl Send for SpeechSession {}

static SESSION: Mutex<Option<SpeechSession>> = Mutex::new(None);

fn ns_to_string(s: *mut NSString) -> String {
    if s.is_null() {
        String::new()
    } else {
        unsafe { (*s).to_string() }
    }
}

/// Swift 侧运行期是否可用（macOS 26+）。
pub fn is_available() -> bool {
    let Some(cls) = AnyClass::get(c"AHSpeechBridge") else {
        return false;
    };
    unsafe { msg_send![cls, isAvailable] }
}

/// 开始语音输入：`locale` 为 BCP-47（如 zh-CN），空串=跟随系统。
pub fn start(app: AppHandle, locale: &str) {
    if !is_available() {
        let _ = app.emit(EVENT_ERROR, "语音输入需要 macOS 26 及以上");
        return;
    }
    let Some(cls) = AnyClass::get(c"AHSpeechBridge") else {
        let _ = app.emit(EVENT_ERROR, "语音桥未就绪");
        return;
    };

    // 构造回调 block：闭包捕获 AppHandle，直接 emit。
    let committed = {
        let app = app.clone();
        RcBlock::new(move |s: *mut NSString| {
            let _ = app.emit(EVENT_COMMITTED, ns_to_string(s));
        })
    };
    let volatile = {
        let app = app.clone();
        RcBlock::new(move |s: *mut NSString| {
            let _ = app.emit(EVENT_VOLATILE, ns_to_string(s));
        })
    };
    let level = {
        let app = app.clone();
        RcBlock::new(move |v: f32| {
            let _ = app.emit(EVENT_LEVEL, v);
        })
    };
    let status = {
        let app = app.clone();
        RcBlock::new(move |s: *mut NSString| {
            let _ = app.emit(EVENT_STATUS, ns_to_string(s));
        })
    };
    let error = {
        let app = app.clone();
        RcBlock::new(move |s: *mut NSString| {
            let _ = app.emit(EVENT_ERROR, ns_to_string(s));
        })
    };
    let stopped = {
        let app = app.clone();
        RcBlock::new(move || {
            let _ = app.emit(EVENT_STOPPED, ());
        })
    };
    let ready = {
        let app = app.clone();
        RcBlock::new(move || {
            let _ = app.emit(EVENT_READY, ());
        })
    };

    let bridge: Retained<AnyObject> = unsafe { msg_send![cls, new] };
    let loc = NSString::from_str(locale);
    unsafe {
        let _: () = msg_send![&*bridge, setOnCommitted: &*committed];
        let _: () = msg_send![&*bridge, setOnVolatile: &*volatile];
        let _: () = msg_send![&*bridge, setOnLevel: &*level];
        let _: () = msg_send![&*bridge, setOnStatus: &*status];
        let _: () = msg_send![&*bridge, setOnError: &*error];
        let _: () = msg_send![&*bridge, setOnStopped: &*stopped];
        let _: () = msg_send![&*bridge, setOnReady: &*ready];
        let _: () = msg_send![&*bridge, start: &*loc];
    }

    *SESSION.lock().unwrap() = Some(SpeechSession {
        bridge,
        _committed: committed,
        _volatile: volatile,
        _level: level,
        _status: status,
        _error: error,
        _stopped: stopped,
        _ready: ready,
    });
}

/// 停止当前会话。
pub fn stop() {
    let guard = SESSION.lock().unwrap();
    if let Some(s) = guard.as_ref() {
        unsafe {
            let _: () = msg_send![&*s.bridge, stop];
        }
    }
    // Swift 的 stop 是异步并在其 Task 内持有 bridge，故这里可安全释放 Rust 侧持有。
    drop(guard);
    *SESSION.lock().unwrap() = None;
}

/// 固定已写文本并重启识别会话（用户听写中途移动光标时）。
pub fn flush() {
    let guard = SESSION.lock().unwrap();
    if let Some(s) = guard.as_ref() {
        unsafe {
            let _: () = msg_send![&*s.bridge, flush];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speech_bridge_ping() {
        // 验证：Swift 静态库链入成功 + @objc 类注册 + objc2 调用通路。
        let cls = AnyClass::get(c"AHSpeechBridge").expect("AHSpeechBridge 类未注册");
        let s: String = unsafe {
            let p: *mut NSString = msg_send![cls, ping];
            ns_to_string(p)
        };
        assert_eq!(s, "pong");
    }
}
