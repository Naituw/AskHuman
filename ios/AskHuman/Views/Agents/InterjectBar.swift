import SwiftUI
import AHCore
import AHProtocol

/// The message field at the bottom of a working Agent's page.
struct InterjectBar: View {
    let item: AgentItem
    let composer: InterjectComposer
    var onSent: (InterjectOutcome) -> Void
    var onProblem: (String) -> Void

    @Environment(AppStore.self) private var store
    @FocusState private var focused: Bool
    @State private var preparation: AttachmentPreparation?

    var body: some View {
        @Bindable var composer = composer
        GlassGroup(spacing: 4) {
            VStack(spacing: 8) {
                if BottomBar.floats {
                    // The files and their upload are one panel of glass: on their own they would sit straight on
                    // the content that scrolls by underneath.
                    if !composer.attachments.isEmpty || composer.uploadProgress != nil || preparation != nil {
                        VStack(spacing: 8) {
                            attachmentRows
                            uploadProgress
                            preparationProgress
                        }
                        .barPanel()
                    }
                } else {
                    attachmentRows
                    uploadProgress
                    preparationProgress
                }
                HStack(alignment: .bottom, spacing: BottomBar.floats ? 8 : 6) {
                    AttachmentMenu(remaining: Wire.maxReplyAttachments - composer.attachments.count, identifier: "interject-attach", compact: true, preparationChanged: { id, active in
                        composer.setAttachmentPreparation(id, active: active)
                    }, progressChanged: {
                        preparation = $0
                    }) { name, mime, data, isImage in
                        try composer.addAttachment(name: name, mime: mime, data: data, isImage: isImage)
                    }
                    .disabled(!composer.canAttachMore || composer.isSending)
                    TextField("Message to the Agent", text: $composer.text, axis: .vertical)
                        .lineLimit(1...5)
                        .focused($focused)
                        .barFieldBox(focus: $focused)
                        .disabled(composer.isSending)
                        .accessibilityIdentifier("interject-field")
                    BarSendButton(label: "Send message", identifier: "interject-send", sending: composer.isSending, action: send)
                        .disabled(!composer.canSend)
                }
                if composer.trimmedText.count > InterjectComposer.maxChars - 1_000 {
                    Text("\(composer.trimmedText.count) / \(InterjectComposer.maxChars)")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(composer.overLimit ? Color.red : Color.secondary)
                        .barStatusChip(compact: true)
                        .frame(maxWidth: .infinity, alignment: .trailing)
                }
            }
        }
        .padding(.horizontal, BottomBar.floats ? 16 : 12)
        .padding(.vertical, 8)
        .classicBarBacking(hairline: false)
    }

    @ViewBuilder
    private var attachmentRows: some View {
        if !composer.attachments.isEmpty {
            VStack(spacing: 6) {
                ForEach(composer.attachments) { attachment in
                    ReplyAttachmentRow(attachment: attachment, url: composer.url(of: attachment)) {
                        composer.removeAttachment(id: attachment.id)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private var preparationProgress: some View {
        if let preparation { AttachmentPreparationView(preparation: preparation) }
    }

    @ViewBuilder
    private var uploadProgress: some View {
        if let progress = composer.uploadProgress {
            ProgressView(value: progress)
                .accessibilityLabel("Uploading attachments")
        }
    }

    private func send() {
        let composer = composer
        let item = item
        focused = false
        Task {
            do {
                let outcome = try await store.send(composer, to: item)
                onSent(outcome)
            } catch {
                onProblem(error.userMessage)
            }
        }
    }
}
