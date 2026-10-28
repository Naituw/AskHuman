import SwiftUI
import AHCore
import AHProtocol

struct RootView: View {
    @Environment(AppStore.self) private var store
    @Environment(PushCoordinator.self) private var push
    let leftoverPairings: Int
    @State private var showPairing = false
    @State private var links = LinkPresenter()
    @State private var askAboutLeftovers: Bool
    @Environment(\.launchOptions) private var launch

    init(leftoverPairings: Int) {
        self.leftoverPairings = leftoverPairings
        _askAboutLeftovers = State(initialValue: leftoverPairings > 0)
    }

    var body: some View {
        @Bindable var store = store
        TabView(selection: $store.tab) {
            InboxScreen(showPairing: $showPairing)
                .tabItem { Label("Inbox", systemImage: "tray.full") }
                .tag(AppTab.inbox)
                .badge(store.pendingCount)
            AgentsScreen(showPairing: $showPairing)
                .tabItem { Label("Agents", systemImage: "person.2.wave.2") }
                .tag(AppTab.agents)
            TodosScreen(showPairing: $showPairing)
                .tabItem { Label("Todos", systemImage: "checklist") }
                .tag(AppTab.todos)
            HistoryScreen()
                .tabItem { Label("History", systemImage: "clock.arrow.circlepath") }
                .tag(AppTab.history)
            MacsScreen(showPairing: $showPairing)
                .tabItem { Label("Macs", systemImage: "desktopcomputer") }
                .tag(AppTab.macs)
        }
        .environment(links)
        .overlay(alignment: .top) {
            VStack(spacing: 8) {
                SentPillHost()
                InAppBanner()
            }
        }
        .sheet(isPresented: $showPairing) { PairingSheet() }
        .confirmationDialog(
            "Open this link?", isPresented: Binding(get: { links.pending != nil }, set: { if !$0 { links.pending = nil } }),
            titleVisibility: .visible, presenting: links.pending
        ) { url in
            Button("Open \(links.pendingDomain)") { links.presented = url }
            Button("Cancel", role: .cancel) {}
        } message: { url in
            Text(url.absoluteString)
        }
        .sheet(item: $links.presented) { url in SafariView(url: url).ignoresSafeArea() }
        .alert("Pairings from a previous install", isPresented: $askAboutLeftovers) {
            Button("Keep them") {}
            Button("Remove them", role: .destructive) { Task { await store.forgetAll() } }
        } message: {
            Text("This phone still holds \(leftoverPairings) pairing(s) from before AskHuman was reinstalled. Keep using them, or start fresh?")
        }
        .sensoryFeedback(.impact(weight: .light), trigger: store.arrivalTick)
        // The Mac took our reply: a success for an answer, a lighter tap for a cancellation.
        .sensoryFeedback(trigger: store.sentTick) { _, _ in
            store.receipt?.outcome == .answered ? .success : .impact(weight: .light)
        }
        .onChange(of: store.pendingCount) { _, count in push.updateBadge(count) }
        .onChange(of: store.state.pairing?.stage) { _, stage in
            // A pairing started from a notification-less state (a deep link in the future) opens the sheet.
            if stage != nil, !showPairing { showPairing = true }
        }
    }
}

/// The pill that drops in over the request the app moved on to, after one was sent.
struct SentPillHost: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        ZStack(alignment: .top) {
            if let note = store.sentNote {
                SentPill(note: note)
                    .padding(.top, 4)
                    .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
        .animation(.spring(duration: 0.45, bounce: 0.3), value: store.sentNote)
    }
}

private struct SentPill: View {
    let note: SentNote

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
            Text("Sent · \(note.remaining) left").font(.subheadline.weight(.semibold))
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.regularMaterial, in: Capsule())
        .shadow(color: .black.opacity(0.15), radius: 10, y: 3)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("sent-pill")
        .allowsHitTesting(false)
    }
}

/// The banner that slides in when a request arrives while the app is open.
struct InAppBanner: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        ZStack(alignment: .top) {
            if let banner = store.banner {
                BannerCard(banner: banner)
                    .padding(.horizontal, 12)
                    .padding(.top, 4)
                    .transition(.move(edge: .top).combined(with: .opacity))
                    .onTapGesture {
                        switch banner.kind {
                        case .request(let macId, let requestId): store.open(macId: macId, requestId: requestId)
                        case .agent(let macId, let agentKey, _): store.openAgent(macId: macId, agentKey: agentKey)
                        case .info, .warning: break
                        }
                        store.dismissBanner()
                    }
                    .gesture(DragGesture(minimumDistance: 8).onEnded { value in
                        if value.translation.height < -8 { store.dismissBanner() }
                    })
            }
        }
        .animation(.spring(duration: 0.35), value: store.banner)
    }
}

private struct BannerCard: View {
    let banner: Banner

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: symbol)
                .font(.title3)
                .foregroundStyle(tint)
            VStack(alignment: .leading, spacing: 2) {
                if let label = eventLabel {
                    Text(label).font(.caption.weight(.semibold)).foregroundStyle(tint)
                }
                titleText.font(.subheadline.weight(.semibold)).lineLimit(1)
                if let subtitle, !subtitle.isEmpty {
                    Text(subtitle).font(.footnote).foregroundStyle(.secondary).lineLimit(2)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(12)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .shadow(color: .black.opacity(0.15), radius: 12, y: 4)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("banner")
    }

    private var symbol: String {
        switch banner.kind {
        case .request: "bell.badge.fill"
        case .agent(_, _, let event):
            switch event {
            case "failed": "xmark.octagon.fill"
            case "waiting": "questionmark.bubble.fill"
            default: "checkmark.circle.fill"
            }
        case .info: "info.circle.fill"
        case .warning: "exclamationmark.triangle.fill"
        }
    }

    private var tint: Color {
        switch banner.kind {
        case .request: .accentColor
        case .agent(_, _, let event):
            switch event {
            case "failed": .red
            case "waiting": .orange
            default: .green
            }
        case .info: .blue
        case .warning: .orange
        }
    }

    /// What happened to a followed Agent.
    private var eventLabel: LocalizedStringKey? {
        guard case .agent(_, _, let event) = banner.kind else { return nil }
        switch event {
        case "failed": return "Agent failed"
        case "waiting": return "Agent needs you"
        default: return "Agent finished"
        }
    }

    private var titleText: Text {
        Text(banner.title)
    }

    private var subtitle: String? {
        banner.subtitle
    }
}
