import SwiftUI
import AppKit
import SpeechKit

func glog(_ s: String) {
    let line = "[GUI \(Date())] \(s)\n"
    let url = URL(fileURLWithPath: "/tmp/speechdemo.log")
    if let data = line.data(using: .utf8) {
        if let h = try? FileHandle(forWritingTo: url) {
            h.seekToEndOfFile(); h.write(data); try? h.close()
        } else {
            try? data.write(to: url)
        }
    }
}

// SwiftUI GUI Demo：可选新/旧 API、选语言、看电平条，
// 最终结果会插入到文本框「光标处」，便于验证插入效果。⌘D 开始/停止。

@MainActor
final class EditorCoordinator: NSObject, NSTextViewDelegate {
    weak var textView: NSTextView?
    // 用户在听写中途移动光标时回调（用于通知引擎 flush 重启识别）。
    var onUserMovedCaret: (() -> Void)?

    // 当前「实时片段」固定锚点与长度（就地替换刷新）。
    private var interimStart = 0
    private var interimLen = 0
    private var expectedCaret = -1   // 过滤我们自己造成的选区变化
    private var writing = false
    private var sessionActive = false

    func textViewDidChangeSelection(_ notification: Notification) {
        guard sessionActive, !writing, let tv = textView else { return }
        let loc = tv.selectedRange().location
        if loc == expectedCaret { return } // 我们程序性移动的光标，忽略
        // 用户主动移动光标：固定当前实时片段，新锚点=新光标，并通知引擎重启。
        interimStart = loc
        interimLen = 0
        expectedCaret = loc
        glog("用户移动光标→固定+flush, 新锚点=\(loc)")
        onUserMovedCaret?()
    }

    func beginSession() {
        guard let tv = textView else { return }
        let len = (tv.string as NSString).length
        let caret = (tv.window?.firstResponder === tv)
            ? tv.selectedRange().location : len
        interimStart = min(max(caret, 0), len)
        interimLen = 0
        expectedCaret = interimStart
        sessionActive = true
        glog("beginSession interimStart=\(interimStart) textLen=\(len)")
    }

    func endSession() { sessionActive = false }

    // 锚点固定：就地替换 [interimStart, interimLen]。permanent=true 提交为永久文本。
    private func write(_ text: String, permanent: Bool) {
        guard let tv = textView else { return }
        writing = true
        defer { writing = false }
        let full = tv.string as NSString
        var r = NSRange(location: interimStart, length: interimLen)
        if r.location + r.length > full.length {
            r = NSRange(location: min(interimStart, full.length), length: 0)
        }
        guard tv.shouldChangeText(in: r, replacementString: text) else { return }
        tv.replaceCharacters(in: r, with: text)
        tv.didChangeText()
        let textLen = (text as NSString).length
        if permanent {
            interimStart = r.location + textLen
            interimLen = 0
        } else {
            interimStart = r.location
            interimLen = textLen
        }
        let caret = r.location + textLen
        expectedCaret = caret
        tv.setSelectedRange(NSRange(location: caret, length: 0))
        tv.scrollRangeToVisible(tv.selectedRange())
    }

    func setVolatile(_ s: String) { write(s, permanent: false) }
    func commit(_ s: String) { if !s.isEmpty { write(s, permanent: true) } }
}

struct CaretTextView: NSViewRepresentable {
    let coordinator: EditorCoordinator

    func makeCoordinator() -> EditorCoordinator { coordinator }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSTextView.scrollableTextView()
        let tv = scroll.documentView as! NSTextView
        tv.delegate = context.coordinator
        tv.font = .systemFont(ofSize: 15)
        tv.isRichText = false
        tv.isAutomaticQuoteSubstitutionEnabled = false
        tv.allowsUndo = true
        tv.string = ""
        context.coordinator.textView = tv
        return scroll
    }

    func updateNSView(_ nsView: NSScrollView, context: Context) {}
}

@MainActor
final class VM: ObservableObject {
    @Published var useNew = true
    @Published var localeID = "zh-CN"
    @Published var listening = false
    @Published var status = "就绪"
    @Published var level: Float = 0

    let editor = EditorCoordinator()
    private var engine: SpeechEngine?
    let locales = ["zh-CN", "en-US", "zh-TW", "ja-JP", "ko-KR"]

    func toggle() { listening ? stop() : start() }

    func start() {
        status = "准备中…"
        editor.beginSession()
        editor.onUserMovedCaret = { [weak self] in
            guard let self, let e = self.engine, self.listening else { return }
            Task { await e.flush() }
        }
        let e = SpeechEngine(kind: useNew ? .new : .old, locale: Locale(identifier: localeID))
        e.onStatus = { [weak self] s in Task { @MainActor in self?.status = s } }
        e.onError = { [weak self] s in Task { @MainActor in self?.status = "错误: \(s)" } }
        e.onLevel = { [weak self] p in Task { @MainActor in self?.level = p } }
        e.onCommitted = { [weak self] t in Task { @MainActor in self?.editor.commit(t) } }
        e.onVolatile = { [weak self] t in Task { @MainActor in self?.editor.setVolatile(t) } }
        engine = e
        listening = true
        Task {
            let auth = await SpeechEngine.requestAuth()
            guard auth.speech, auth.mic else {
                status = "未授权 语音=\(auth.speech) 麦克风=\(auth.mic)"
                listening = false
                return
            }
            do { try await e.start() }
            catch {
                status = "启动失败: \(error.localizedDescription)"
                listening = false
            }
        }
    }

    func stop() {
        listening = false
        level = 0
        editor.endSession()
        let e = engine
        engine = nil
        Task { await e?.stop() }
        status = "已停止"
    }
}

struct ContentView: View {
    @StateObject private var vm = VM()

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Picker("", selection: $vm.useNew) {
                    Text("新 SpeechAnalyzer").tag(true)
                    Text("旧 SFSpeechRecognizer").tag(false)
                }
                .pickerStyle(.segmented)
                .frame(width: 320)
                .disabled(vm.listening)

                Picker("语言", selection: $vm.localeID) {
                    ForEach(vm.locales, id: \.self) { Text($0).tag($0) }
                }
                .frame(width: 150)
                .disabled(vm.listening)

                Spacer()

                Button(vm.listening ? "停止 (⌘D)" : "开始 (⌘D)") { vm.toggle() }
                    .keyboardShortcut("d", modifiers: .command)
                    .buttonStyle(.borderedProminent)
            }

            HStack(spacing: 8) {
                Image(systemName: vm.listening ? "mic.fill" : "mic.slash")
                    .foregroundStyle(vm.listening ? .red : .secondary)
                ProgressView(value: Double(min(vm.level * 8, 1)))
                    .frame(maxWidth: .infinity)
            }

            Text(vm.status)
                .font(.caption)
                .foregroundStyle(.secondary)

            CaretTextView(coordinator: vm.editor)
                .frame(minHeight: 200)
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(.gray.opacity(0.4)))

            Text("把光标放到文字中间再开始，识别文字会实时插入在光标处、边说边刷新。")
                .font(.caption2)
                .foregroundStyle(.secondary)
        }
        .padding(16)
        .frame(minWidth: 600, minHeight: 460)
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.regular)

let window = NSWindow(
    contentRect: NSRect(x: 0, y: 0, width: 640, height: 480),
    styleMask: [.titled, .closable, .miniaturizable, .resizable],
    backing: .buffered,
    defer: false
)
window.title = "Speech Demo (新/旧 API)"
window.center()
window.contentView = NSHostingView(rootView: ContentView())
window.makeKeyAndOrderFront(nil)

app.activate(ignoringOtherApps: true)
app.run()
