//! macOS 原生语音输入（离线）：`SFSpeechRecognizer` + `AVAudioEngine`，连续听写。
//!
//! 已知系统行为：on-device 识别在用户停顿后会「重置」`formattedString`（丢弃前文），
//! 同一识别任务继续工作，只是把停顿前的内容当作一段「最终化」。若直接用
//! `formattedString` 显示，就会出现「停顿后前句被删」。
//!
//! 正确做法（与 Apple 示例/开发者论坛实践一致）：单任务内自行累计——
//! - 当某次结果带有 `speechRecognitionMetadata`（该段已最终化）时，把它并入累计；
//! - 兜底：若新片段相对上一片段明显变短/换头，判定为发生了重置，把上一片段并入累计；
//! - 展示文本 = 已累计(committed) + 当前实时片段。
//!
//! 线程模型：识别任务的创建/状态变更都在主线程；麦克风 tap 在音频实时线程，
//! 仅把缓冲喂给识别请求（线程安全）；结果回调在识别队列，统一派发回主线程处理。

use std::sync::Mutex;

use block2::RcBlock;
use core::ptr::NonNull;
use objc2::rc::Retained;
use objc2::AnyThread;
use objc2_avf_audio::{AVAudioEngine, AVAudioInputNode, AVAudioPCMBuffer, AVAudioTime};
use objc2_foundation::{NSError, NSLocale, NSString};
use objc2_speech::{
    SFSpeechAudioBufferRecognitionRequest, SFSpeechRecognitionResult, SFSpeechRecognitionTask,
    SFSpeechRecognizer, SFSpeechRecognizerAuthorizationStatus,
};
use tauri::{AppHandle, Emitter};

/// 事件名：识别出的文本（committed + 当前片段，前端在「起始文本」后覆盖写入）。
const EVENT_TEXT: &str = "speech-text";
/// 事件名：识别已开始（前端据此点亮录音指示）。
const EVENT_STARTED: &str = "speech-started";
/// 事件名：识别已停止/结束。
const EVENT_STOPPED: &str = "speech-stopped";
/// 事件名：出错（前端据此提示并复位录音状态）。
const EVENT_ERROR: &str = "speech-error";

/// 一次语音会话：引擎、输入节点、识别请求/任务整段复用；累计文本在此维护。
struct SpeechSession {
    engine: Retained<AVAudioEngine>,
    input: Retained<AVAudioInputNode>,
    #[allow(dead_code)]
    request: Retained<SFSpeechAudioBufferRecognitionRequest>,
    #[allow(dead_code)]
    task: Retained<SFSpeechRecognitionTask>,
    /// 已最终化的累计文本。
    committed: String,
    /// 当前段落的实时片段（尚未最终化）。
    last_partial: String,
    #[allow(dead_code)]
    result_block: RcBlock<dyn Fn(*mut SFSpeechRecognitionResult, *mut NSError)>,
    #[allow(dead_code)]
    tap_block: RcBlock<dyn Fn(NonNull<AVAudioPCMBuffer>, NonNull<AVAudioTime>)>,
}

// 仅在主线程访问（start/stop/回调处理均派发到主线程），故断言可跨线程持有。
unsafe impl Send for SpeechSession {}

static SESSION: Mutex<Option<SpeechSession>> = Mutex::new(None);

/// 开始语音输入：请求授权，授权通过后在主线程启动采集与识别。
pub fn start(app: AppHandle) {
    log("start() called");
    let app_auth = app.clone();
    let handler = RcBlock::new(move |status: SFSpeechRecognizerAuthorizationStatus| {
        log(&format!("auth status={}", status.0));
        if status == SFSpeechRecognizerAuthorizationStatus::Authorized {
            let app_main = app_auth.clone();
            let _ = app_auth
                .clone()
                .run_on_main_thread(move || setup_and_start(app_main.clone()));
        } else {
            let _ = app_auth.emit(EVENT_ERROR, "未获得语音识别授权");
        }
    });
    unsafe { SFSpeechRecognizer::requestAuthorization(&handler) };
}

