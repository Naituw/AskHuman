import SwiftUI
import AHCore
import AHProtocol

// The quick replies and one-shot instructions that go with an answer (docs/specs/special-replies.md D30): the menu beside
// "Send", and the panel above the bar that shows what is armed. A quick reply is sent at once; an instruction waits until
// the answer is sent, and the cross takes it back.

/// The glass button beside "Send" that opens the menu: the Mac's quick replies (the first one nearest the thumb, as the
/// menu opens upwards), then the two instructions.
struct DirectiveMenu: View {
    let composer: AnswerComposer
    let disabled: Bool
    /// The answer can be sent right now (the Mac is reachable, nothing is still being prepared): a quick reply sends it.
    let canSend: Bool
    let onQuickReply: (String) -> Void
    /// The circle is as tall as the row it is in, which is laid out by the answer bar's `ButtonRow` (floating bar only):
    /// the glyph's box takes all the room it is offered. Otherwise it is `BottomBar.rowHeight` across.
    var fillsRow = false

    var body: some View {
        Menu {
            let replies = composer.quickReplies
            if !replies.isEmpty {
                // Greyed out once every question has an answer: there is nothing left for a reply to answer.
                let usable = canSend && composer.quickReplyTargets > 0
                Section("Quick replies") {
                    ForEach(Array(replies.enumerated()), id: \.offset) { index, reply in
                        Button {
                            onQuickReply(reply)
                        } label: {
                            Text(verbatim: reply)
                        }
                        .disabled(!usable)
                        .accessibilityIdentifier("quick-reply-\(index)")
                    }
                }
            }
            Section {
                Button {
                    composer.directive = .away
                } label: {
                    Label("Away mode", systemImage: "moon")
                }
                .accessibilityIdentifier("directive-away")
                Button {
                    composer.directive = .mute
                } label: {
                    Label("Stop using AskHuman", systemImage: "bell.slash")
                }
                .accessibilityIdentifier("directive-mute")
            }
        } label: {
            let glyph = Image(systemName: "text.bubble")
            if BottomBar.floats {
                if fillsRow { glyph.barRowRoundGlyph() } else { glyph.barRoundGlyph() }
            } else {
                glyph.font(.title3).frame(width: 32, height: 32)
            }
        }
        .barRoundButton()
        .disabled(disabled)
        .accessibilityLabel("Quick replies")
        .accessibilityIdentifier("quick-reply-menu")
    }
}

/// What is armed, above the bar: the kind of instruction, how far it reaches, a note for the Agents, and a cross.
struct DirectivePanel: View {
    let composer: AnswerComposer
    @FocusState private var noteFocused: Bool

