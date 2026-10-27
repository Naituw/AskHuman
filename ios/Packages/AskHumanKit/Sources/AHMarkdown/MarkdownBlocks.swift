import Foundation

/// One block of a rendered Markdown document. The tree is plain data: the app target turns it into
/// SwiftUI views, tests compare it directly.
public indirect enum MarkdownBlock: Equatable, Sendable {
    case heading(level: Int, text: AttributedString)
    case paragraph(AttributedString)
    case list(ordered: Bool, start: Int, items: [MarkdownListItem])
    case quote([MarkdownBlock])
    case code(language: String?, text: String)
    case table(MarkdownTable)
    /// `source` is the destination as written, `alt` the description.
    case image(source: String, alt: String)
    case rule
}

public struct MarkdownListItem: Equatable, Sendable {
    public var blocks: [MarkdownBlock]

    public init(blocks: [MarkdownBlock]) {
        self.blocks = blocks
    }
}

public struct MarkdownTable: Equatable, Sendable {
    public enum Alignment: Sendable, Equatable { case leading, center, trailing }

    public var header: [AttributedString]
    public var rows: [[AttributedString]]
    public var alignments: [Alignment]

    public init(header: [AttributedString], rows: [[AttributedString]], alignments: [Alignment]) {
        self.header = header
        self.rows = rows
        self.alignments = alignments
    }
}

extension MarkdownBlock {
    /// The text of the block without any markup, for list previews and accessibility.
    public var plainText: String {
        switch self {
        case .heading(_, let text), .paragraph(let text):
            return String(text.characters)
        case .list(_, _, let items):
            return items.map { $0.blocks.map(\.plainText).joined(separator: " ") }.joined(separator: " ")
        case .quote(let blocks):
            return blocks.map(\.plainText).joined(separator: " ")
        case .code(_, let text):
            return text
        case .table(let table):
            return (table.header + table.rows.flatMap { $0 }).map { String($0.characters) }.joined(separator: " ")
        case .image(_, let alt):
            return alt
        case .rule:
            return ""
        }
    }
}
