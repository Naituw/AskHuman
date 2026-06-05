import Foundation
import AVFoundation
import Speech

// macOS 26 新语音识别（SpeechAnalyzer / SpeechTranscriber），离线、实时增量。
// 仅在 macOS 26+ 可用：整类标注 @available，调用方需经 `if #available` 进入。
//
// 回调语义（供桥转发为前端事件）：
// - onCommitted: 新「已最终化」的文本片段，应永久插入到当前光标处。
// - onVolatile : 当前未最终化的实时片段，应就地替换显示（可随光标移动而重置）。
// - onLevel    : 输入电平峰值(0~1)，用于录音动效。
// - onStatus   : 人类可读状态（识别语言/下载/聆听中…）。
// - onError    : 错误文案（不支持/未授权/下载失败等）。
//
// 关键音频处理：在输入节点开启 Voice Processing 拿到响亮的 AGC 信号，
// 取第 0 声道做成单声道，再转换到分析器要求的格式，避免 MacBook 多麦阵列原始信号过弱。
@available(macOS 26, *)
final class SpeechEngine: @unchecked Sendable {
    var onCommitted: ((String) -> Void)?
    var onVolatile: ((String) -> Void)?
    var onLevel: ((Float) -> Void)?
    var onStatus: ((String) -> Void)?
    var onError: ((String) -> Void)?
    // 识别管线已就绪、真正进入实时录制（前端据此从 loading 切到高亮）。
    var onReady: (() -> Void)?

    private let locale: Locale
    private let audioEngine = AVAudioEngine()

    private var analyzer: SpeechAnalyzer?
    private var transcriber: SpeechTranscriber?
    private var inputBuilder: AsyncStream<AnalyzerInput>.Continuation?
    private var analyzerFormat: AVAudioFormat?
    private var converter: AVAudioConverter?
    private var resultsTask: Task<Void, Never>?
    private var resolvedLocale: Locale?
    // 会话代数：flush/stop 后递增，旧会话的回调据此被忽略，避免错位/重复。
    private var sessionGen = 0

    // 启动期音频缓冲：点下即采集，管线就绪后补喂，避免前几个字丢失。
    private var pending: [AVAudioPCMBuffer] = []
    private let pendingLock = NSLock()
    private var live = false
    // 缓冲上限（约 12 秒）：正常启动只需 1 秒内；超出丢最旧。
    private static let pendingCap = 150

    init(locale: Locale) {
        self.locale = locale
    }

    // MARK: - 生命周期

    func start() async {
        do { try await startNew() }
        catch { onError?(Self.message(error)) }
    }

    func stop() async {
        sessionGen += 1
        audioEngine.inputNode.removeTap(onBus: 0)
        audioEngine.stop()
        inputBuilder?.finish()
        inputBuilder = nil
        try? await analyzer?.finalizeAndFinishThroughEndOfInput()
        resultsTask?.cancel()
    }

    // 固定已生成文本并重启识别会话（用户听写中途移动光标时调用）；不停音频引擎与 tap。
    func flush() async {
        let oldBuilder = inputBuilder
        inputBuilder = nil
        oldBuilder?.finish()
        resultsTask?.cancel()
        let old = analyzer
        Task { try? await old?.finalizeAndFinishThroughEndOfInput() }
        do { try await buildNewSession() }
        catch { onError?(Self.message(error)) }
    }

    // MARK: - 启动新会话

    private func startNew() async throws {
        // 1) 点下立刻开始采集并缓冲（管线就绪前的音频先存起来），避免前几个字丢失。
        let monoFormat = try installInputTap { [weak self] mono in
            guard let self else { return }
            if self.live {
                if let conv = self.converter, let f = self.analyzerFormat, let b = self.inputBuilder,
                   let out = self.convert(mono, to: f, using: conv) {
                    b.yield(AnalyzerInput(buffer: out))
                }
            } else {
                self.pendingLock.lock()
                self.pending.append(mono)
                if self.pending.count > Self.pendingCap {
                    self.pending.removeFirst(self.pending.count - Self.pendingCap)
                }
                self.pendingLock.unlock()
            }
        }
        onStatus?("准备中…")

        // 2) 初始化识别管线。
        guard let supported = await SpeechTranscriber.supportedLocale(equivalentTo: locale) else {
            throw Self.err("当前机型不支持所选语言（\(locale.identifier)）的语音识别")
        }
        let probe = SpeechTranscriber(locale: supported, preset: .progressiveTranscription)
        if let req = try await AssetInventory.assetInstallationRequest(supporting: [probe]) {
            onStatus?("正在下载语言模型…")
            try await req.downloadAndInstall()
            onStatus?("模型就绪")
        }
        resolvedLocale = supported
        guard let fmt = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [probe]) else {
            throw Self.err("无法获取兼容的音频格式")
        }
        analyzerFormat = fmt
        converter = AVAudioConverter(from: monoFormat, to: fmt)

