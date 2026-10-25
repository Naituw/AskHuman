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

    var body: some View {
        @Bindable var composer = composer
        VStack(spacing: 8) {
            if !composer.attachments.isEmpty {
                VStack(spacing: 6) {
                    ForEach(composer.attachments) { attachment in
                        ReplyAttachmentRow(attachment: attachment, url: composer.url(of: attachment)) {
                            composer.removeAttachment(id: attachment.id)
                        }
                    }
                }
            }
            if let progress = composer.uploadProgress {
                ProgressView(value: progress)
                    .accessibilityLabel("Uploading attachments")
            }
            HStack(alignment: .bottom, spacing: 6) {
                AttachmentMenu(remaining: Wire.maxReplyAttachments - composer.attachments.count, identifier: "interject-attach", compact: true) { name, mime, data, isImage in
                    try composer.addAttachment(name: name, mime: mime, data: data, isImage: isImage)
                }
                .disabled(!composer.canAttachMore || composer.isSending)
                TextField("Message to the Agent", text: $composer.text, axis: .vertical)
                    .lineLimit(1...5)
                    .focused($focused)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                    .disabled(composer.isSending)
                    .accessibilityIdentifier("interject-field")
                Button(action: send) {
                    Group {
                        if composer.isSending {
                            ProgressView()
                        } else {
                            Image(systemName: "arrow.up.circle.fill").font(.system(size: 32))
                        }
                    }
                    .frame(width: 34, height: 34)
                }
                .disabled(!composer.canSend)
                .accessibilityLabel("Send message")
                .accessibilityIdentifier("interject-send")
            }
            if composer.trimmedText.count > InterjectComposer.maxChars - 1_000 {
                Text("\(composer.trimmedText.count) / \(InterjectComposer.maxChars)")
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(composer.overLimit ? Color.red : Color.secondary)
                    .frame(maxWidth: .infinity, alignment: .trailing)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(.bar)
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
