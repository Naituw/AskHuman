import SwiftUI
import AHCore
import AHProtocol
import AHStore

/// A finished request as it was: what was asked and what this phone answered.
struct HistoryDetailScreen: View {
    let entry: HistoryEntry

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                header
                if let ask = entry.request.ask {
                    askContent(ask)
                } else if let confirm = entry.request.confirm {
                    confirmContent(confirm)
                }
            }
            .padding()
        }
        .background(Color(.systemGroupedBackground))
        .navigationTitle(entry.request.agent.label)
        .navigationBarTitleDisplayMode(.inline)
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 12) {
                AgentBadge(agent: entry.request.agent, size: 44)
                VStack(alignment: .leading, spacing: 2) {
                    if let title = entry.sessionTitle {
                        Text(verbatim: title).font(.headline).lineLimit(2).accessibilityIdentifier("history-title")
                        Text(verbatim: [entry.request.agent.label, entry.request.project.name, entry.macName].filter { !$0.isEmpty }.joined(separator: " · "))
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    } else {
                        Text(entry.request.agent.label).font(.headline)
                        HStack(spacing: 5) {
                            if !entry.request.project.name.isEmpty {
                                Image(systemName: "folder.fill").imageScale(.small)
                                Text(entry.request.project.name)
                                Text("·")
                            }
                            Image(systemName: "desktopcomputer").imageScale(.small)
                            Text(entry.macName)
                        }
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    }
                }
            }
            Label(entry.outcome.title(by: entry.by), systemImage: entry.outcome.symbol)
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(entry.outcome.color)
            Text("Asked \(entry.receivedAt.formatted(.dateTime.month().day().hour().minute())) · Finished \(entry.finishedAt.formatted(.dateTime.month().day().hour().minute()))")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .card()
    }

    @ViewBuilder
    private func askContent(_ ask: AskView) -> some View {
        let message = ask.message.text.trimmingCharacters(in: .whitespacesAndNewlines)
        if !message.isEmpty || !ask.message.files.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                if !message.isEmpty {
                    if ask.isMarkdown { WebMarkdownView(text: message) } else { Text(verbatim: message).textSelection(.enabled) }
                }
                ForEach(ask.message.files, id: \.assetId) { file in FileRow(asset: file).allowsHitTesting(false) }
                if !ask.message.files.isEmpty {
                    Text("Only the file names are kept on this phone.").font(.caption).foregroundStyle(.secondary)
                }
            }
            .card()
        }
        ForEach(Array(ask.questions.enumerated()), id: \.offset) { index, question in
            let answer = entry.answers.first { $0.q == index }
            VStack(alignment: .leading, spacing: 10) {
                if ask.questions.count > 1 { Text("Question \(index + 1)").font(.caption.weight(.semibold)).foregroundStyle(.secondary) }
                if ask.isMarkdown { MarkdownView(text: question.text, baseFont: .body.weight(.semibold)) } else { Text(verbatim: question.text).font(.body.weight(.semibold)) }
                ForEach(Array(question.options.enumerated()), id: \.offset) { optionIndex, option in
                    let chosen = answer?.sel.contains(optionIndex) == true
                    HStack(spacing: 8) {
                        Image(systemName: chosen ? "checkmark.circle.fill" : "circle")
                            .foregroundStyle(chosen ? Color.accentColor : Color(.tertiaryLabel))
                        Text(verbatim: option.text).fontWeight(chosen ? .semibold : .regular).foregroundStyle(chosen ? .primary : .secondary)
                    }
                }
                if let text = answer?.text, !text.isEmpty {
                    Divider()
                    Text(verbatim: text).font(.body).textSelection(.enabled)
                }
                if let answer, !answer.images.isEmpty || !answer.files.isEmpty {
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Image(systemName: "paperclip")
                        VStack(alignment: .leading, spacing: 2) {
                            if !answer.images.isEmpty { Text("\(answer.images.count) images attached") }
                            if !answer.files.isEmpty { Text("\(answer.files.count) files attached") }
                        }
                    }
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                }
                if answer == nil, entry.outcome == .answered {
                    Text("Not answered").font(.footnote).foregroundStyle(.tertiary)
                }
            }
            .card()
        }
    }

    @ViewBuilder
    private func confirmContent(_ confirm: ConfirmView) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(confirm.title).font(.title3.weight(.semibold))
            let body = confirm.detail.bodyMd.trimmingCharacters(in: .whitespacesAndNewlines)
            if !body.isEmpty { WebMarkdownView(text: body) } else if !confirm.detail.summary.isEmpty { Text(verbatim: confirm.detail.summary).textSelection(.enabled) }
        }
        .card()
        if let choice = entry.confirmChoice {
            VStack(alignment: .leading, spacing: 6) {
                Text("Your response").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                Text(verbatim: choice).font(.body.weight(.semibold))
                if let input = entry.confirmInput, !input.isEmpty { Text(verbatim: input).font(.body).foregroundStyle(.secondary) }
            }
            .card()
        }
    }
}
