import Foundation

/// 把标准 Markdown 处理为 Telegram MarkdownV2（移植自原版逻辑）
enum TelegramMarkdown {
    static func process(_ text: String) -> String {
        var protected: [String] = []
        var result = text

        result = protectCodeBlocks(result, into: &protected)
        result = protectInlineCode(result, into: &protected)
        result = convertMarkdown(result)
        result = escapeSpecialChars(result)

        for (i, segment) in protected.enumerated() {
            result = result.replacingOccurrences(of: "CODEBLOCK\(i)", with: segment)
        }
        return result
    }

    private static func protectCodeBlocks(_ text: String, into protected: inout [String]) -> String {
        var text = text
        while let start = text.range(of: "```") {
            let afterStart = text[start.upperBound...]
            guard let end = afterStart.range(of: "```") else { break }
            let blockRange = start.lowerBound..<end.upperBound
            let block = String(text[blockRange])
            let placeholder = "CODEBLOCK\(protected.count)"
            protected.append(block)
            text.replaceSubrange(blockRange, with: placeholder)
        }
        return text
    }

    private static func protectInlineCode(_ text: String, into protected: inout [String]) -> String {
        var text = text
        var searchStart = text.startIndex
        while let open = text.range(of: "`", range: searchStart..<text.endIndex) {
            let afterOpen = open.upperBound
            guard let close = text.range(of: "`", range: afterOpen..<text.endIndex) else { break }
            let codeRange = open.lowerBound..<close.upperBound
            let code = String(text[codeRange])
            let placeholder = "CODEBLOCK\(protected.count)"
            protected.append(code)
            text.replaceSubrange(codeRange, with: placeholder)
            searchStart = text.range(of: placeholder)?.upperBound ?? text.endIndex
        }
        return text
    }

    private static func convertMarkdown(_ text: String) -> String {
        var lines = text.components(separatedBy: "\n")

        // 标题 # .. -> >标题
        let headerRegex = try? NSRegularExpression(pattern: "^(#{1,6})\\s+(.+)$")
        lines = lines.map { line in
            guard let re = headerRegex else { return line }
            let range = NSRange(line.startIndex..<line.endIndex, in: line)
            if let m = re.firstMatch(in: line, range: range), m.numberOfRanges >= 3,
               let titleRange = Range(m.range(at: 2), in: line) {
                return ">" + String(line[titleRange])
            }
            return line
        }
        var result = lines.joined(separator: "\n")

        // 粗体 **text** -> *text*
        if let boldRegex = try? NSRegularExpression(pattern: "\\*\\*([^*]+)\\*\\*") {
            let range = NSRange(result.startIndex..<result.endIndex, in: result)
            result = boldRegex.stringByReplacingMatches(
                in: result, range: range, withTemplate: "*$1*"
            )
        }
        return result
    }

    private static func escapeSpecialChars(_ text: String) -> String {
        // 不转义 * (粗体)、> (引用)、` (代码已保护)
        let charsToEscape: [Character] = ["_", "[", "]", "(", ")", "~", "#", "+", "-", "=", "|", "{", "}", ".", "!"]
        var result = text
        for ch in charsToEscape {
            result = result.replacingOccurrences(of: String(ch), with: "\\\(ch)")
        }
        return result
    }
}