/// 在主线程上搭建引擎、安装 tap、启动采集，并开始识别任务。
fn setup_and_start(app: AppHandle) {
    // 已在进行中：先停掉旧会话，避免重复装 tap。
    stop_session_locked();

    let lang = crate::config::AppConfig::load().general.speech_language;
    log(&format!("setup_and_start lang={lang}"));

    unsafe {
        let recognizer = make_recognizer(&lang);
        let available = recognizer.isAvailable();
        let on_device = recognizer.supportsOnDeviceRecognition();
        log(&format!("recognizer available={available} on_device={on_device}"));
        if !available {
            let _ = app.emit(EVENT_ERROR, "当前语言的语音识别不可用");
            return;
        }

        let engine = AVAudioEngine::new();
        let input = engine.inputNode();
        let format = input.outputFormatForBus(0);

        let request = SFSpeechAudioBufferRecognitionRequest::new();
        request.setShouldReportPartialResults(true);
        if recognizer.supportsOnDeviceRecognition() {
            request.setRequiresOnDeviceRecognition(true);
        }

        // 结果回调（运行在识别队列）：直接在本线程累计并 emit（与可工作版本一致）；
        // 仅「停止引擎」这类需主线程的收尾动作派发回主线程。
        let app_cb = app.clone();
        let result_block = RcBlock::new(
            move |result: *mut SFSpeechRecognitionResult, error: *mut NSError| {
                if let Some(result) = result.as_ref() {
                    let text = result.bestTranscription().formattedString().to_string();
                    let has_meta = result.speechRecognitionMetadata().is_some();
                    let is_final = result.isFinal();
                    log(&format!(
                        "result text={:?} meta={} final={}",
                        text, has_meta, is_final
                    ));
                    handle_result(&app_cb, text, has_meta, is_final);
                }
                if let Some(error) = error.as_ref() {
                    let domain = error.domain().to_string();
                    let code = error.code();
                    log(&format!("error domain={} code={}", domain, code));
                    if !is_benign_error(&domain, code) {
                        let desc = error.localizedDescription().to_string();
                        let _ = app_cb.emit(
                            EVENT_ERROR,
                            format!("语音识别出错: {desc}（{domain} {code}）"),
                        );
                    }
                    let app2 = app_cb.clone();
                    let _ = app_cb.run_on_main_thread(move || stop_session(&app2));
                }
            },
        );

        // 关键顺序：先创建识别任务（开始监听），再装 tap、启动引擎喂音频。
        // 若反过来（先启动引擎后建任务），任务建立前的音频会丢失，导致「无语音」(1110)。
        let task = recognizer.recognitionTaskWithRequest_resultHandler(&request, &result_block);

        // 麦克风 tap：实时线程回调，仅把缓冲喂给识别请求。
        let request_tap = request.clone();
        let tap_block = RcBlock::new(
            move |buffer: NonNull<AVAudioPCMBuffer>, _when: NonNull<AVAudioTime>| {
                request_tap.appendAudioPCMBuffer(buffer.as_ref());
            },
        );
        let tap_ptr: objc2_avf_audio::AVAudioNodeTapBlock =
            (&*tap_block as *const block2::DynBlock<_>).cast_mut();
        input.installTapOnBus_bufferSize_format_block(0, 4096, Some(&format), tap_ptr);

        engine.prepare();
        if let Err(e) = engine.startAndReturnError() {
            log(&format!("engine start error: {}", e.localizedDescription()));
            input.removeTapOnBus(0);
            let _ = app.emit(EVENT_ERROR, "无法启动麦克风采集");
            return;
        }
        log("engine started, task listening");

        *SESSION.lock().unwrap() = Some(SpeechSession {
            engine,
            input,
            request,
            task,
            committed: String::new(),
            last_partial: String::new(),
            result_block,
            tap_block,
        });
        let _ = app.emit(EVENT_STARTED, ());
    }
}

