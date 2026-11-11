import SwiftUI
import Observation
import AHCore
import AHProtocol

struct RootView: View {
    @Environment(AppStore.self) private var store
    @Environment(PushCoordinator.self) private var push
    let leftoverPairings: Int
    @State private var presentation = TabPresentation()
    @State private var links = LinkPresenter()
    @State private var askAboutLeftovers: Bool
    @State private var arrivalFullscreenFixture = false
    @State private var showedArrivalFixture = false
    @Environment(\.launchOptions) private var launch
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    init(leftoverPairings: Int) {
        self.leftoverPairings = leftoverPairings
        _askAboutLeftovers = State(initialValue: leftoverPairings > 0)
    }

    private var tabSelection: Binding<AppTab> {
        Binding(get: { store.tab }, set: { tab in
            if store.tab == tab { presentation.listRevealRequests[tab, default: 0] += 1 }
            store.tab = tab
        })
    }

    @ViewBuilder
    var body: some View {
        @Bindable var presentation = presentation
        tabsWithBanner
        .environment(links)
        .background { rootBackground }
        .sheet(isPresented: $presentation.showPairing) { PairingSheet() }
        .fullScreenCover(isPresented: $arrivalFullscreenFixture) {
            // A mock-only layout fixture verifies the no-navigation-bar presentation boundary.
            VStack(spacing: 24) {
                Image(systemName: "photo").font(.system(size: 100)).foregroundStyle(.blue)
                    .accessibilityIdentifier("arrival-fullscreen-fixture")
                Button("Close") { arrivalFullscreenFixture = false }
                    .accessibilityIdentifier("arrival-fixture-close")
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Color(.systemGroupedBackground))
        }
        .onAppear {
            if launch.mock && launch.arrivalFullscreenFixture && !showedArrivalFixture {
                showedArrivalFixture = true
                arrivalFullscreenFixture = true
            }
            // A link nobody can open is said, not dropped. (Links are asked about and shown by `LinkPrompt`, from
            // whatever is on top; only the notice lives here, where the banner is.)
            links.onUnopened = { [store] url in
                store.show(Banner(kind: .warning, title: String(localized: "No app can open this link"), subtitle: url.absoluteString))
            }
        }
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
        // Keep the todo subscription in sync even when its hosted screen is offscreen.
        .onChange(of: store.tab, initial: true) { _, tab in store.todos.setVisible(tab == .todos) }
        .onChange(of: store.pendingCount) { _, count in push.updateBadge(count) }
        .onChange(of: store.state.pairing?.stage) { _, stage in
            // A pairing started from a notification-less state (a deep link in the future) opens the sheet.
            if stage != nil, !presentation.showPairing { presentation.showPairing = true }
        }
    }

    @ViewBuilder
    private var rootBackground: some View {
        ArrivalSceneHost(arrivals: store.requestArrivals, pendingCount: store.pendingCount,
                         reduceMotion: reduceMotion || (launch.mock && launch.arrivalReducedMotion),
                         diagnostics: launch.mock && launch.arrivalMotionDiagnostics,
                         slowdown: launch.mock ? launch.arrivalMotionSlowdown : 1,
                         compositionFixture: launch.mock && launch.arrivalMarkedTextFixture) { id in
            store.finishRequestArrival(id: id)
        }
        .frame(width: 0, height: 0)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
        // Remembers where the screen is touched, for the popover that asks before a link is followed.
        LastTouchAnchor()
            .frame(width: 0, height: 0)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
        DeveloperOverlayAnchor()
    }

    private var rootBanner: some View {
        VStack(spacing: 8) {
            SentPillHost()
            InAppBanner()
        }
    }

    @ViewBuilder
    private var tabsWithBanner: some View {
        if #available(iOS 27.1, *) {
            tabs
        } else {
            tabs.overlay(alignment: .top) { rootBanner }
        }
    }

    @ViewBuilder
    private var tabs: some View {
        if #available(iOS 27.1, *) {
            AppTabs(selection: tabSelection, pendingCount: store.pendingCount,
                    hidesTabBarWhenCompact: hidesTabBarWhenCompact, isOverlayInteractive: store.banner != nil) { tab in
                TabPage(tab: tab, presentation: presentation)
                    .environment(store)
                    .environment(push)
                    .environment(links)
                    .environment(\.launchOptions, launch)
            } overlay: {
                rootBanner.environment(store)
            }
            // UIKit lays out its bars outside the safe area; give the container the whole window.
            .ignoresSafeArea()
        } else {
            TabView(selection: tabSelection) {
                ForEach(AppTab.allCases, id: \.self) { tab in
                    TabPage(tab: tab, presentation: presentation)
                        .tabItem { Label(tab.title, systemImage: tab.symbol) }
                        .tag(tab)
                        .badge(tab == .inbox ? store.pendingCount : 0)
                }
            }
        }
    }

    private var hidesTabBarWhenCompact: Bool {
        switch store.tab {
        case .inbox: return !store.inboxPath.isEmpty
        case .agents: return !store.agentPath.isEmpty
        case .todos: return store.todos.opened != nil
        case .history, .macs: return false
        }
    }
}

/// Presentation state is shared across the independently hosted tab graphs.
@MainActor @Observable
private final class TabPresentation {
    var showPairing = false
    var listRevealRequests: [AppTab: Int] = [:]
}

private struct TabPage: View {
    let tab: AppTab
    let presentation: TabPresentation

    var body: some View {
        @Bindable var presentation = presentation
        Group {
            switch tab {
            case .inbox: InboxScreen(showPairing: $presentation.showPairing)
            case .agents: AgentsScreen(showPairing: $presentation.showPairing)
            case .todos: TodosScreen(showPairing: $presentation.showPairing)
            case .history: HistoryScreen()
            case .macs: MacsScreen(showPairing: $presentation.showPairing)
            }
        }
        .environment(\.sidebarRevealRequest, presentation.listRevealRequests[tab, default: 0])
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
                    .frame(maxWidth: Columns.bannerWidth)
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
            case "incomplete": "exclamationmark.triangle.fill"
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
            case "incomplete": .orange
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
        case "incomplete": return "Task incomplete"
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
