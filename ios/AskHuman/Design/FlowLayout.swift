import SwiftUI

/// Lays children out left to right and wraps to a new line when the row is full.
/// Used for chips, so one never breaks across two lines. A child wider than the row is offered the
/// row's width and shortens itself: the layout is never wider than the space it was given.
struct FlowLayout: Layout {
    var spacing: CGFloat = 6
    var lineSpacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(width: proposal.width, subviews: subviews)
        let width = rows.map(\.width).max() ?? 0
        let height = rows.reduce(0) { $0 + $1.height } + CGFloat(max(rows.count - 1, 0)) * lineSpacing
        return CGSize(width: width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in arrange(width: bounds.width, subviews: subviews) {
            var x = bounds.minX
            for index in row.indices {
                let size = measure(subviews[index], width: bounds.width)
                subviews[index].place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
                x += size.width + spacing
            }
            y += row.height + lineSpacing
        }
    }

    private struct Row {
        var indices: [Int] = []
        var width: CGFloat = 0
        var height: CGFloat = 0
    }

    /// The child's size when the row is `width` wide (`nil` or infinite: its ideal size).
    private func measure(_ subview: LayoutSubview, width: CGFloat?) -> CGSize {
        guard let width, width.isFinite else { return subview.sizeThatFits(.unspecified) }
        return subview.sizeThatFits(ProposedViewSize(width: width, height: nil))
    }

    private func arrange(width: CGFloat?, subviews: Subviews) -> [Row] {
        let limit = (width?.isFinite ?? false) ? width! : .infinity
        var rows: [Row] = []
        var current = Row()
        for index in subviews.indices {
            let size = measure(subviews[index], width: width)
            let needed = current.indices.isEmpty ? size.width : current.width + spacing + size.width
            if needed > limit, !current.indices.isEmpty {
                rows.append(current)
                current = Row()
            }
            current.width = current.indices.isEmpty ? size.width : current.width + spacing + size.width
            current.height = max(current.height, size.height)
            current.indices.append(index)
        }
        if !current.indices.isEmpty { rows.append(current) }
        return rows
    }
}
