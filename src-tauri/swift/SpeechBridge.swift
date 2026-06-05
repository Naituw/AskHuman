import Foundation
import AVFoundation
import Speech

// Objective-C 桥：把仅 Swift 可用的 SpeechEngine 暴露给 Rust(objc2)。
// 回调用 @convention(block) 闭包属性（Rust 侧用 block2 RcBlock 设置，闭包内 app.emit）。
// 暴露面只用 ObjC 兼容类型（NSString/Float/Bool/block）；async/SpeechAnalyzer 全在 Swift 内。
@objc(AHSpeechBridge)
final class AHSpeechBridge: NSObject {
    // M1 自检用，保留以便 Rust 单测回归。
    @objc static func ping() -> NSString { "pong" as NSString }

    // 运行期是否可用（macOS 26+）。前端据此显隐麦克风按钮。
    @objc static func isAvailable() -> Bool {
        if #available(macOS 26, *) { return true }
        return false
    }

    private var onCommittedBlock: (@convention(block) (NSString) -> Void)?
    private var onVolatileBlock: (@convention(block) (NSString) -> Void)?
    private var onLevelBlock: (@convention(block) (Float) -> Void)?
    private var onStatusBlock: (@convention(block) (NSString) -> Void)?
    private var onErrorBlock: (@convention(block) (NSString) -> Void)?
    private var onStoppedBlock: (@convention(block) () -> Void)?
    private var onReadyBlock: (@convention(block) () -> Void)?

    // 持有 SpeechEngine（其类型仅 26+ 可用，故用 Any? 擦除，调用处经 if #available 还原）。
    private var engineBox: Any?

    @objc func setOnCommitted(_ b: @escaping @convention(block) (NSString) -> Void) { onCommittedBlock = b }
    @objc func setOnVolatile(_ b: @escaping @convention(block) (NSString) -> Void) { onVolatileBlock = b }
    @objc func setOnLevel(_ b: @escaping @convention(block) (Float) -> Void) { onLevelBlock = b }
    @objc func setOnStatus(_ b: @escaping @convention(block) (NSString) -> Void) { onStatusBlock = b }
    @objc func setOnError(_ b: @escaping @convention(block) (NSString) -> Void) { onErrorBlock = b }
    @objc func setOnStopped(_ b: @escaping @convention(block) () -> Void) { onStoppedBlock = b }
    @objc func setOnReady(_ b: @escaping @convention(block) () -> Void) { onReadyBlock = b }

    // localeID：BCP-47（如 zh-CN）；空串=跟随系统首选语言。
    @objc func start(_ localeID: NSString) {
        guard #available(macOS 26, *) else {
            onErrorBlock?("语音输入需要 macOS 26 及以上" as NSString)
            return
        }
        let id = localeID as String
        let locale: Locale
        if id.isEmpty {
            locale = Locale(identifier: Locale.preferredLanguages.first ?? Locale.current.identifier)
        } else {
            locale = Locale(identifier: id)
        }

        let engine = SpeechEngine(locale: locale)
        engine.onCommitted = { [weak self] s in self?.onCommittedBlock?(s as NSString) }
        engine.onVolatile = { [weak self] s in self?.onVolatileBlock?(s as NSString) }
        engine.onLevel = { [weak self] v in self?.onLevelBlock?(v) }
        engine.onStatus = { [weak self] s in self?.onStatusBlock?(s as NSString) }
        engine.onError = { [weak self] s in self?.onErrorBlock?(s as NSString) }
        engine.onReady = { [weak self] in self?.onReadyBlock?() }
        engineBox = engine

        Task { [weak self] in
            let (speech, mic) = await AHSpeechBridge.requestAuth()
            guard speech, mic else {
                self?.onErrorBlock?("麦克风或语音识别未获授权" as NSString)
                return
            }
            await engine.start()
        }
    }

    @objc func stop() {
        if #available(macOS 26, *), let e = engineBox as? SpeechEngine {
            Task { [weak self] in
                await e.stop()
                self?.onStoppedBlock?()
                self?.engineBox = nil
            }
        } else {
            onStoppedBlock?()
        }
    }

    @objc func flush() {
        if #available(macOS 26, *), let e = engineBox as? SpeechEngine {
            Task { await e.flush() }
        }
    }

    // 麦克风 + 语音识别授权（均为旧 API，无需版本门）。
    static func requestAuth() async -> (Bool, Bool) {
        let speech = await withCheckedContinuation { c in
            SFSpeechRecognizer.requestAuthorization { c.resume(returning: $0 == .authorized) }
        }
        let mic = await withCheckedContinuation { c in
            AVCaptureDevice.requestAccess(for: .audio) { c.resume(returning: $0) }
        }
        return (speech, mic)
    }
}
