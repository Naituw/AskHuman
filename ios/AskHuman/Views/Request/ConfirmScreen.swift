import SwiftUI
import AHCore
import AHProtocol

/// A confirmation (permission prompt, stop hook): title, context, detail and the buttons the Mac offers.
struct ConfirmScreen: View {
    let item: InboxItem
    @Environment(AppStore.self) private var store
    @State private var confirmDismiss = false
    @State private var problem: String?

    var body: some View {
        if let confirm = item.view?.confirm, let composer = store.confirmComposer(for: item) {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if let finished = store.endedNotice(for: item.id) { FinishedNotice(finished: finished) }
                    if case .failed = item.state { FailedSubmissionNotice(item: item) }
                    RequestHeader(item: item)
                    DetailCard(confirm: confirm)
                    VStack(spacing: 10) {
                        ForEach(confirm.choices, id: \.wireIndex) { choice in
                            ChoiceRow(choice: choice, selected: composer.selectedWireIndex == choice.wireIndex) {
                                composer.select(wireIndex: choice.wireIndex)
                            }
                            .accessibilityIdentifier("choice-\(choice.wireIndex)")
                        }
                    }
                    if let field = composer.visibleInput {
                        InputCard(field: field, composer: composer)
                    }
                    ExpiryLabel(date: composer.expiresAt)
                }
                .padding()
            }
            .background(Color(.systemGroupedBackground))
            .scrollDismissesKeyboard(.interactively)
            .safeAreaInset(edge: .bottom, spacing: 0) {
                bar(composer: composer, confirm: confirm)
            }
            .toolbar {
                if composer.canDismiss {
                    ToolbarItem(placement: .topBarTrailing) {
                        Menu {
                            Button("Dismiss", systemImage: "xmark.circle", role: .destructive) { confirmDismiss = true }
                                .disabled(store.finished[item.id] != nil || !store.isOnline(macId: item.macId))
                        } label: {
                            Image(systemName: "ellipsis.circle")
                        }
                    }
                }
            }
            .confirmationDialog("Dismiss this confirmation?", isPresented: $confirmDismiss, titleVisibility: .visible) {
                Button("Dismiss", role: .destructive) {
                    Task {
                        do { try await store.dismiss(composer, item: item) } catch { problem = error.userMessage }
                    }
                }
                Button("Keep it", role: .cancel) {}
            } message: {
                Text("The Agent receives the default response, usually a refusal.")
            }
            .alert("Could not send", isPresented: Binding(get: { problem != nil }, set: { if !$0 { problem = nil } })) {
                Button("OK", role: .cancel) {}
            } message: {
                Text(problem ?? "")
            }
        }
    }

    private func bar(composer: ConfirmComposer, confirm: ConfirmView) -> some View {
        let sending: Bool = { if case .sending = item.state { return true } else { return false } }()
        let ended = store.endedNotice(for: item.id) != nil
        let online = store.isOnline(macId: item.macId)
        return VStack(spacing: 6) {
            if ended {
                Text("This request has ended.").font(.caption).foregroundStyle(.secondary)
            } else if !online {
                Text("The Mac is offline. Try again when it is back.").font(.caption).foregroundStyle(.orange)
            } else if sending {
                Text("Waiting for your Mac to confirm…").font(.caption).foregroundStyle(.secondary)
            }
            Button {
                Task {
                    do {
                        try await store.decide(composer, item: item)
                    } catch {
                        problem = error.userMessage
                    }
                }
            } label: {
                HStack(spacing: 6) {
                    if composer.isSending || sending { ProgressView().controlSize(.small).tint(.white) }
                    Text(confirm.submitLabel.isEmpty ? String(localized: "Submit") : confirm.submitLabel).font(.headline)
                }
                .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .disabled(!composer.canSubmit || ended || !online || sending)
            .accessibilityIdentifier("submit-button")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.bar)
        .overlay(alignment: .top) { Divider() }
    }
}