        try await buildNewSession()

        // 3) 倒出启动期缓冲，转入实时。
        pendingLock.lock()
        if let conv = converter, let f = analyzerFormat, let b = inputBuilder {
            for mono in pending {
                if let out = convert(mono, to: f, using: conv) {
                    b.yield(AnalyzerInput(buffer: out))
                }
            }
        }
        pending.removeAll()
        live = true
        pendingLock.unlock()
        onStatus?("聆听中…")
        onReady?()
    }

    // 创建全新识别会话（新 transcriber/analyzer/stream/results），不动音频引擎与 tap。
    private func buildNewSession() async throws {
        guard let supported = resolvedLocale else { throw Self.err("识别语言未解析") }
        sessionGen += 1
        let gen = sessionGen

        let t = SpeechTranscriber(locale: supported, preset: .progressiveTranscription)
        transcriber = t
        let a = SpeechAnalyzer(modules: [t])
        analyzer = a

        let (stream, builder) = AsyncStream<AnalyzerInput>.makeStream()
        inputBuilder = builder

        resultsTask = Task { [weak self] in
            guard let self else { return }
            do {
                for try await r in t.results {
                    if gen != self.sessionGen { break } // 旧会话回调，忽略
                    let piece = String(r.text.characters)
                    if r.isFinal {
                        self.onCommitted?(piece)
                        self.onVolatile?("")
                    } else {
                        self.onVolatile?(piece)
                    }
                }
            } catch {
                // 结束/取消属正常收尾，不上报为错误。
            }
        }

        try await a.start(inputSequence: stream)
    }

    // MARK: - 音频采集

    private func installInputTap(onMono: @escaping (AVAudioPCMBuffer) -> Void) throws -> AVAudioFormat {
        let input = audioEngine.inputNode
        // 不开 Voice Processing：其初始化约耗 0.7s，是启动延迟主因。
        // 改为裸采集（近讲信号已够 SpeechAnalyzer 识别），换取 <100ms 的即时启动。

        let inFormat = input.outputFormat(forBus: 0)
        guard let monoFormat = AVAudioFormat(commonFormat: .pcmFormatFloat32,
                                             sampleRate: inFormat.sampleRate,
                                             channels: 1, interleaved: false) else {
            throw Self.err("无法创建单声道音频格式")
        }

        input.installTap(onBus: 0, bufferSize: 4096, format: inFormat) { [weak self] buffer, _ in
            guard let self else { return }
            let mono = self.extractMono(buffer, monoFormat: monoFormat) ?? buffer
            self.reportLevel(mono)
            onMono(mono)
        }
        audioEngine.prepare()
        try audioEngine.start()
        return monoFormat
    }

    // 取第 0 声道（VP 通常把处理后的人声放在 ch0）做成单声道缓冲。
    private func extractMono(_ buffer: AVAudioPCMBuffer, monoFormat: AVAudioFormat) -> AVAudioPCMBuffer? {
        guard let inData = buffer.floatChannelData, buffer.frameLength > 0 else { return nil }
        let frames = Int(buffer.frameLength)
        guard let out = AVAudioPCMBuffer(pcmFormat: monoFormat, frameCapacity: buffer.frameLength) else { return nil }
        out.frameLength = buffer.frameLength
        let dst = out.floatChannelData![0]
        let src = inData[0]
        for i in 0..<frames { dst[i] = src[i] }
        return out
    }

    private func reportLevel(_ buffer: AVAudioPCMBuffer) {
        guard let ch = buffer.floatChannelData, buffer.frameLength > 0 else { return }
        let n = Int(buffer.frameLength)
        var peak: Float = 0
        for i in 0..<n { peak = max(peak, abs(ch[0][i])) }
        onLevel?(peak)
    }

    private func convert(_ buffer: AVAudioPCMBuffer, to format: AVAudioFormat, using converter: AVAudioConverter) -> AVAudioPCMBuffer? {
        let ratio = format.sampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount(Double(buffer.frameLength) * ratio) + 1024
        guard let out = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: capacity) else { return nil }
        var error: NSError?
        var consumed = false
        let status = converter.convert(to: out, error: &error) { _, inStatus in
            if consumed { inStatus.pointee = .noDataNow; return nil }
            consumed = true
            inStatus.pointee = .haveData
            return buffer
        }
        if error != nil || status == .error { return nil }
        return out
    }

    // MARK: - 工具

    private static func err(_ m: String) -> NSError {
        NSError(domain: "AHSpeech", code: 1, userInfo: [NSLocalizedDescriptionKey: m])
    }

    private static func message(_ e: Error) -> String {
        let ns = e as NSError
        if let d = ns.userInfo[NSLocalizedDescriptionKey] as? String { return d }
        return ns.localizedDescription
    }
}
