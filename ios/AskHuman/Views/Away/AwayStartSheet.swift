import SwiftUI
import AHCore

/// The moon button in the bar of the Agents tab (docs/specs/special-replies.md D32): hollow while away mode is off, a
/// solid orange one while it is on. Off, it opens the sheet that turns it on; on, it ends it.
struct AwayToolbarButton: View {
    @Binding var showSheet: Bool
    @Environment(AppStore.self) private var store
    @State private var ending = false
    @State private var problem = false

    var body: some View {
        let on = store.away.isGlobal
        let label: LocalizedStringKey = on ? "End away mode" : "Away mode"
        let value: LocalizedStringKey = on ? "On" : "Off"
        Group {
            if on {
                Button(action: tap) { Image(systemName: "moon.fill") }
                    .buttonStyle(.borderedProminent)
                    .tint(.orange)
            } else {
                Button(action: tap) { Image(systemName: "moon") }
            }
        }
        .disabled(ending)
        .accessibilityLabel(label)
        .accessibilityIdentifier("away-moon")
        .accessibilityValue(value)
        .alert("Could not end away mode", isPresented: $problem) {
            Button("OK", role: .cancel) {}
        } message: {
            Text("A Mac did not answer. Away mode is still on there.")
        }
    }

    private func tap() {
        if store.away.isGlobal {
            ending = true
            Task {
                let failed = await store.endAway()
                ending = false
                if !failed.isEmpty { problem = true }
            }
        } else {
            showSheet = true
        }
    }
}

/// The half-height sheet that turns away mode on (D33, D34): what it does, which Macs it goes to, and a note the
/// Agents see with every automatic reply.
struct AwayStartSheet: View {
    @Environment(AppStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var picked: Set<String> = []
    @State private var seeded = false
    @State private var note = ""
    @State private var sending = false
    @State private var problem: String?
    @FocusState private var noteFocused: Bool

    var body: some View {
        let away = store.away
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    Text("Once it is on, an Agent's questions get an automatic reply: you are away, so it should go on by its own judgement. Only when it cannot go on does it ask you again, and then your phone and Mac let you know.")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 4)
                    // One Mac needs no choice.
                    if away.macs.count > 1 { macs(away) }
                    if away.startable.isEmpty {
                        Label("No Mac is online with away mode right now.", systemImage: "desktopcomputer.trianglebadge.exclamationmark")
                            .font(.footnote)
                            .foregroundStyle(.orange)
                            .accessibilityIdentifier("away-none")
                    }
                    noteCard
                    if let problem {
                        Label(problem, systemImage: "exclamationmark.triangle")
                            .font(.footnote)
                            .foregroundStyle(.orange)
                            .accessibilityIdentifier("away-problem")
                    }
                }
                .readableWidth()
                .padding()
            }
            .background(Color(.systemGroupedBackground))
            .scrollDismissesKeyboard(.interactively)
            .navigationTitle("Away mode")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button { dismiss() } label: { Image(systemName: "xmark") }
                        .accessibilityLabel("Close")
                        .accessibilityIdentifier("away-close")
                }
            }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                Button(action: start) {
                    Group {
                        if sending { ProgressView() } else { Text("Turn on away mode") }
                    }
                    .font(.headline)
                    .frame(maxWidth: .infinity)
                }
                .barPrimaryButton()
                .disabled(picked.isEmpty || sending)
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
                .accessibilityIdentifier("away-start")
            }
        }
        .presentationDetents([.large])
        .onAppear { seed(away) }
        .onChange(of: away) { _, next in
            // A Mac that comes online while the sheet is open becomes pickable; the choice made so far stays.
            if !seeded { seed(next) }
        }
    }

    /// Everything that can be set is picked, until the user changes it.
    private func seed(_ away: AwayOverview) {
        guard !seeded, !away.macs.isEmpty else { return }
        seeded = true
        picked = Set(away.startable.map(\.id))
    }

    private func macs(_ away: AwayOverview) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Macs it applies to")
                .font(.subheadline.weight(.semibold))
                .padding(.bottom, 6)
            ForEach(Array(away.macs.enumerated()), id: \.element.id) { index, mac in
                if index > 0 { Divider() }
                macRow(mac)
            }
        }
        .card()
    }

    private func macRow(_ mac: AwayOverview.Entry) -> some View {
        let reason = AwayTexts.unavailable(mac)
        let on = picked.contains(mac.id)
        return Button {
            if on { picked.remove(mac.id) } else { picked.insert(mac.id) }
        } label: {
            HStack(spacing: 12) {
                Image(systemName: on ? "checkmark.circle.fill" : "circle")
                    .font(.title3)
                    .foregroundStyle(on ? Color.accentColor : Color.secondary.opacity(0.5))
                VStack(alignment: .leading, spacing: 1) {
                    Text(verbatim: mac.name).foregroundStyle(reason == nil ? .primary : .secondary)
                    if let reason {
                        Text(reason).font(.caption).foregroundStyle(.secondary)
                    } else {
                        Text("Online").font(.caption).foregroundStyle(.secondary)
                    }
                }
                Spacer(minLength: 0)
            }
            .padding(.vertical, 8)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(reason != nil)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(on ? .isSelected : [])
        .accessibilityIdentifier("away-mac-\(mac.id)")
    }

    private var noteCard: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Note (optional)")
                .font(.subheadline.weight(.semibold))
            TextField("", text: $note, prompt: Text("Go with your best judgement; ask again if you are stuck"), axis: .vertical)
                .lineLimit(2...5)
                .focused($noteFocused)
                .padding(10)
                .background(Color(.tertiarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .onChange(of: note) { _, text in
                    if text.count > AwayNote.maxCharacters { note = String(text.prefix(AwayNote.maxCharacters)) }
                }
                .accessibilityIdentifier("away-note")
            Text("Every automatic reply carries this sentence while away mode is on.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .card()
    }

    private func start() {
        noteFocused = false
        sending = true
        problem = nil
        let ids = store.away.macs.map(\.id).filter { picked.contains($0) }
        Task {
            let failed = await store.startAway(macIds: ids, note: note)
            sending = false
            if failed.isEmpty {
                dismiss()
            } else if failed.count < ids.count {
                // Some are on: the strip shows, and the user is told which Mac did not take it.
                let names = store.away.macs.filter { failed.contains($0.id) }.map(\.name).joined(separator: ", ")
                problem = String(localized: "Not turned on for \(names): it did not answer.")
                picked.subtract(ids.filter { !failed.contains($0) })
            } else {
                problem = String(localized: "The Mac did not answer. Try again when it is online.")
            }
        }
    }
}
