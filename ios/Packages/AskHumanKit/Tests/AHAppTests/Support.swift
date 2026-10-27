import Foundation
import XCTest
import AHProtocol
import AHCore

/// Polls `condition` until it holds or the timeout elapses.
@MainActor
func eventually(
    timeout: Double = 3, _ message: @autoclosure () -> String = "condition not met", file: StaticString = #filePath,
    line: UInt = #line, _ condition: () -> Bool
) async {
    let deadline = Date().addingTimeInterval(timeout)
    while Date() < deadline {
        if condition() { return }
        try? await Task.sleep(for: .milliseconds(10))
    }
    XCTFail(message(), file: file, line: line)
}

func question(_ text: String, _ options: String...) -> QuestionView {
    QuestionView(text: text, options: options.map { OptionView(text: $0, recommended: false) })
}

func ask(
    _ questions: [QuestionView], selectOnly: Bool = false, single: Bool = false
) -> AskView {
    AskView(isMarkdown: true, message: MessageView(text: "message", files: []), questions: questions, selectOnly: selectOnly, single: single)
}
