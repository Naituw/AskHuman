import AppKit
import SwiftUI
import UniformTypeIdentifiers

struct PopupImageItem: Identifiable {
    let id = UUID()
    let image: NSImage
    let attachment: ImageAttachment
}

@MainActor
final class PopupViewModel: ObservableObject {
    let request: AskRequest

    @Published var selectedOptions: [String] = []
    @Published var userInput: String = ""
    @Published var images: [PopupImageItem] = []

    private var onResult: ((ChannelResult) -> Void)?
    private var didResolve = false

    init(request: AskRequest, onResult: @escaping (ChannelResult) -> Void) {
        self.request = request
        self.onResult = onResult
    }

    var renderedMessage: AttributedString {
        MarkdownRenderer.render(request.message, isMarkdown: request.isMarkdown)
    }

    func isSelected(_ option: String) -> Bool {
        selectedOptions.contains(option)
    }

    func toggle(_ option: String) {
        if let idx = selectedOptions.firstIndex(of: option) {
            selectedOptions.remove(at: idx)
        } else {
            selectedOptions.append(option)
        }
    }

    func send() {
        resolve(ChannelResult(
            action: .send,
            selectedOptions: selectedOptions,
            userInput: userInput,
            images: images.map { $0.attachment },
            sourceChannelId: "popup"
        ))
    }

    func cancel() {
        resolve(.cancel(sourceChannelId: "popup"))
    }

    private func resolve(_ result: ChannelResult) {
        guard !didResolve else { return }
        didResolve = true
        onResult?(result)
        onResult = nil
    }

    /// 标记已被外部解决（如其他 Channel 抢答或窗口被强制关闭），避免再次回调
    func markResolvedSilently() {
        didResolve = true
        onResult = nil
    }

    // MARK: - 图片采集

    func addImagesFromPasteboard() {
        let pb = NSPasteboard.general
        if let urls = pb.readObjects(forClasses: [NSURL.self], options: nil) as? [URL], !urls.isEmpty {
            for url in urls {
                addImage(fromURL: url)
            }
            return
        }
        if let objects = pb.readObjects(forClasses: [NSImage.self], options: nil) as? [NSImage] {
            for img in objects {
                appendImage(img, filename: nil)
            }
        }
    }

    func addImage(fromURL url: URL) {
        guard let data = try? Data(contentsOf: url) else { return }
        let mediaType = Self.mediaType(forExtension: url.pathExtension)
        let attachment = ImageAttachment(
            data: data.base64EncodedString(),
            mediaType: mediaType,
            filename: url.lastPathComponent
        )
        if let img = NSImage(data: data) {
            images.append(PopupImageItem(image: img, attachment: attachment))
        }
    }

    func appendImage(_ image: NSImage, filename: String?) {
        guard let data = pngData(from: image) else { return }
        let attachment = ImageAttachment(
            data: data.base64EncodedString(),
            mediaType: "image/png",
            filename: filename
        )
        images.append(PopupImageItem(image: image, attachment: attachment))
    }

    func removeImage(_ item: PopupImageItem) {
        images.removeAll { $0.id == item.id }
    }

    private func pngData(from image: NSImage) -> Data? {
        guard let tiff = image.tiffRepresentation,
              let rep = NSBitmapImageRep(data: tiff) else { return nil }
        return rep.representation(using: .png, properties: [:])
    }

    static func mediaType(forExtension ext: String) -> String {
        switch ext.lowercased() {
        case "png": return "image/png"
        case "jpg", "jpeg": return "image/jpeg"
        case "gif": return "image/gif"
        case "webp": return "image/webp"
        case "bmp": return "image/bmp"
        case "svg": return "image/svg+xml"
        default: return "application/octet-stream"
        }
    }
}