    var body: some View {
        if let directive = composer.directive {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 8) {
                    // The moon is away mode's colour; "Stop using AskHuman" is not away mode, and is a caution.
                    Image(systemName: directive.kind == .away ? "moon.fill" : "bell.slash.fill")
                        .foregroundStyle(directive.kind == .away ? AwayStyle.tint : Color.orange)
                    Text(directive.kind == .away ? "Away mode" : "Stop using AskHuman")
                        .font(.subheadline.weight(.semibold))
                    if directive.kind == .away {
                        scopeMenu(directive)
                    } else {
                        Text("This session only").font(.footnote).foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 0)
                    Button {
                        composer.directive = nil
                    } label: {
                        Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Take it back")
                    .accessibilityIdentifier("directive-cancel")
                }
                if directive.kind == .away {
                    TextField(
                        "", text: noteBinding, prompt: Text("Note for the Agents (optional)"), axis: .vertical
                    )
                    .lineLimit(1...3)
                    .font(.footnote)
                    .focused($noteFocused)
                    .padding(8)
                    .background(Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                    .accessibilityIdentifier("directive-note")
                } else {
                    Text("After you send, the Agent is told. Later calls to AskHuman get a hint at once.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .padding(10)
            .background {
                // The floating bar's panel is glass (`barPanel`); the classic bar's is a card.
                if !BottomBar.floats {
                    RoundedRectangle(cornerRadius: 16, style: .continuous).fill(Color(.secondarySystemGroupedBackground))
                }
            }
            .barPanel()
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("directive-panel")
        }
    }

    private var noteBinding: Binding<String> {
        Binding(
            get: { composer.directive?.note ?? "" },
            set: { text in composer.directive?.note = String(text.prefix(AwayNote.maxCharacters)) }
        )
    }

    private func scopeMenu(_ directive: AnswerDirectiveChoice) -> some View {
        Menu {
            Picker("Scope", selection: Binding(
                get: { composer.directive?.sessionOnly ?? false },
                set: { composer.directive?.sessionOnly = $0 }
            )) {
                Text("All Agents").tag(false)
                Text("This session only").tag(true)
            }
        } label: {
            HStack(spacing: 3) {
                Text(directive.sessionOnly ? "This session only" : "All Agents")
                Image(systemName: "chevron.up.chevron.down").imageScale(.small)
            }
            .font(.footnote)
            .foregroundStyle(.secondary)
        }
        .accessibilityIdentifier("directive-scope")
    }
}

/// The card at the top of a question that came as an escalation (D36): the Agent asked although the user is away. The
/// page does not show the root list's away strip; this card has its own way to end away mode.
struct EscalationNotice: View {
    let item: InboxItem
    @Environment(AppStore.self) private var store
    @State private var ending = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label {
                Text("Escalated while you are away").font(.subheadline.weight(.semibold))
            } icon: {
                Image(systemName: "moon.fill").foregroundStyle(AwayStyle.tint)
            }
            Text(item.view?.ask?.whatsNext == true
                ? "The Agent says everything is done."
                : "The Agent says it cannot go on without you.")
                .font(.footnote)
                .foregroundStyle(.secondary)
            if store.away.isGlobal {
                Button {
                    ending = true
                    Task {
                        await store.endAway()
                        ending = false
                    }
                } label: {
                    Text("End away mode").font(.footnote.weight(.semibold))
                }
                .disabled(ending)
                .accessibilityIdentifier("escalation-end-away")
            }
        }
        .card()
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("escalation-notice")
    }
}

/// "AskHuman in this session" (D35): what this one session does on its own, in the header card of its page. It
/// applies at once, unlike the instruction in the answer bar, which waits for the answer to be sent.
struct SessionAwayRow: View {
    let item: AgentItem
    @Environment(AppStore.self) private var store
    @State private var problem: String?

    var body: some View {
        if store.away.macs.first(where: { $0.id == item.macId })?.supported == true, !item.isEnded {
            let globalOn = store.isGlobalAway(macId: item.macId)
            let mode = store.sessionMode(of: item)
            let reachable = store.canSetSessionAway(item)
            VStack(alignment: .leading, spacing: 8) {
                Divider()
                HStack(spacing: 8) {
                    Image(systemName: "moon").foregroundStyle(AwayStyle.tint)
                    Text("AskHuman in this session").font(.subheadline)
                    Spacer(minLength: 8)
                    Menu {
                        Picker("AskHuman in this session", selection: Binding(
                            get: { AwayTexts.current(mode, globalOn: globalOn) },
                            set: { setting in apply(setting) }
                        )) {
                            ForEach(AwayTexts.sessionChoices(globalOn: globalOn), id: \.setting) { choice in
                                Text(choice.title).tag(choice.setting)
                            }
                        }
                    } label: {
                        HStack(spacing: 4) {
                            Text(AwayTexts.sessionStatus(mode, globalOn: globalOn))
                            Image(systemName: "chevron.up.chevron.down").imageScale(.small)
                        }
                        .font(.subheadline)
                        .foregroundStyle(statusTint(mode, globalOn: globalOn))
                    }
                    .disabled(!reachable)
                    .accessibilityIdentifier("session-away-menu")
                }
                if let problem {
                    Text(verbatim: problem).font(.footnote).foregroundStyle(.red)
                        .accessibilityIdentifier("session-away-problem")
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("session-away")
        }
    }

    private func statusTint(_ mode: AwaySessionMode, globalOn: Bool) -> Color {
        switch mode {
        case .away: AwayStyle.tint
        case .follow: globalOn ? AwayStyle.tint : .secondary
        case .mute, .normal: .secondary
        }
    }

    private func apply(_ setting: AwaySessionSetting) {
        problem = nil
        Task {
            do {
                try await store.setSessionAway(setting, for: item)
            } catch {
                problem = error.userMessage
            }
        }
    }
}
