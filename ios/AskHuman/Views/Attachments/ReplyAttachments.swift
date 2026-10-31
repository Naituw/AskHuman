import SwiftUI
import PhotosUI
import UIKit
import UniformTypeIdentifiers
import CoreTransferable
import AHCore
import AHProtocol
import AHStore

/// Adding photos, videos and files to the reply of one question.
struct ReplyAttachmentBar: View {
    let composer: AnswerComposer
    let question: Int
    var revealTarget: InputRevealTarget?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            let attachments = composer.answers.indices.contains(question) ? composer.answers[question].attachments : []
            ForEach(attachments) { attachment in
                ReplyAttachmentRow(attachment: attachment, url: composer.url(of: attachment)) {
                    composer.removeAttachment(id: attachment.id, question: question)
                }
            }
            if composer.canAttachMore {
                AttachmentMenu(remaining: Wire.maxReplyAttachments - composer.attachmentCount, identifier: "add-attachment-\(question)", revealTarget: revealTarget, retainedPreparation: composer.attachmentPreparation(for: question), preparationChanged: { id, active in
                    composer.setAttachmentPreparation(id, active: active)
                }) { name, mime, data, isImage in
                    try composer.addAttachment(name: name, mime: mime, data: data, isImage: isImage, question: question)
                }
            }
        }
    }
}

/// The "Add attachment" menu: photo library, camera and files. Every picked file goes through
/// `add`, which stores it or throws why it cannot.
struct AttachmentMenu: View {
    /// How many more files the form accepts.
    let remaining: Int
    let identifier: String
    var compact = false
    var fillsRow = false
    var revealTarget: InputRevealTarget?
    var retainedPreparation: AttachmentPreparationState?
    var preparationChanged: (UUID, Bool) -> Void = { _, _ in }
    var progressChanged: (AttachmentPreparation?) -> Void = { _ in }
    let add: (_ name: String, _ mime: String, _ data: Data, _ isImage: Bool) throws -> Void

