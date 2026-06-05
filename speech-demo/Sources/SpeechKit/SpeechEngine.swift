import Foundation
import AVFoundation
import Speech

public enum SpeechAPIKind: Sendable { case new, old }

// 统一封装新(SpeechAnalyzer)与旧(SFSpeechRecognizer)两套识别，供 CLI 与 GUI 共用。
// 关键修复：在输入节点开启 Voice Processing，拿到干净的单声道/AGC 信号，
// 否则 MacBook 三麦阵列读到的原始信号电平极低，识别不到。
public final class SpeechEngine: @unchecked Sendable {
    // 增量提交：新「最终化」的文本片段(永久插入光标处)。
    public var onCommitted: ((String) -> Void)?
    // 实时片段：当前未最终化的转写(就地替换，可随光标移动)。
    public var onVolatile: ((String) -> Void)?
    public var onLevel: ((Float) -> Void)?
    public var onStatus: ((String) -> Void)?
    public var onError: ((String) -> Void)?

    private let kind: SpeechAPIKind
    private let locale: Locale
    private let audioEngine = AVAudioEngine()

    // 新 API
    private var analyzer: SpeechAnalyzer?
    private var transcriber: SpeechTranscriber?
    private var inputBuilder: AsyncStream<AnalyzerInput>.Continuation?
    private var analyzerFormat: AVAudioFormat?
    private var converter: AVAudioConverter?
    private var resultsTask: Task<Void, Never>?
    private var resolvedLocale: Locale?
    // 会话代数：flush 后递增，旧会话的回调据此被忽略，避免错位/重复。
    private var sessionGen = 0

    // 旧 API
    private var recognizer: SFSpeechRecognizer?
    private var request: SFSpeechAudioBufferRecognitionRequest?
    private var task: SFSpeechRecognitionTask?

    // 诊断计数
    private var fedCount = 0
    private var resultCount = 0
    private static let logURL = URL(fileURLWithPath: "/tmp/speechdemo.log")

    public init(kind: SpeechAPIKind, locale: Locale) {
        self.kind = kind
        self.locale = locale
        log("==== 新会话 kind=\(kind) locale=\(locale.identifier) ====")
    }

    private func log(_ s: String) {
        let line = "[\(Date())] \(s)\n"
        if let data = line.data(using: .utf8) {
            if let h = try? FileHandle(forWritingTo: SpeechEngine.logURL) {
                h.seekToEndOfFile(); h.write(data); try? h.close()
            } else {
                try? data.write(to: SpeechEngine.logURL)
            }
        }
    }

    public static func requestAuth() async -> (speech: Bool, mic: Bool) {
        let s = await withCheckedContinuation { c in
            SFSpeechRecognizer.requestAuthorization { c.resume(returning: $0 == .authorized) }
        }
        let m = await withCheckedContinuation { c in
            AVCaptureDevice.requestAccess(for: .audio) { c.resume(returning: $0) }
        }
        return (s, m)
    }

    public static func defaultInputName() -> String {
        AVCaptureDevice.default(for: .audio)?.localizedName ?? "未知"
    }

    public func start() async throws {
        switch kind {
        case .new: try await startNew()
        case .old: try startOld()
        }
    }

    public func stop() async {
        sessionGen += 1 // 让后续回调失效
        audioEngine.inputNode.removeTap(onBus: 0)
        audioEngine.stop()
        switch kind {
        case .new:
            inputBuilder?.finish()
            try? await analyzer?.finalizeAndFinishThroughEndOfInput()
            resultsTask?.cancel()
        case .old:
            request?.endAudio()
            task?.finish()
        }
    }

    // 安装输入 tap：开 VP 拿到响亮的 AGC 信号，直接 tap 输入节点(持续出数据)，
    // 取第 0 声道做成单声道再回调。返回单声道格式(原采样率)。
    private func installInputTap(onMono: @escaping (AVAudioPCMBuffer) -> Void) throws -> AVAudioFormat {
        let input = audioEngine.inputNode
        let disableVP = ProcessInfo.processInfo.environment["SPEECH_NO_VP"] == "1"
        if disableVP {
            log("VP 被禁用(env)")
        } else {
            do {
                try input.setVoiceProcessingEnabled(true)
                log("VP 已开启")
            } catch {
                log("VP 开启失败: \(error)")
            }
        }
        let inFormat = input.outputFormat(forBus: 0)
        log("输入格式 \(inFormat.sampleRate)Hz/\(inFormat.channelCount)ch device=\(SpeechEngine.defaultInputName())")
        onStatus?("麦克风: \(SpeechEngine.defaultInputName())  \(Int(inFormat.sampleRate))Hz/\(inFormat.channelCount)ch")

        guard let monoFormat = AVAudioFormat(commonFormat: .pcmFormatFloat32,
                                             sampleRate: inFormat.sampleRate,
                                             channels: 1, interleaved: false) else {
            throw err("无法创建单声道格式")
        }

        input.installTap(onBus: 0, bufferSize: 4096, format: inFormat) { [weak self] buffer, _ in
            guard let self else { return }
            let mono = self.extractMono(buffer, monoFormat: monoFormat) ?? buffer
            self.reportLevel(mono)
            onMono(mono)
        }
        audioEngine.prepare()
        try audioEngine.start()
        log("音频引擎已启动，单声道 \(monoFormat.sampleRate)Hz/1ch")
        return monoFormat
    }

    // 取第 0 声道(VP 通常把处理后的人声放在 ch0)，做成单声道缓冲。
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

