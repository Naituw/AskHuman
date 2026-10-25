import Foundation

/// Turns Markdown text into render blocks with Foundation's own parser (`AttributedString`), so the
/// app carries no Markdown dependency. The parser reports structure through `PresentationIntent`
/// attributes on runs; this type rebuilds the block tree from them.
public enum MarkdownParser {
    /// Parses `text`. Never fails: text that cannot be parsed comes back as one plain paragraph.
    public static func parse(_ text: String) -> [MarkdownBlock] {
        let options = AttributedString.MarkdownParsingOptions(
            allowsExtendedAttributes: false,
            interpretedSyntax: .full,
            failurePolicy: .returnPartiallyParsedIfPossible
        )
        guard let parsed = try? AttributedString(markdown: text, options: options) else {
            return text.isEmpty ? [] : [.paragraph(AttributedString(text))]
        }
        return Builder(parsed).build()
    }

    /// The text with Markdown syntax removed (one line, whitespace collapsed).
    public static func plainText(_ text: String) -> String {
        let blocks = parse(text)
        let joined = blocks.map(\.plainText).joined(separator: " ")
        return joined.split(whereSeparator: \.isWhitespace).joined(separator: " ")
    }
}

// MARK: tree building

private final class Node {
    enum Kind {
        case root
        case header(Int)
        case paragraph
        case codeBlock(String?)
        case orderedList
        case unorderedList
        case listItem(Int)
        case blockQuote
        case table([MarkdownTable.Alignment])
        case tableHeaderRow
        case tableRow
        case tableCell
        case thematicBreak
    }

    let kind: Kind
    let identity: Int
    var children: [Node] = []
    var inline = AttributedString()

    init(kind: Kind, identity: Int) {
        self.kind = kind
        self.identity = identity
    }
}

private struct Builder {
    let source: AttributedString

    init(_ source: AttributedString) { self.source = source }

    func build() -> [MarkdownBlock] {
        let root = Node(kind: .root, identity: 0)
        for run in source.runs {
            var content = AttributedString(source[run.range])
            content.presentationIntent = nil
            let components = run.presentationIntent?.components.reversed() ?? []
            insert(content, under: Array(components), into: root)
        }
        return blocks(of: root.children)
    }

    /// Walks the chain of enclosing blocks (outermost first), creating nodes that do not exist yet
    /// (a block is identified by its parser identity), and appends the run to the innermost one.
    private func insert(_ content: AttributedString, under components: [PresentationIntent.IntentType], into root: Node) {
        guard !components.isEmpty else {
            // Text outside any block (inline-only input): one implicit paragraph.
            if let last = root.children.last, case .paragraph = last.kind, last.identity == -1 {
                last.inline.append(content)
            } else {
                let node = Node(kind: .paragraph, identity: -1)
                node.inline = content
                root.children.append(node)
            }
            return
        }
        var parent = root
        for component in components {
            if let existing = parent.children.last(where: { $0.identity == component.identity }) {
                parent = existing
            } else {
                let node = Node(kind: kind(of: component), identity: component.identity)
                parent.children.append(node)
                parent = node
            }
        }
        parent.inline.append(content)
    }

    private func kind(of component: PresentationIntent.IntentType) -> Node.Kind {
        switch component.kind {
        case .paragraph: return .paragraph
        case .header(let level): return .header(level)
        case .orderedList: return .orderedList
        case .unorderedList: return .unorderedList
        case .listItem(let ordinal): return .listItem(ordinal)
        case .codeBlock(let language): return .codeBlock(language)
        case .blockQuote: return .blockQuote
        case .thematicBreak: return .thematicBreak
        case .table(let columns): return .table(columns.map(Self.alignment))
        case .tableHeaderRow: return .tableHeaderRow
        case .tableRow: return .tableRow
        case .tableCell: return .tableCell
        @unknown default: return .paragraph
        }
    }

    private static func alignment(_ column: PresentationIntent.TableColumn) -> MarkdownTable.Alignment {
        switch column.alignment {
        case .center: return .center
        case .right: return .trailing
        default: return .leading
        }
    }

    // MARK: nodes → blocks

    private func blocks(of nodes: [Node]) -> [MarkdownBlock] {
        nodes.flatMap(block(of:))
    }

    private func block(of node: Node) -> [MarkdownBlock] {
        switch node.kind {
        case .root:
            return blocks(of: node.children)
        case .header(let level):
            return inlineBlocks(node.inline) { .heading(level: level, text: $0) }
        case .paragraph:
            return inlineBlocks(node.inline) { .paragraph($0) }
        case .codeBlock(let language):
            var text = String(node.inline.characters)
            if text.hasSuffix("\n") { text.removeLast() }
            let hint = language.flatMap { $0.isEmpty ? nil : $0 }
            return [.code(language: hint, text: text)]
        case .orderedList, .unorderedList:
            let ordered: Bool
            if case .orderedList = node.kind { ordered = true } else { ordered = false }
            let items = node.children.map { MarkdownListItem(blocks: listItemBlocks($0)) }
            var start = 1
            if ordered, let first = node.children.first, case .listItem(let ordinal) = first.kind { start = ordinal }
            return [.list(ordered: ordered, start: start, items: items)]
        case .listItem:
            // A list item outside a list (cannot happen with the parser); keep its content.
            return listItemBlocks(node)
        case .blockQuote:
            return [.quote(blocks(of: node.children))]
        case .table(let alignments):
            return [table(node, alignments: alignments)]
        case .tableHeaderRow, .tableRow, .tableCell:
            return []
        case .thematicBreak:
            return [.rule]
        }
    }

    /// A list item holds paragraphs and nested lists; tight lists put the text straight into the item.
    private func listItemBlocks(_ item: Node) -> [MarkdownBlock] {
        var result: [MarkdownBlock] = []
        if !item.inline.characters.isEmpty {
            result += inlineBlocks(item.inline) { .paragraph($0) }
        }
        result += blocks(of: item.children)
        return result
    }

    private func table(_ node: Node, alignments: [MarkdownTable.Alignment]) -> MarkdownBlock {
        var header: [AttributedString] = []
        var rows: [[AttributedString]] = []
        for child in node.children {
            let cells = child.children.map(\.inline)
            if case .tableHeaderRow = child.kind {
                header = cells
            } else {
                rows.append(cells)
            }
        }
        return .table(MarkdownTable(header: header, rows: rows, alignments: alignments))
    }

    /// Splits inline content at images, which become blocks of their own.
    private func inlineBlocks(_ content: AttributedString, make: (AttributedString) -> MarkdownBlock) -> [MarkdownBlock] {
        var result: [MarkdownBlock] = []
        var current = AttributedString()
        func flush() {
            if !current.characters.isEmpty { result.append(make(current)) }
            current = AttributedString()
        }
        for run in content.runs {
            if let url = run.imageURL {
                flush()
                result.append(.image(source: url.absoluteString, alt: String(content[run.range].characters)))
            } else {
                current.append(AttributedString(content[run.range]))
            }
        }
        flush()
        if result.isEmpty, content.characters.isEmpty { return [] }
        return result
    }
}
