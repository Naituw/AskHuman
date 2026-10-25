import SwiftUI
import AHMarkdown

/// Renders Markdown as native SwiftUI blocks.
struct MarkdownView: View {
    let text: String
    var baseFont: Font = .body

    @Environment(LinkPresenter.self) private var links

    var body: some View {
        MarkdownBlocks(blocks: MarkdownCache.blocks(for: text), baseFont: baseFont)
            .environment(\.openURL, OpenURLAction { url in links.request(url) })
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// Parsed documents, so that scrolling and redraws do not parse again.
enum MarkdownCache {
    private final class Box {
        let blocks: [MarkdownBlock]
        init(_ blocks: [MarkdownBlock]) { self.blocks = blocks }
    }

    // NSCache is thread-safe.
    nonisolated(unsafe) private static let cache: NSCache<NSString, Box> = {
        let cache = NSCache<NSString, Box>()
        cache.countLimit = 200
        return cache
    }()

    static func blocks(for text: String) -> [MarkdownBlock] {
        let key = text as NSString
        if let hit = cache.object(forKey: key) { return hit.blocks }
        let blocks = MarkdownParser.parse(text)
        cache.setObject(Box(blocks), forKey: key)
        return blocks
    }
}

private struct MarkdownBlocks: View {
    let blocks: [MarkdownBlock]
    let baseFont: Font

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { _, block in
                MarkdownBlockView(block: block, baseFont: baseFont)
            }
        }
    }
}

private struct MarkdownBlockView: View {
    let block: MarkdownBlock
    let baseFont: Font

    var body: some View {
        switch block {
        case .heading(let level, let text):
            Text(text)
                .font(Self.headingFont(level))
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, level <= 2 ? 4 : 0)
                .accessibilityAddTraits(.isHeader)
        case .paragraph(let text):
            Text(text)
                .font(baseFont)
                .fixedSize(horizontal: false, vertical: true)
        case .list(let ordered, let start, let items):
            MarkdownList(ordered: ordered, start: start, items: items, baseFont: baseFont)
        case .quote(let blocks):
            HStack(alignment: .top, spacing: 10) {
                RoundedRectangle(cornerRadius: 1.5).fill(Color(.tertiaryLabel)).frame(width: 3)
                MarkdownBlocks(blocks: blocks, baseFont: baseFont)
                    .foregroundStyle(.secondary)
            }
            .fixedSize(horizontal: false, vertical: true)
        case .code(let language, let text):
            CodeBlock(language: language, text: text)
        case .table(let table):
            MarkdownTableView(table: table, baseFont: baseFont)
        case .image(_, let alt):
            ImagePlaceholder(alt: alt)
        case .rule:
            Divider()
        }
    }

    static func headingFont(_ level: Int) -> Font {
        switch level {
        case 1: .title2.weight(.bold)
        case 2: .title3.weight(.bold)
        case 3: .headline
        default: .subheadline.weight(.semibold)
        }
    }
}

private struct MarkdownList: View {
    let ordered: Bool
    let start: Int
    let items: [MarkdownListItem]
    let baseFont: Font

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(Array(items.enumerated()), id: \.offset) { index, item in
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(ordered ? "\(start + index)." : "•")
                        .font(baseFont)
                        .foregroundStyle(.secondary)
                        .frame(minWidth: ordered ? 22 : 10, alignment: .trailing)
                    MarkdownBlocks(blocks: item.blocks, baseFont: baseFont)
                }
            }
        }
    }
}

private struct CodeBlock: View {
    let language: String?
    let text: String
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(language ?? "").font(.caption2.weight(.medium)).foregroundStyle(.secondary)
                Spacer()
                Button {
                    UIPasteboard.general.string = text
                    copied = true
                } label: {
                    Image(systemName: copied ? "checkmark" : "doc.on.doc").imageScale(.small)
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
                .accessibilityLabel("Copy code")
            }
            .padding(.horizontal, 10)
            .padding(.top, 6)
            ScrollView(.horizontal, showsIndicators: false) {
                Text(text)
                    .font(.system(.footnote, design: .monospaced))
                    .textSelection(.enabled)
                    .padding(.horizontal, 10)
                    .padding(.bottom, 10)
                    .padding(.top, 4)
            }
        }
        .background(Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .task(id: copied) {
            guard copied else { return }
            try? await Task.sleep(for: .seconds(1.5))
            copied = false
        }
    }
}

private struct MarkdownTableView: View {
    let table: MarkdownTable
    let baseFont: Font

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            Grid(alignment: .leading, horizontalSpacing: 0, verticalSpacing: 0) {
                GridRow {
                    ForEach(Array(table.header.enumerated()), id: \.offset) { column, cell in
                        cellView(cell, column: column, header: true)
                    }
                }
                .background(Color(.tertiarySystemFill))
                ForEach(Array(table.rows.enumerated()), id: \.offset) { _, row in
                    Divider().gridCellUnsizedAxes(.horizontal)
                    GridRow {
                        ForEach(Array(row.enumerated()), id: \.offset) { column, cell in
                            cellView(cell, column: column, header: false)
                        }
                    }
                }
            }
            .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).stroke(Color(.separator), lineWidth: 0.5))
            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        }
    }

    private func cellView(_ cell: AttributedString, column: Int, header: Bool) -> some View {
        let alignment = table.alignments.indices.contains(column) ? table.alignments[column] : .leading
        return Text(cell)
            .font(header ? .subheadline.weight(.semibold) : .subheadline)
            .multilineTextAlignment(alignment == .trailing ? .trailing : alignment == .center ? .center : .leading)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .frame(minWidth: 60, maxWidth: .infinity, alignment: alignment == .trailing ? .trailing : alignment == .center ? .center : .leading)
            .gridColumnAlignment(alignment == .trailing ? .trailing : alignment == .center ? .center : .leading)
    }
}

/// Pictures in the text are not fetched yet; the alt text stands in for them.
private struct ImagePlaceholder: View {
    let alt: String

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "photo").foregroundStyle(.secondary)
            Text(alt.isEmpty ? String(localized: "Image") : alt)
                .font(.footnote)
                .foregroundStyle(.secondary)
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
    }
}
