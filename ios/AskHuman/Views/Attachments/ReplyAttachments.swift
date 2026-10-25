import SwiftUI
import PhotosUI
import UIKit
import UniformTypeIdentifiers
import AHCore
import AHProtocol
import AHStore

/// Adding photos and files to the reply of one question.
struct ReplyAttachmentBar: View {
    let composer: AnswerComposer
    let question: Int

    @State private var photoItems: [PhotosPickerItem] = []
    @State private var showPhotos = false
    @State private var showCamera = false
    @State private var showFiles = false
    @State private var problem: String?
    @State private var working = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            let attachments = composer.answers.indices.contains(question) ? composer.answers[question].attachments : []
            ForEach(attachments) { attachment in
                ReplyAttachmentRow(attachment: attachment, url: composer.url(of: attachment)) {
                    composer.removeAttachment(id: attachment.id, question: question)
                }
            }
            if composer.canAttachMore {
                Menu {
                    Button { showPhotos = true } label: { Label("Photo Library", systemImage: "photo.on.rectangle") }
                    if UIImagePickerController.isSourceTypeAvailable(.camera) {
                        Button { showCamera = true } label: { Label("Take Photo", systemImage: "camera") }
                    }
                    Button { showFiles = true } label: { Label("Choose File", systemImage: "doc") }
                } label: {
                    HStack(spacing: 6) {
                        if working { ProgressView().controlSize(.small) } else { Image(systemName: "paperclip") }
                        Text("Add attachment")
                    }
                    .font(.subheadline)
                }
                .disabled(working)
                .accessibilityIdentifier("add-attachment-\(question)")
            }
        }
        .photosPicker(
            isPresented: $showPhotos, selection: $photoItems,
            maxSelectionCount: max(1, Wire.maxReplyAttachments - composer.attachmentCount), matching: .images
        )
        .onChange(of: photoItems) { _, items in
            guard !items.isEmpty else { return }
            photoItems = []
            Task { await addPhotos(items) }
        }
        .fullScreenCover(isPresented: $showCamera) {
            CameraPicker { image in
                if let data = image.jpegData(compressionQuality: 0.9) { addImage(data, name: "Photo.jpg") }
            }
            .ignoresSafeArea()
        }
        .fileImporter(isPresented: $showFiles, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            guard case .success(let urls) = result else { return }
            Task { await addFiles(urls) }
        }
        .alert("Could not attach", isPresented: Binding(get: { problem != nil }, set: { if !$0 { problem = nil } })) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(problem ?? "")
        }
    }

    // MARK: adding

    private func addPhotos(_ items: [PhotosPickerItem]) async {
        working = true
        defer { working = false }
        for (index, item) in items.enumerated() {
            guard let data = try? await item.loadTransferable(type: Data.self) else {
                problem = String(localized: "A photo could not be read.")
                continue
            }
            addImage(data, name: items.count > 1 ? "Photo-\(index + 1).jpg" : "Photo.jpg")
        }
    }

    private func addImage(_ data: Data, name: String) {
        guard let output = ImageDownscaler.jpeg(from: data) else {
            problem = String(localized: "That image could not be processed.")
            return
        }
        add(name: name, mime: "image/jpeg", data: output.data, isImage: true)
    }

    private func addFiles(_ urls: [URL]) async {
        working = true
        defer { working = false }
        for url in urls {
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            let size = (try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
            guard size <= Wire.maxAssetBytes else {
                problem = BackendError.tooLarge.userMessage
                continue
            }
            guard let data = try? Data(contentsOf: url) else {
                problem = String(localized: "The file could not be read.")
                continue
            }
            let type = UTType(filenameExtension: url.pathExtension)
            if type?.conforms(to: .image) == true {
                addImage(data, name: url.deletingPathExtension().lastPathComponent + ".jpg")
            } else {
                add(name: url.lastPathComponent, mime: type?.preferredMIMEType ?? "application/octet-stream", data: data, isImage: false)
            }
        }
    }

    private func add(name: String, mime: String, data: Data, isImage: Bool) {
        do {
            try composer.addAttachment(name: name, mime: mime, data: data, isImage: isImage, question: question)
        } catch {
            problem = error.userMessage
        }
    }
}

private struct ReplyAttachmentRow: View {
    let attachment: DraftAttachment
    let url: URL
    let onRemove: () -> Void
    @State private var thumbnail: UIImage?

    var body: some View {
        HStack(spacing: 10) {
            Group {
                if let thumbnail {
                    Image(uiImage: thumbnail).resizable().scaledToFill()
                } else {
                    Image(systemName: attachment.isImage ? "photo" : "doc").foregroundStyle(.secondary)
                }
            }
            .frame(width: 40, height: 40)
            .background(Color(.tertiarySystemFill))
            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
            VStack(alignment: .leading, spacing: 1) {
                Text(attachment.name).font(.subheadline).lineLimit(1).truncationMode(.middle)
                Text(attachment.size.formatted(.file)).font(.caption).foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
            Button(role: .destructive, action: onRemove) {
                Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Remove attachment")
        }
        .padding(8)
        .background(Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
        .task(id: attachment.id) {
            guard attachment.isImage else { return }
            let url = url
            thumbnail = await Task.detached { Self.thumbnail(at: url) }.value
        }
    }

    private nonisolated static func thumbnail(at url: URL) -> UIImage? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: 120,
        ]
        guard let cg = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }
        return UIImage(cgImage: cg)
    }
}

/// The system camera.
struct CameraPicker: UIViewControllerRepresentable {
    let onImage: (UIImage) -> Void
    @Environment(\.dismiss) private var dismiss

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeUIViewController(context: Context) -> UIImagePickerController {
        let picker = UIImagePickerController()
        picker.sourceType = .camera
        picker.delegate = context.coordinator
        return picker
    }

    func updateUIViewController(_ controller: UIImagePickerController, context: Context) {}

    final class Coordinator: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
        let parent: CameraPicker
        init(_ parent: CameraPicker) { self.parent = parent }

        func imagePickerController(_ picker: UIImagePickerController, didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]) {
            if let image = info[.originalImage] as? UIImage { parent.onImage(image) }
            parent.dismiss()
        }

        func imagePickerControllerDidCancel(_ picker: UIImagePickerController) {
            parent.dismiss()
        }
    }
}
