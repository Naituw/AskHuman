import Foundation
import SpeechKit

// 纯 Swift CLI Demo：实时麦克风转写，对比新/旧 API。
// 用法:
//   swift run speech-demo --new --locale zh-CN   (默认 --new)
//   swift run speech-demo --old --locale zh-CN
// 说话实时显示 ⏳临时 / ✅最终，按回车结束。

func elog(_ s: String) { FileHandle.standardError.write((s + "\n").data(using: .utf8)!) }
func showText(_ t: String) { print("\r\u{1B}[K\(t)", terminator: ""); fflush(stdout) }

var useNew = true
var localeID = Locale.current.identifier(.bcp47)
do {
    let args = CommandLine.arguments
    var i = 1
    while i < args.count {
        switch args[i] {
        case "--old": useNew = false
        case "--new": useNew = true
        case "--locale": i += 1; if i < args.count { localeID = args[i] }
        case "-h", "--help":
            print("用法: speech-demo [--new|--old] [--locale <bcp47>]")
            exit(0)
        default: break
        }
        i += 1
    }
}

let auth = await SpeechEngine.requestAuth()
elog("授权 语音=\(auth.speech) 麦克风=\(auth.mic)")
guard auth.speech, auth.mic else {
    print("未获授权（系统设置→隐私→语音识别/麦克风 给终端打勾）")
    exit(1)
}

print("================================")
print("模式: \(useNew ? "新 API (SpeechAnalyzer)" : "旧 API (SFSpeechRecognizer)")  区域: \(localeID)")
print("开始说话…（按回车结束）")
print("================================")

let engine = SpeechEngine(kind: useNew ? .new : .old, locale: Locale(identifier: localeID))
var committed = ""
var volatileText = ""
func refresh() { showText(committed + volatileText) }
engine.onStatus = { elog("· \($0)") }
engine.onError = { elog("错误: \($0)") }
engine.onCommitted = { committed += $0; volatileText = ""; refresh() }
engine.onVolatile = { volatileText = $0; refresh() }
var peakSeen: Float = 0
engine.onLevel = { peakSeen = max(peakSeen, $0) }

do {
    try await engine.start()
} catch {
    print("启动失败: \(error.localizedDescription)")
    exit(1)
}

_ = readLine()
await engine.stop()
elog(String(format: "本次最大音量峰值=%.4f", peakSeen))
print("\n已结束。")