    private var levelTick = 0
    private func reportLevel(_ buffer: AVAudioPCMBuffer) {
        guard let ch = buffer.floatChannelData, buffer.frameLength > 0 else { return }
        let n = Int(buffer.frameLength)
        var peak: Float = 0
        for i in 0..<n { peak = max(peak, abs(ch[0][i])) }
        onLevel?(peak)
        levelTick += 1
        if levelTick % 50 == 1 { log(String(format: "单声道峰值=%.4f", peak)) }
    }

    private func err(_ m: String) -> NSError {
        NSError(domain: "SpeechKit", code: 1, userInfo: [NSLocalizedDescriptionKey: m])
    }

    // MARK: - 新 API
    private func startNew() async throws {
        guard let supported = await SpeechTranscriber.supportedLocale(equivalentTo: locale) else {
            let all = await SpeechTranscriber.supportedLocales.map { $0.identifier(.bcp47) }
            throw err("新 API 不支持 \(locale.identifier)。支持: \(all.prefix(20).joined(separator: ", "))")
        }
        onStatus?("识别区域 \(supported.identifier(.bcp47))")

        let t = SpeechTranscriber(locale: supported, preset: .progressiveTranscription)
        transcriber = t

        if let req = try await AssetInventory.assetInstallationRequest(supporting: [t]) {
            onStatus?("下载语言模型…(首次)")
            try await req.downloadAndInstall()
            onStatus?("模型就绪")
        }

        resolvedLocale = supported
        guard let fmt = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [t]) else {
            throw err("拿不到兼容音频格式")
        }
        analyzerFormat = fmt
        log("分析器目标格式 \(fmt.sampleRate)Hz/\(fmt.channelCount)ch")

        try await buildNewSession()

        // 先建好 mono→分析器格式 的转换器，再装 tap，避免早期缓冲丢失。
        let monoFormat = try installInputTap { [weak self] mono in
            guard let self, let conv = self.converter, let f = self.analyzerFormat,
                  let b = self.inputBuilder else { return }
            if let out = self.convert(mono, to: f, using: conv) {
                self.fedCount += 1
                if self.fedCount % 50 == 1 { self.log("已喂入 \(self.fedCount) 个缓冲, 帧=\(out.frameLength)") }
                b.yield(AnalyzerInput(buffer: out))
            } else {
                self.log("⚠️ 转换失败")
            }
        }
        converter = AVAudioConverter(from: monoFormat, to: fmt)
        if converter == nil { log("⚠️ 无法创建转换器 \(monoFormat) -> \(fmt)") }
        onStatus?("聆听中…")
    }

    // 创建一个全新的识别会话(新 transcriber/analyzer/stream/results)，不动音频引擎与 tap。
    private func buildNewSession() async throws {
        guard let supported = resolvedLocale else { throw err("locale 未解析") }
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
            self.log("开始监听结果流 gen=\(gen)")
            do {
                for try await r in t.results {
                    if gen != self.sessionGen { break } // 旧会话回调，忽略
                    let piece = String(r.text.characters)
                    self.resultCount += 1
                    self.log("结果#\(self.resultCount) gen=\(gen) isFinal=\(r.isFinal) text=\(piece.isEmpty ? "<空>" : piece)")
                    if r.isFinal {
                        self.onCommitted?(piece)
                        self.onVolatile?("")
                    } else {
                        self.onVolatile?(piece)
                    }
                }
            } catch {
                self.log("结果流出错 gen=\(gen): \(error)")
            }
        }
        try await a.start(inputSequence: stream)
    }

    // 固定已生成文本、重启识别会话（用户在听写中途移动光标时调用）。
    public func flush() async {
        log("flush kind=\(kind)")
        switch kind {
        case .new:
            let oldBuilder = inputBuilder
            inputBuilder = nil
            oldBuilder?.finish()
            resultsTask?.cancel()
            let old = analyzer
            Task { try? await old?.finalizeAndFinishThroughEndOfInput() }
            do { try await buildNewSession() } catch { log("flush 重建失败: \(error)") }
        case .old:
            restartOldTask()
        }
    }

    private func restartOldTask() {
        guard let r = recognizer else { return }
        request?.endAudio()
        task?.finish()
        sessionGen += 1
        let gen = sessionGen
        let req = SFSpeechAudioBufferRecognitionRequest()
        req.shouldReportPartialResults = true
        if r.supportsOnDeviceRecognition { req.requiresOnDeviceRecognition = true }
        request = req
        task = r.recognitionTask(with: req) { [weak self] result, error in
            guard let self, gen == self.sessionGen else { return }
            if let result {
                let txt = result.bestTranscription.formattedString
                if result.isFinal { self.onCommitted?(txt); self.onVolatile?("") }
                else { self.onVolatile?(txt) }
            }
            if let error { self.log("旧API错误 gen=\(gen): \(error)") }
        }
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

    // MARK: - 旧 API
    private func startOld() throws {
        guard let r = SFSpeechRecognizer(locale: locale) else { throw err("无法为 \(locale.identifier) 创建识别器") }
        recognizer = r
        if r.supportsOnDeviceRecognition {
            onStatus?("旧 API: 离线识别")
        } else {
            onStatus?("旧 API: 该区域不支持离线，走在线")
        }
        restartOldTask()
        _ = try installInputTap { [weak self] mono in
            guard let self else { return }
            self.fedCount += 1
            if self.fedCount % 50 == 1 { self.log("已喂入 \(self.fedCount) 个缓冲 帧=\(mono.frameLength) fmt=\(mono.format.sampleRate)/\(mono.format.channelCount)ch") }
            self.request?.append(mono)
        }
        onStatus?("聆听中…")
    }
}
