import SwiftUI
import AHCore

// The strip above the tab bar while away mode is on (docs/specs/special-replies.md D31): orange, so that it is not
// taken for a toolbar and forgotten. From iOS 26 it is tinted Liquid Glass; before, a material under orange. The
// place is the same either way: the app draws it itself instead of the system's bottom accessory, which only
// exists on iOS 26.

extension View {
    /// Shows the away strip at the foot of a tab's root page, above the tab bar. A page pushed over the root has no
    /// tab bar and no strip (the escalated question carries its own way to end away mode).
    func awayStrip() -> some View {
        safeAreaInset(edge: .bottom, spacing: 0) { AwayStrip() }
    }
}

struct AwayStrip: View {
    @Environment(AppStore.self) private var store
    @State private var ending = false
    @State private var problem = false

    var body: some View {
        let away = store.away
        ZStack {
            if away.isGlobal {
                TimelineView(.periodic(from: .now, by: 30)) { context in
                    content(away: away, now: context.date)
                }
                .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(.snappy, value: away.isGlobal)
        .alert("Could not end away mode", isPresented: $problem) {
            Button("OK", role: .cancel) {}
        } message: {
            Text("A Mac did not answer. Away mode is still on there.")
        }
    }

    private func content(away: AwayOverview, now: Date) -> some View {
        HStack(spacing: 12) {
            Image(systemName: "moon.fill")
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: 34, height: 34)
                .background(.white.opacity(0.22), in: Circle())
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 1) {
                Text("Away mode is on")
                    .font(.subheadline.weight(.semibold))
                    .lineLimit(1)
                let detail = AwayTexts.stripDetail(away, now: now)
                if !detail.isEmpty {
                    Text(verbatim: detail)
                        .font(.caption)
                        .opacity(0.85)
                        .lineLimit(1)
                }
            }
            .foregroundStyle(.white)
            Spacer(minLength: 8)
            Button {
                end()
            } label: {
                Group {
                    if ending { ProgressView().tint(.orange) } else { Text("End") }
                }
                .font(.subheadline.weight(.bold))
                .foregroundStyle(Color.orange)
                .frame(minWidth: 40)
                .padding(.horizontal, 14)
                .padding(.vertical, 7)
                .background(.white, in: Capsule())
            }
            .buttonStyle(.plain)
            .disabled(ending)
            .accessibilityIdentifier("away-end")
        }
        .padding(.leading, 10)
        .padding(.trailing, 10)
        .padding(.vertical, 8)
        .modifier(AwayStripBackground())
        .padding(.horizontal, 16)
        .padding(.bottom, 6)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("away-strip")
    }

    private func end() {
        ending = true
        Task {
            let failed = await store.endAway()
            ending = false
            if !failed.isEmpty { problem = true }
        }
    }
}

/// The strip's body: tinted glass from iOS 26, a material under orange before.
private struct AwayStripBackground: ViewModifier {
    func body(content: Content) -> some View {
        if #available(iOS 26, *) {
            content.glassEffect(.regular.tint(.orange), in: .capsule)
        } else {
            content
                .background(Color.orange.opacity(0.94), in: Capsule())
                .background(.ultraThinMaterial, in: Capsule())
                .shadow(color: .orange.opacity(0.35), radius: 10, y: 3)
        }
    }
}
