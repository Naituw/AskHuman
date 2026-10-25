import SwiftUI
import AHCore
import AHProtocol

struct InboxScreen: View {
    @Environment(AppStore.self) private var store
    @Binding var showPairing: Bool

    var body: some View {
        @Bindable var store = store
        NavigationStack(path: $store.inboxPath) {
            Group {
                if store.hasPairedMacs {
                    InboxList()
                } else {
                    OnboardingView(showPairing: $showPairing)
                }
            }
            .navigationTitle("Inbox")
            .navigationBarTitleDisplayMode(.inline)
            .navigationDestination(for: String.self) { id in
                RequestScreen(id: id)
            }
        }
    }
}

private struct InboxList: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        let sections = store.inboxSections()
        List {
            ConnectionNotices()
            ForEach(sections) { section in
                Section {
                    ForEach(section.items) { item in
                        NavigationLink(value: item.id) {
                            InboxRow(item: item, hasDraft: store.hasDraft(for: item))
                        }
                        .accessibilityIdentifier("inbox-row-\(item.requestId)")
                    }
                } header: {
                    SectionTitle(section: section, showMac: store.macs.count > 1)
                }
            }
        }
        .listStyle(.insetGrouped)
        .overlay {
            if sections.isEmpty { EmptyInbox(filtered: store.filter != .all) }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            if store.state.inbox.count > 0 || store.filter != .all { FilterBar() }
        }
        .refreshable { await store.refresh() }
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

// MARK: filter

private struct FilterBar: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                chip(Text("All"), filter: .all, id: "all")
                if store.macs.count > 1 {
                    ForEach(store.macs) { mac in
                        chip(Text(mac.name), filter: .mac(mac.id), id: "mac-" + mac.id)
                    }
                }
                chip(Text("Questions"), filter: .asks, id: "asks")
                chip(Text("Confirmations"), filter: .confirms, id: "confirms")
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
        }
        .background(.bar)
    }

    private func chip(_ label: Text, filter: InboxFilter, id: String) -> some View {
        let selected = store.filter == filter
        return Button {
            store.filter = filter
        } label: {
            label
                .font(.subheadline.weight(.medium))
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .foregroundStyle(selected ? Color.white : Color.primary)
                .background(selected ? Color.accentColor : Color(.tertiarySystemFill), in: Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityIdentifier("filter-" + id)
    }
}

// MARK: notices

/// Reasons the inbox may be stale or unanswerable, shown above the list.
private struct ConnectionNotices: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        let notices = notices()
        if !notices.isEmpty {
            Section {
                ForEach(notices, id: \.id) { notice in
                    Label {
                        Text(notice.text).font(.footnote)
                    } icon: {
                        Image(systemName: notice.symbol).foregroundStyle(notice.color)
                    }
                    .listRowBackground(notice.color.opacity(0.1))
                }
            }
        }
    }

    private struct Notice {
        var id: String
        var text: LocalizedStringKey
        var symbol: String
        var color: Color
    }

    private func notices() -> [Notice] {
        var result: [Notice] = []
        switch store.state.phase {
        case .refused:
            result.append(Notice(id: "refused", text: "The broker refused the connection. Check its settings on your Mac and pair again.", symbol: "exclamationmark.octagon.fill", color: .red))
        case .backoff, .connecting:
            if store.hasPairedMacs, store.state.foreground {
                result.append(Notice(id: "broker", text: "Connecting to the broker… Pull down to retry.", symbol: "wifi.exclamationmark", color: .orange))
            }
        default: break
        }
        if store.state.clockSkew {
            result.append(Notice(id: "clock", text: "This phone's clock differs too much from a Mac's, so some messages are ignored. Check Date & Time in Settings.", symbol: "clock.badge.exclamationmark.fill", color: .orange))
        }
        for mac in store.macs where mac.link == .macOffline && store.state.inbox.contains(where: { $0.macId == mac.id }) {
            result.append(Notice(id: "offline-" + mac.id, text: "\(mac.name) is offline. Its requests cannot be answered until it is back.", symbol: "desktopcomputer.trianglebadge.exclamationmark", color: .gray))
        }
        return result
    }
}