/// 处理一次识别结果（识别队列线程）。单任务内累计「已最终化」的段落，避免停顿丢句。
/// 直接 emit（与可工作版本一致）；is_final 时把收尾停止派发到主线程。
fn handle_result(app: &AppHandle, text: String, has_meta: bool, is_final: bool) {
    let mut guard = SESSION.lock().unwrap();
    let Some(session) = guard.as_mut() else {
        return; // 已停止。
    };

    if is_final {
        let display = join(&session.committed, &text);
        let _ = app.emit(EVENT_TEXT, display);
        drop(guard);
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || stop_session(&app2));
        return;
    }

    if has_meta {
        // 该段已最终化：并入累计，开启新段。
        if !text.is_empty() {
            session.committed = join(&session.committed, &text);
        }
        session.last_partial.clear();
        let _ = app.emit(EVENT_TEXT, session.committed.clone());
    } else {
        // 兜底：检测到重置（新片段明显变短/换头）则把上一片段并入累计。
        if !session.last_partial.is_empty() && is_reset(&session.last_partial, &text) {
            let prev = std::mem::take(&mut session.last_partial);
            session.committed = join(&session.committed, &prev);
        }
        session.last_partial = text.clone();
        let _ = app.emit(EVENT_TEXT, join(&session.committed, &text));
    }
}

/// 临时诊断日志：追加到 /tmp/askhuman_speech.log，便于定位语音链路问题。
fn log(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/askhuman_speech.log")
    {
        let _ = writeln!(f, "{msg}");
    }
}

/// 拼接累计文本与片段（任一为空取另一个，避免多余空格）。
fn join(committed: &str, partial: &str) -> String {
    if committed.is_empty() {
        partial.to_string()
    } else if partial.is_empty() {
        committed.to_string()
    } else {
        format!("{committed} {partial}")
    }
}

/// 兜底的「重置」判定：新片段不再以上一片段为前缀，且明显变短或换头，
/// 视为识别器在停顿后丢弃了前文（应把上一片段并入累计）。按字符数判断，兼容中英文。
fn is_reset(prev: &str, new: &str) -> bool {
    if new.starts_with(prev) {
        return false; // 正常增长。
    }
    let prev_len = prev.chars().count();
    let new_len = new.chars().count();
    if new_len * 2 < prev_len {
        return true; // 长度骤降，几乎可断定是重置。
    }
    if new_len < prev_len {
        // 变短且首字符不同：判为重置；否则视作同段内的修订。
        return prev.chars().next() != new.chars().next();
    }
    false
}

/// 按配置语言创建识别器：
/// - "auto"/空：取系统首选语言（`NSLocale.preferredLanguages` 第一项）；
/// - 否则用给定的 BCP-47 标识（如 "zh-CN"）。
/// 选定语言机型不支持（`initWithLocale` 返回 nil）时回退到系统默认 `new()`。
unsafe fn make_recognizer(lang: &str) -> Retained<SFSpeechRecognizer> {
    let ident: Option<Retained<NSString>> = if lang.is_empty() || lang == "auto" {
        NSLocale::preferredLanguages().firstObject()
    } else {
        Some(NSString::from_str(lang))
    };
    if let Some(ident) = ident {
        let locale = NSLocale::localeWithLocaleIdentifier(&ident);
        if let Some(recognizer) =
            SFSpeechRecognizer::initWithLocale(SFSpeechRecognizer::alloc(), &locale)
        {
            return recognizer;
        }
    }
    SFSpeechRecognizer::new()
}

/// 判断是否为「良性」结束错误：停止收音时常见的无语音/取消，不应弹错。
/// 301「请求被取消」(kLSRErrorDomain)、1110「未检测到语音」(kAFAssistantErrorDomain)、216 取消等。
fn is_benign_error(domain: &str, code: isize) -> bool {
    matches!(domain, "kAFAssistantErrorDomain" | "kLSRErrorDomain")
        && matches!(code, 216 | 301 | 1110)
}

/// 停止语音输入（派发到主线程关闭引擎与识别任务）。
pub fn stop(app: AppHandle) {
    let _ = app.clone().run_on_main_thread(move || stop_session(&app));
}

/// 关闭并丢弃当前会话并通知前端（须在主线程调用）。无会话则只发停止事件。
fn stop_session(app: &AppHandle) {
    stop_session_locked();
    let _ = app.emit(EVENT_STOPPED, ());
}

/// 关闭当前会话（须在主线程调用）。无会话则为空操作。
/// 用 `endAudio()` 让识别任务自然收尾，不调用 `cancel()`（取消会丢结果并报错）。
fn stop_session_locked() {
    let taken = SESSION.lock().unwrap().take();
    if let Some(session) = taken {
        unsafe {
            session.engine.stop();
            session.input.removeTapOnBus(0);
            session.request.endAudio();
        }
    }
}
