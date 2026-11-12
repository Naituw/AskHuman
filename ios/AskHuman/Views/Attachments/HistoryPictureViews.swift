import SwiftUI
import AHCore
import AHProtocol

// The pictures of the history (docs/specs/ios-history-pictures.md): on the cards of a session's earlier exchanges and on
// the History tab's detail page. A picture is an `AssetRef` the Mac describes with a thumbnail; its original comes from
// the Mac when it is opened, through the same download as the files of a request.

/// What a card or a page knows of the pictures of one exchange, in the order the viewer pages through them: the message's
/// pictures, then the pictures of each question's answer.
struct HistoryPictures {
    let message: [AssetRef]
    let replies: [[AssetRef]]

    init(_ exchange: SessionExchange?) {
        message = exchange?.pictures ?? []
        replies = exchange?.qs.map(\.pictures) ?? []
    }

    /// Every picture, including the ones whose file is gone.
    var all: [AssetRef] { message + replies.flatMap { $0 } }
    var isEmpty: Bool { message.isEmpty && !replies.contains { !$0.isEmpty } }
    /// The pictures the viewer can show: those the Mac still has.
    var viewable: [AssetRef] { all.filter { !$0.gone } }

    func replies(of question: Int) -> [AssetRef] { replies.indices.contains(question) ? replies[question] : [] }
}

/// A picture of the history: its thumbnail in a square, or a place held for one whose file the Mac no longer has
/// (HP8). `more` covers the cell with "+N" when it is the last one a strip shows and there are more (HP3).
struct HistoryPictureCell: View {
    let picture: AssetRef
    var more = 0
    var cornerRadius: CGFloat = 10
    var compact = false

    var body: some View {
        Group {
            if picture.gone {
                GonePictureCell(cornerRadius: cornerRadius, compact: compact)
            } else {
                ImageThumbnail(asset: picture, cornerRadius: cornerRadius, compact: compact)
            }
        }
        .overlay {
            if more > 0 {
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .fill(Color.black.opacity(0.5))
                    .overlay { Text(verbatim: "+\(more)").font(.headline).foregroundStyle(.white) }
            }
        }
    }
}

/// Where a picture was whose file is gone from the Mac: a dashed square that says so (a small one has the sign only).
private struct GonePictureCell: View {
    let cornerRadius: CGFloat
    let compact: Bool

    var body: some View {
        Color(.tertiarySystemFill).opacity(0.4)
            .aspectRatio(1, contentMode: .fit)
            .overlay {
                VStack(spacing: 4) {
                    Image(systemName: "photo.badge.exclamationmark").foregroundStyle(.secondary)
                    if !compact {
                        Text("No longer on the Mac")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .multilineTextAlignment(.center)
                            .minimumScaleFactor(0.8)
                            .padding(.horizontal, 4)
                    }
                }
            }
            .overlay {
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .strokeBorder(Color.secondary.opacity(0.5), style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
            }
            .clipShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
            .contentShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
    }
}

/// What a tap on a picture does: a picture the Mac still has opens the viewer on it, paging through the pictures of the
/// exchange; one it no longer has says why there is nothing to open. A picture that was only deleted after the thumbnail
/// came is told apart by the viewer itself ("file unavailable").
private struct HistoryPictureOpening: ViewModifier {
    let macId: String
    let pictures: [AssetRef]
    @Binding var tapped: AssetRef?

    func body(content: Content) -> some View {
        content
            .sheet(item: Binding(
                get: { tapped.flatMap { $0.gone ? nil : $0 } },
                set: { if $0 == nil { tapped = nil } }
            )) { picture in
                ImageGalleryViewer(macId: macId, images: pictures.filter { !$0.gone }, start: picture.assetId)
            }
            .alert("No longer on the Mac", isPresented: Binding(
                get: { tapped?.gone == true },
                set: { if !$0 { tapped = nil } }
            )) {
                Button("OK", role: .cancel) {}
            } message: {
                Text("The file was deleted from the Mac, often because the system clears temporary files. There is nothing left to show.")
            }
    }
}

extension View {
    /// Opens the viewer over `pictures` (or the note for a gone one) when `tapped` is set; it is reset when that is over.
    func historyPictureOpening(macId: String, pictures: [AssetRef], tapped: Binding<AssetRef?>) -> some View {
        modifier(HistoryPictureOpening(macId: macId, pictures: pictures, tapped: tapped))
    }
}

/// How a cell reads out: its name, and what is special about it.
private func pictureLabel(_ picture: AssetRef, more: Int = 0) -> Text {
    if picture.gone { return Text("\(picture.name), no longer on the Mac") }
    if more > 1 { return Text("\(picture.name), and \(more - 1) more") }
    return Text(verbatim: picture.name)
}

/// The row of pictures a folded card shows (HP3): the first four, or the first three and a fourth covered with how many
/// there are from it on. A person who makes the text large gets squares that grow with it and wrap into more rows.
struct HistoryPictureStrip: View {
    let pictures: [AssetRef]
    let open: (AssetRef) -> Void
    @ScaledMetric(relativeTo: .subheadline) private var side: CGFloat = 56
    static let cells = 4

    var body: some View {
        let shown = Array(pictures.prefix(Self.cells))
        LazyVGrid(columns: [GridItem(.adaptive(minimum: side, maximum: side), spacing: 8)], alignment: .leading, spacing: 8) {
            ForEach(Array(shown.enumerated()), id: \.offset) { index, picture in
                let more = pictures.count > Self.cells && index == Self.cells - 1 ? pictures.count - (Self.cells - 1) : 0
                Button { open(picture) } label: {
                    HistoryPictureCell(picture: picture, more: more, cornerRadius: 8, compact: true)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(pictureLabel(picture, more: more))
                .accessibilityIdentifier(picture.gone ? "session-picture-gone-\(index)" : "session-picture-\(index)")
            }
        }
    }
}

/// The pictures of an open card or of the detail page: squares in columns that fit the width (HP4), as on a request's page.
struct HistoryPictureGrid: View {
    let pictures: [AssetRef]
    var idPrefix = "session-picture"
    let open: (AssetRef) -> Void

    var body: some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 96, maximum: 140), spacing: 8)], alignment: .leading, spacing: 8) {
            ForEach(Array(pictures.enumerated()), id: \.offset) { index, picture in
                Button { open(picture) } label: { HistoryPictureCell(picture: picture) }
                    .buttonStyle(.plain)
                    .accessibilityLabel(pictureLabel(picture))
                    .accessibilityIdentifier(picture.gone ? "\(idPrefix)-grid-gone-\(index)" : "\(idPrefix)-grid-\(index)")
            }
        }
    }
}
