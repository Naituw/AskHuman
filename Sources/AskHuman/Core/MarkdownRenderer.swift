import Foundation

enum MarkdownRenderer {
    /// 把消息渲染为 AttributedString 供 SwiftUI Text 使用
    static func render(_ text: String, isMarkdown: Bool) -> AttributedString {
        guard isMarkdown else {
            return AttributedString(text)
        }
        let options = AttributedString.MarkdownParsingOptions(
            allowsExtendedAttributes: true,
            interpretedSyntax: .full,
            failurePolicy: .returnPartiallyParsedIfPossible
        )
        if let attr = try? AttributedString(markdown: text, options: options) {
            return attr
        }
        return AttributedString(text)
    }
}