private struct DetailCard: View {
    let confirm: ConfirmView

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(confirm.title).font(.title3.weight(.semibold))
            if !confirm.context.isEmpty {
                Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 12, verticalSpacing: 6) {
                    ForEach(confirm.context, id: \.id) { field in
                        GridRow {
                            Text(field.label).font(.footnote).foregroundStyle(.secondary).gridColumnAlignment(.trailing)
                            value(of: field)
                        }
                    }
                }
            }
            let body = confirm.detail.bodyMd.trimmingCharacters(in: .whitespacesAndNewlines)
            if !body.isEmpty {
                Divider()
                MarkdownView(text: body)
            } else if !confirm.detail.summary.isEmpty {
                Divider()
                Text(verbatim: confirm.detail.summary).font(.body).textSelection(.enabled)
            }
        }
        .card()
    }

    @ViewBuilder
    private func value(of field: ConfirmFieldView) -> some View {
        switch field.kind {
        case "path":
            Text(verbatim: field.value)
                .font(.system(.footnote, design: .monospaced))
                .lineLimit(2)
                .truncationMode(.middle)
                .textSelection(.enabled)
        case "timestamp":
            if let ms = Double(field.value) {
                Text(Date(timeIntervalSince1970: ms / 1000), format: .dateTime.month().day().hour().minute().second())
                    .font(.footnote)
            } else {
                Text(verbatim: field.value).font(.footnote)
            }
        default:
            Text(verbatim: field.value).font(.footnote).textSelection(.enabled)
        }
    }
}

private struct ChoiceRow: View {
    let choice: ConfirmChoiceView
    let selected: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: selected ? "largecircle.fill.circle" : "circle")
                    .font(.title3)
                    .foregroundStyle(selected ? tint : Color(.tertiaryLabel))
                VStack(alignment: .leading, spacing: 3) {
                    Text(verbatim: choice.label).font(.body.weight(.semibold)).foregroundStyle(.primary)
                    if !choice.description.isEmpty {
                        Text(verbatim: choice.description).font(.footnote).foregroundStyle(.secondary)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                if choice.recommended {
                    Chip(text: "Recommended", systemImage: "star.fill", tint: .orange)
                }
            }
            .padding(14)
            .background(RoundedRectangle(cornerRadius: 14, style: .continuous).fill(selected ? tint.opacity(0.12) : Color(.secondarySystemGroupedBackground)))
            .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).stroke(selected ? tint : .clear, lineWidth: 1.5))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    private var tint: Color {
        switch choice.role {
        case "destructive", "danger": .red
        case "primary": .accentColor
        default: .gray
        }
    }
}

private struct InputCard: View {
    let field: ConfirmInputView
    let composer: ConfirmComposer

    var body: some View {
        @Bindable var composer = composer
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(verbatim: field.label).font(.subheadline.weight(.semibold))
                if field.required { Text("Required").font(.caption).foregroundStyle(.red) }
                Spacer()
                Text("\(composer.input.count)/\(field.maxChars)").font(.caption).foregroundStyle(.tertiary)
            }
            TextField(field.placeholder, text: $composer.input, axis: .vertical)
                .lineLimit(1...8)
                .padding(10)
                .background(Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .onChange(of: composer.input) { _, value in
                    if value.count > field.maxChars { composer.input = String(value.prefix(field.maxChars)) }
                }
                .accessibilityIdentifier("confirm-input")
        }
        .card()
    }
}

private struct ExpiryLabel: View {
    let date: Date

    var body: some View {
        let remaining = date.timeIntervalSinceNow
        if remaining > 0 {
            HStack(spacing: 6) {
                Image(systemName: "timer")
                if remaining < 3600 {
                    Text("Expires in \(Text(timerInterval: Date.now...date, countsDown: true))")
                } else {
                    Text("Expires in \(Text(date, style: .relative))")
                }
            }
            .font(.footnote)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .center)
        }
    }
}