    @State private var photoItems: [PhotosPickerItem] = []
    @State private var showPhotos = false
    @State private var showCamera = false
    @State private var showFiles = false
    @State private var localPreparation = AttachmentPreparationState()
    private var state: AttachmentPreparationState { retainedPreparation ?? localPreparation }
    private var problem: String? { get { state.problem } nonmutating set { state.problem = newValue } }
    private var working: Bool { get { state.working } nonmutating set { state.working = newValue } }
    private var status: String { get { state.status } nonmutating set { state.status = newValue } }
    private var fraction: Double? { get { state.fraction } nonmutating set { state.fraction = newValue } }
    private var operation: Task<Void, Never>? { get { state.operation } nonmutating set { state.operation = newValue } }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            pickerMenu
                .background(InputRevealAnchor(target: revealTarget, part: .attachment))
            if working && !compact {
                AttachmentPreparationView(preparation: preparation)
            }
        }
    }

    private var preparation: AttachmentPreparation {
        AttachmentPreparation(status: status, fraction: fraction, cancel: { operation?.cancel() })
    }

    private var pickerMenu: some View {
        Menu {
            if working {
                Text(status)
                Button("Cancel adding attachments", role: .destructive) { operation?.cancel() }
            } else {
                Button { showPhotos = true } label: { Label("Photo Library", systemImage: "photo.on.rectangle") }
                if UIImagePickerController.isSourceTypeAvailable(.camera) {
                    Button { showCamera = true } label: { Label("Take Photo", systemImage: "camera") }
                }
                Button { showFiles = true } label: { Label("Choose File", systemImage: "doc") }
            }
        } label: {
            if compact {
                // The compact menu is the attach button of a compose bar, a glass circle on the floating bar
                // (`barRoundButton`), which sizes itself around the glyph.
                let glyph = Group {
                    if working { ProgressView().controlSize(.small) } else { Image(systemName: "paperclip") }
                }
                if BottomBar.floats {
                    glyph.barRoundGlyph()
                } else {
                    glyph.font(.title3).frame(width: 32, height: 32)
                }
            } else {
                HStack(spacing: 6) {
                    if working { ProgressView().controlSize(.small) } else { Image(systemName: "paperclip") }
                    Text("Add attachment")
                }
                .font(.subheadline)
                .frame(maxWidth: fillsRow ? .infinity : nil, alignment: .leading)
                .contentShape(.rect)
            }
        }
        .barRoundButton(compact)
        .accessibilityLabel("Add attachment")
        .accessibilityValue(working ? status : "")
        .accessibilityIdentifier(identifier)
        .photosPicker(isPresented: $showPhotos, selection: $photoItems, maxSelectionCount: max(1, remaining), matching: .any(of: [.images, .videos]), preferredItemEncoding: .current)
        .onChange(of: photoItems) { _, items in
            guard !items.isEmpty else { return }
            photoItems = []
            start { await addMedia(items) }
        }
        .fullScreenCover(isPresented: $showCamera) {
            CameraPicker { image in
                if let data = image.jpegData(compressionQuality: 0.9) { addImage(data, name: "Photo.jpg") }
            }
            .ignoresSafeArea()
        }
        .fileImporter(isPresented: $showFiles, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            guard case .success(let urls) = result else { return }
            start { await addFiles(urls) }
        }
        .onDisappear { if retainedPreparation == nil { operation?.cancel() } }
        .alert("Could not attach", isPresented: Binding(get: { problem != nil }, set: { if !$0 { problem = nil } })) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(problem ?? "")
        }
    }

    // MARK: adding

    private func start(_ action: @escaping @MainActor () async -> Void) {
        guard !working else { return }
        let id = UUID()
        working = true
        updateStatus(String(localized: "Loading attachment…"))
        preparationChanged(id, true)
        operation = Task {
            defer {
                working = false
                operation = nil
                preparationChanged(id, false)
                progressChanged(nil)
            }
            await action()
        }
    }

    private func updateStatus(_ status: String, fraction: Double? = nil) {
        self.status = status
        self.fraction = fraction
        progressChanged(preparation)
    }

    private func addMedia(_ items: [PhotosPickerItem]) async {
        for (index, item) in items.prefix(remaining).enumerated() {
            do {
                try Task.checkCancellation()
                updateStatus(String(localized: "Loading attachment…"))
                let types = item.supportedContentTypes
                if !types.contains(where: { $0.conforms(to: .image) }),
                   types.contains(where: { $0.conforms(to: .movie) || $0.conforms(to: .video) }) {
                    guard let movie = try await load(item, type: PickedMovie.self) else { throw VideoAttachmentError.unreadable }
                    defer { movie.file.remove() }
                    try await addVideo(movie.file.url)
                } else {
                    guard let data = try await load(item, type: Data.self) else {
                        problem = String(localized: "A photo could not be read.")
                        continue
                    }
                    try Task.checkCancellation()
                    addImage(data, name: items.count > 1 ? "Photo-\(index + 1).jpg" : "Photo.jpg")
                }
            } catch {
                if Task.isCancelled || error is CancellationError { break }
                problem = error.userMessage
            }
        }
    }

    private func addVideo(_ url: URL) async throws {
        let output = try await VideoAttachmentPreparer.prepare(url: url) { stage in
            switch stage {
            case .inspecting: updateStatus(String(localized: "Checking video size…"))
            case .compressing(let value):
                updateStatus(String(localized: "Compressing video…"), fraction: min(1, max(0, value)))
            }
        }
        try Task.checkCancellation()
        try add(output.name, output.mime, output.data, false)
    }

    private func load<T: Transferable>(_ item: PhotosPickerItem, type: T.Type) async throws -> T? {
        let cancellation = PickerTransferCancellation()
        return try await withTaskCancellationHandler {
            try Task.checkCancellation()
            return try await withCheckedThrowingContinuation { continuation in
                cancellation.register(item.loadTransferable(type: type) { result in
                    continuation.resume(with: result)
                })
            }
        } onCancel: {
            cancellation.cancel()
        }
    }

    private func addImage(_ data: Data, name: String) {
        guard let output = ImageDownscaler.jpeg(from: data) else {
            problem = String(localized: "That image could not be processed.")
            return
        }
        store(name: name, mime: "image/jpeg", data: output.data, isImage: true)
    }

    private func addFiles(_ urls: [URL]) async {
        if urls.count > remaining {
            problem = String(localized: "Some files were not added because the attachment limit was reached.")
        }
        for url in urls.prefix(remaining) {
            do {
                try Task.checkCancellation()
                updateStatus(String(localized: "Loading attachment…"))
                let scoped = url.startAccessingSecurityScopedResource()
                defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                let type = UTType(filenameExtension: url.pathExtension)
                if type?.conforms(to: .movie) == true || type?.conforms(to: .video) == true {
                    try await addVideo(url)
                    continue
                }
                let size = (try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
                guard size <= Wire.maxAssetBytes else {
                    problem = BackendError.tooLarge.userMessage
                    continue
                }
                guard let data = try? Data(contentsOf: url) else {
                    problem = String(localized: "The file could not be read.")
                    continue
                }
                try Task.checkCancellation()
                if type?.conforms(to: .image) == true {
                    addImage(data, name: url.deletingPathExtension().lastPathComponent + ".jpg")
                } else {
                    store(name: url.lastPathComponent, mime: type?.preferredMIMEType ?? "application/octet-stream", data: data, isImage: false)
                }
            } catch {
                if Task.isCancelled || error is CancellationError { break }
                problem = error.userMessage
            }
        }
    }

    private func store(name: String, mime: String, data: Data, isImage: Bool) {
        do {
            try add(name, mime, data, isImage)
        } catch {
            problem = error.userMessage
        }
    }
}

