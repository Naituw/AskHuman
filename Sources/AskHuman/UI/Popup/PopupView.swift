import SwiftUI
import AppKit
import UniformTypeIdentifiers

struct PopupView: View {
    @ObservedObject var viewModel: PopupViewModel
    @State private var pasteMonitor: Any?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    messageSection
                    if !viewModel.request.predefinedOptions.isEmpty {
                        optionsSection
                    }
                    inputSection
                    imagesSection
                }
                .padding(20)
            }

            Divider()
            footer
                .padding(16)
        }
        .frame(minWidth: 420, minHeight: 480)
        .onAppear { installPasteMonitor() }
        .onDisappear { removePasteMonitor() }
    }

    /// 拦截 Cmd+V：当剪贴板含图片时，粘贴到附件区而非文本框
    private func installPasteMonitor() {
        guard pasteMonitor == nil else { return }
        pasteMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
            guard event.modifierFlags.contains(.command),
                  event.charactersIgnoringModifiers?.lowercased() == "v" else {
                return event
            }
            let pb = NSPasteboard.general
            let hasImage = pb.canReadObject(forClasses: [NSImage.self], options: nil)
            let imageURLs = (pb.readObjects(forClasses: [NSURL.self], options: nil) as? [URL])?
                .filter { isImageURL($0) } ?? []
            if hasImage || !imageURLs.isEmpty {
                viewModel.addImagesFromPasteboard()
                return nil
            }
            return event
        }
    }

    private func removePasteMonitor() {
        if let monitor = pasteMonitor {
            NSEvent.removeMonitor(monitor)
            pasteMonitor = nil
        }
    }

    private func isImageURL(_ url: URL) -> Bool {
        let ext = url.pathExtension.lowercased()
        return ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "tiff", "heic"].contains(ext)
    }

    @ViewBuilder
    private var messageSection: some View {
        if viewModel.request.isMarkdown {
            switch viewModel.markdownMode {
            case .native:
                MarkdownContentView(markdown: viewModel.request.message)
                    .frame(maxWidth: .infinity, alignment: .leading)
            case .webview:
                MarkdownWebContentView(markdown: viewModel.request.message, theme: viewModel.theme)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        } else {
            Text(viewModel.request.message)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var optionsSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("预定义选项")
                .font(.caption)
                .foregroundStyle(.secondary)
            ForEach(viewModel.request.predefinedOptions, id: \.self) { option in
                Button {
                    viewModel.toggle(option)
                } label: {
                    HStack(alignment: .top, spacing: 8) {
                        Image(systemName: viewModel.isSelected(option) ? "checkmark.square.fill" : "square")
                            .foregroundStyle(viewModel.isSelected(option) ? Color.accentColor : Color.secondary)
                        Text(option)
                            .multilineTextAlignment(.leading)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .padding(10)
                    .background(
                        RoundedRectangle(cornerRadius: 8)
                            .fill(viewModel.isSelected(option) ? Color.accentColor.opacity(0.12) : Color.secondary.opacity(0.06))
                    )
                }
                .buttonStyle(.plain)
            }
        }
    }

    private var inputSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("补充说明")
                .font(.caption)
                .foregroundStyle(.secondary)
            TextEditor(text: $viewModel.userInput)
                .font(.body)
                .frame(minHeight: 90)
                .padding(6)
                .overlay(
                    RoundedRectangle(cornerRadius: 8)
                        .stroke(Color.secondary.opacity(0.25))
                )
        }
    }

    private var imagesSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("图片附件")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
                Button("粘贴", systemImage: "doc.on.clipboard") {
                    viewModel.addImagesFromPasteboard()
                }
                Button("选择文件", systemImage: "photo.on.rectangle") {
                    pickImageFiles()
                }
            }
            if viewModel.images.isEmpty {
                Text("可粘贴、拖拽或选择图片")
                    .font(.caption)
                    .foregroundStyle(.tertiary)
                    .frame(maxWidth: .infinity, minHeight: 60)
                    .background(
                        RoundedRectangle(cornerRadius: 8)
                            .strokeBorder(style: StrokeStyle(lineWidth: 1, dash: [4]))
                            .foregroundStyle(Color.secondary.opacity(0.4))
                    )
            } else {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 10) {
                        ForEach(viewModel.images) { item in
                            thumbnail(item)
                        }
                    }
                }
            }
        }
        .onDrop(of: [.fileURL, .image], isTargeted: nil) { providers in
            handleDrop(providers)
        }
    }

    private func thumbnail(_ item: PopupImageItem) -> some View {
        ZStack(alignment: .topTrailing) {
            Image(nsImage: item.image)
                .resizable()
                .aspectRatio(contentMode: .fill)
                .frame(width: 72, height: 72)
                .clipShape(RoundedRectangle(cornerRadius: 8))
            Button {
                viewModel.removeImage(item)
            } label: {
                Image(systemName: "xmark.circle.fill")
                    .foregroundStyle(.white, .black.opacity(0.6))
            }
            .buttonStyle(.plain)
            .padding(3)
        }
    }

    private var footer: some View {
        HStack {
            Spacer()
            Button("取消") {
                viewModel.cancel()
            }
            .keyboardShortcut(.cancelAction)
            Button("发送") {
                viewModel.send()
            }
            .keyboardShortcut(.return, modifiers: [.command])
            .buttonStyle(.borderedProminent)
        }
    }

    private func pickImageFiles() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [.image]
        if panel.runModal() == .OK {
            for url in panel.urls {
                viewModel.addImage(fromURL: url)
            }
        }
    }

    private func handleDrop(_ providers: [NSItemProvider]) -> Bool {
        var handled = false
        for provider in providers {
            if provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) {
                handled = true
                _ = provider.loadObject(ofClass: URL.self) { url, _ in
                    guard let url else { return }
                    Task { @MainActor in
                        viewModel.addImage(fromURL: url)
                    }
                }
            } else if provider.canLoadObject(ofClass: NSImage.self) {
                handled = true
                _ = provider.loadObject(ofClass: NSImage.self) { obj, _ in
                    guard let img = obj as? NSImage else { return }
                    Task { @MainActor in
                        viewModel.appendImage(img, filename: nil)
                    }
                }
            }
        }
        return handled
    }
}
