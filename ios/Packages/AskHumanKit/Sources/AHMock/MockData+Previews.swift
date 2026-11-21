import Foundation
import CoreGraphics
import AHProtocol
import AHCore
import AHStore

// Sample content of the `previews` scenario (docs/specs/ios-attachment-traffic.md): a request whose pictures a Mac that makes
// previews describes with `AssetRef.preview`, and the files a viewer then gets of them: a downscaled picture first, the file
// when the viewer asks for it.

extension MockData {
    /// The request of the `previews` scenario.
    public static let previewsRequestId = "req-previews"
    /// A picture whose preview the phone cannot open: the viewer falls back to the file without an error (AT18).
    public static let undecodablePreviewAssetId = "asset-prev-broken"
    /// A picture whose file the Mac fails to send the first time it is asked for it.
    public static let flakyOriginalAssetId = "asset-prev-flaky"

    /// The pictures of the request, as the Mac describes them. Only a picture with `preview` has one to send.
    private static let previewPictures: [(id: String, name: String, width: Int, height: Int, hue: CGFloat, size: UInt64, preview: Bool)] = [
        ("asset-prev-wide", "wide-big.png", 1600, 900, 0.58, 3_400_000, true),
        ("asset-prev-tall", "tall-big.png", 900, 1600, 0.08, 2_100_000, true),
        ("asset-prev-small", "small.png", 800, 600, 0.33, 180_000, false),
        (undecodablePreviewAssetId, "undecodable-preview.png", 1000, 1000, 0.80, 1_200_000, true),
        (flakyOriginalAssetId, "flaky-original.png", 1200, 800, 0.15, 900_000, true),
    ]

    /// A request with five pictures: two that have previews, one too small for one, one whose preview cannot be opened, and
    /// one whose file the Mac fails to send at the first try. The thumbnails keep the shape of the pictures.
    public static func previewsAsk(now: Date = Date()) -> RequestView {
        let files: [AssetRef] = previewPictures.enumerated().map { index, shape in
            let edge = 192.0 / Double(max(shape.width, shape.height))
            let thumb = picture(
                number: index + 1, width: max(1, Int(Double(shape.width) * edge)), height: max(1, Int(Double(shape.height) * edge)),
                hue: shape.hue, type: .jpeg
            )
            return AssetRef(
                assetId: shape.id, name: shape.name, size: shape.size, mime: "image/png", isImage: true,
                thumb: thumb.map(Bytes.b64u), preview: shape.preview
            )
        }
        return RequestView(
            id: previewsRequestId, kind: "ask", createdAt: ms(now.addingTimeInterval(-30)), agent: AgentRef(kind: "cursor", label: "Cursor"),
            project: ProjectRef(key: "proj-humaninloop", name: "HumanInLoop"), sessionTitle: "Review the new screenshots",
            source: "cursor", contentHash: "h-previews",
            ask: AskView(
                isMarkdown: true,
                message: MessageView(text: "五张截图：两张大的（有预览）、一张小的、一张预览打不开的、一张原图第一次取不到的。", files: files),
                questions: [
                    QuestionView(text: "Do these look right?", options: [
                        OptionView(text: "Yes", recommended: true),
                        OptionView(text: "No", recommended: false),
                    ]),
                ],
                selectOnly: true, single: true
            )
        )
    }

    public static func previewsOnly(now: Date = Date()) -> [InboxItem] {
        items([(homeMacId, "Home MacBook Pro", previewsAsk(now: now), true, -30)], now: now)
    }

    /// What the Mac sends of a picture that has a preview, when the phone asks for that: the picture at half its size, as a
    /// JPEG. A picture whose preview is meant to be unreadable gets bytes that are no image at all.
    static func previewContents(of asset: AssetRef) -> Data {
        if asset.assetId == undecodablePreviewAssetId { return Data("not an image".utf8) }
        if let index = previewPictures.firstIndex(where: { $0.id == asset.assetId }) {
            let shape = previewPictures[index]
            return picture(
                number: index + 1, width: max(1, shape.width / 2), height: max(1, shape.height / 2), hue: shape.hue, type: .jpeg
            ) ?? Data()
        }
        if let shape = historyPictureShape(of: asset.assetId) {
            return picture(
                number: shape.number, width: max(1, shape.width / 2), height: max(1, shape.height / 2), hue: shape.hue, type: .jpeg
            ) ?? Data()
        }
        return chartPNG() ?? Data()
    }

    /// The file of one of the pictures of the request (`MockData.contents(of:)` asks here first).
    static func previewPictureContents(of assetId: String) -> Data? {
        guard let index = previewPictures.firstIndex(where: { $0.id == assetId }) else { return nil }
        let shape = previewPictures[index]
        return picture(number: index + 1, width: shape.width, height: shape.height, hue: shape.hue, type: .png) ?? Data()
    }
}