struct AttachmentPreparation {
    let status: String
    let fraction: Double?
    let cancel: () -> Void
}

/// Always-visible progress and cancellation, including above the compact interjection bar.
struct AttachmentPreparationView: View {
    let preparation: AttachmentPreparation

    var body: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    if preparation.fraction == nil { ProgressView().controlSize(.small) }
                    Text(preparation.status)
                        .accessibilityIdentifier("attachment-preparation-status")
                    Spacer(minLength: 4)
                    if let fraction = preparation.fraction {
                        Text(fraction.formatted(.percent.precision(.fractionLength(0))))
                            .monospacedDigit()
                    }
                }
                if let fraction = preparation.fraction {
                    ProgressView(value: fraction)
                        .accessibilityLabel(preparation.status)
                        .accessibilityIdentifier("attachment-compression-progress")
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            Button("Cancel", action: preparation.cancel)
                .buttonStyle(.bordered)
                .controlSize(.small)
                .accessibilityLabel("Cancel adding attachments")
                .accessibilityIdentifier("cancel-attachment-preparation")
        }
    }
}

struct ReplyAttachmentRow: View {
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
                    Image(systemName: attachment.isImage ? "photo" : (attachment.mime.hasPrefix("video/") ? "film" : "doc")).foregroundStyle(.secondary)
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

/// Photos only guarantees the received URL during import. Own a copy until preparation finishes,
/// including cancellation or a failed transfer whose value is discarded by the provider.
private final class PickedMovieFile: Sendable {
    let url: URL
    private let directory: URL
    init(copying source: URL) throws {
        directory = FileManager.default.temporaryDirectory.appendingPathComponent("ah-picked-video-\(UUID().uuidString)")
        url = directory.appendingPathComponent(source.lastPathComponent)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        do {
            try FileManager.default.copyItem(at: source, to: url)
            try Task.checkCancellation()
        } catch {
            try? FileManager.default.removeItem(at: directory)
            throw error
        }
    }
    func remove() { try? FileManager.default.removeItem(at: directory) }
    deinit { remove() }
}

private struct PickedMovie: Transferable {
    let file: PickedMovieFile
    static var transferRepresentation: some TransferRepresentation {
        FileRepresentation(importedContentType: .movie) { received in
            try Task.checkCancellation()
            return PickedMovie(file: try PickedMovieFile(copying: received.file))
        }
    }
}

private final class PickerTransferCancellation: @unchecked Sendable {
    private let lock = NSLock()
    private var progress: Progress?
    private var cancelled = false
    func register(_ progress: Progress) {
        lock.lock()
        self.progress = progress
        let shouldCancel = cancelled
        lock.unlock()
        if shouldCancel { progress.cancel() }
    }
    func cancel() {
        lock.lock()
        cancelled = true
        let progress = progress
        lock.unlock()
        progress?.cancel()
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
