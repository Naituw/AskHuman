import SwiftUI
import AHCore
import AHProtocol

struct InboxScreen: View {
    @Environment(AppStore.self) private var store
    @Binding var showPairing: Bool

    var body: some View {
        // A wide window shows the list beside the request that was picked, and shows them from the first
        // frame on: the page of a tab that changes from a stack to columns (the Macs have answered, or the
        // first one was paired) is where an iPad's bar at the top goes wrong.
        ColumnsOrStack {
            InboxColumns(showPairing: $showPairing)
        } stack: {
            stack
        }
    }

    /// The phone's way: the list, and the request pushed over it.
    @ViewBuilder
    private var stack: some View {
        @Bindable var store = store
        NavigationStack(path: $store.inboxPath) {
            Group {
                if store.hasPairedMacs {
                    InboxList().awayStrip()
                } else {
                    OnboardingView(showPairing: $showPairing)
                }
            }
            .navigationTitle("Inbox")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                // The filter lives in the bar, not in a strip under it: on iOS 26 the bar's edge
                // effect blurs whatever is pinned there, which made the header tall and murky.
                if store.hasPairedMacs {
                    ToolbarItem(placement: .principal) { SyncTitle(title: "Inbox") }
                    ToolbarItem(placement: .topBarLeading) { InboxFilterMenu() }
                }
            }
            .navigationDestination(for: String.self) { id in
                RequestScreen(id: id)
            }
        }
    }
}

/// A wide window: the list on the left, and on the right the request that was picked from it.
///
/// What the list has selected is what the inbox has open (`inboxPath` holds that one request, as it
/// does on the phone), so a request opened from a notification, a banner or the arrows is the one the
/// list shows as picked, and going back to the list is an empty selection.
private struct InboxColumns: View {
    @Environment(AppStore.self) private var store
    @Binding var showPairing: Bool

    var body: some View {
        let open = store.inboxPath.last
        // Before the Macs have answered nobody knows whether one is paired: the list stays until they
        // say so, as it does for everyone who has one.
        let hasList = store.hasPairedMacs || !store.hasState
        ListDetailColumns(hasList: hasList, opened: open) {
            InboxList(selection: selection)
                .awayStrip()
                .navigationTitle("Inbox")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .principal) { SyncTitle(title: "Inbox") }
                    ToolbarItem(placement: .topBarLeading) { InboxFilterMenu() }
                }
        } detail: {
            if !hasList {
                OnboardingView(showPairing: $showPairing)
            } else if let open {
                RequestScreen(id: open)
            } else {
                DetailPlaceholder(
                    title: "Select a request", symbol: "tray",
                    message: "Pick one from the list to read and answer it."
                )
            }
        }
    }

    private var selection: Binding<String?> {
        Binding(
            get: { store.inboxPath.last },
            set: { id in
                if let id {
                    store.openFromList(id)
                } else if !store.inboxPath.isEmpty {
                    store.inboxPath = []
                }
            }
        )
    }
}

private struct InboxList: View {
    @Environment(AppStore.self) private var store
    /// Set where the list is the left column of two: a row is picked, not pushed.
    var selection: Binding<String?>?
    @State private var brokerTrouble = false
    @State private var loadingTooLong = false

    var body: some View {
        let sections = store.inboxSections()
        let readiness = store.listReadiness
        List {
            if store.filter != .all {
                FilterNote(name: filterName) { store.filter = .all }
            }
            ConnectionNotices(scope: .inbox, brokerTrouble: brokerTrouble)
            if sections.isEmpty, readiness == .loading, !loadingTooLong {
                SkeletonRows()
            }
            ForEach(sections) { section in
                Section {
                    ForEach(section.items) { item in
                        row(item)
                            .accessibilityIdentifier("inbox-row-\(item.requestId)")
                    }
                } header: {
                    SectionTitle(section: section, showMac: store.macs.count > 1)
                }
            }
        }
        .listStyle(.insetGrouped)
        .emptyState {
            // An empty list only means "nothing waiting" once the Macs have answered.
            if sections.isEmpty {
                switch readiness {
                case .ready: EmptyInbox(filtered: store.filter != .all)
                case .unreachable: UnreachableInbox()
                case .loading: if loadingTooLong { SlowInbox() }
                }
            }
        }
        .refreshable { await store.refresh() }
        .raised($brokerTrouble, after: 10, while: store.syncStatus == .reconnecting)
        .raised($loadingTooLong, after: 15, while: readiness == .loading)
    }

    @ViewBuilder
    private func row(_ item: InboxItem) -> some View {
        let content = InboxRow(item: item, hasDraft: store.hasDraft(for: item))
        if let selection {
            content.pickable(picked: selection.wrappedValue == item.id) { selection.wrappedValue = item.id }
        } else {
            NavigationLink(value: item.id) { content }
        }
    }

    private var filterName: String {
        switch store.filter {
        case .all: ""
        case .mac(let id): store.macs.first { $0.id == id }?.name ?? ""
        case .asks: String(localized: "Questions")
        case .confirms: String(localized: "Confirmations")
        }
    }
}

private struct SectionTitle: View {
    let section: InboxSection
    let showMac: Bool

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "folder.fill").foregroundStyle(.secondary).imageScale(.small)
            Text(section.projectName.isEmpty ? String(localized: "No project") : section.projectName)
            if showMac {
                Text("·").foregroundStyle(.tertiary)
                Text(section.macName).foregroundStyle(.secondary)
            }
        }
        .font(.footnote.weight(.semibold))
        .textCase(nil)
    }
}

private struct EmptyInbox: View {
    let filtered: Bool

    var body: some View {
        if filtered {
            ContentUnavailableView("No matching requests", systemImage: "line.3.horizontal.decrease.circle", description: Text("Change the filter to see the others."))
        } else {
            ContentUnavailableView("All caught up", systemImage: "checkmark.circle", description: Text("Nothing is waiting for your answer."))
        }
    }
}

/// No Mac can be reached, so there is nothing to list: that is not the same as nothing waiting.
private struct UnreachableInbox: View {
    var body: some View {
        ContentUnavailableView(
            "Your Macs are not reachable", systemImage: "desktopcomputer.trianglebadge.exclamationmark",
            description: Text("Requests show up here as soon as a Mac is connected.")
        )
    }
}

/// A Mac that stays silent for long: say so instead of showing grey bars for ever.
private struct SlowInbox: View {
    var body: some View {
        ContentUnavailableView(
            "Waiting for your Mac", systemImage: "hourglass",
            description: Text("It has not answered yet. Pull down to try again.")
        )
    }
}

// MARK: filter

private struct InboxFilterMenu: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        @Bindable var store = store
        Menu {
            Picker("Show", selection: $store.filter) {
                Label("All", systemImage: "tray.full").tag(InboxFilter.all)
                Label("Questions", systemImage: "questionmark.bubble").tag(InboxFilter.asks)
                Label("Confirmations", systemImage: "checkmark.shield").tag(InboxFilter.confirms)
            }
            .pickerStyle(.inline)
            if store.macs.count > 1 {
                Picker("Mac", selection: $store.filter) {
                    ForEach(store.macs) { mac in
                        Label(mac.name, systemImage: "desktopcomputer").tag(InboxFilter.mac(mac.id))
                    }
                }
                .pickerStyle(.inline)
            }
        } label: {
            FilterIcon(active: store.filter != .all)
        }
        .accessibilityLabel("Filter")
        .accessibilityIdentifier("filter-menu")
    }
}
