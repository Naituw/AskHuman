import SwiftUI
import AHCore
import AHProtocol

/// The Agents tab: every session on the paired Macs, the ones waiting for you first.
struct AgentsScreen: View {
    @Environment(AppStore.self) private var store
    @Binding var showPairing: Bool

    var body: some View {
        ColumnsOrStack {
            AgentsColumns(showPairing: $showPairing)
        } stack: {
            stack
        }
        .sheet(isPresented: Binding(get: { store.isTaskSheetPresented }, set: { if !$0 { store.dismissNewTask() } })) {
            NewTaskSheet()
        }
    }

    /// The phone's way: the list, and the session pushed over it.
    @ViewBuilder
    private var stack: some View {
        @Bindable var store = store
        NavigationStack(path: $store.agentPath) {
            Group {
                if store.hasPairedMacs {
                    AgentsList()
                } else {
                    OnboardingView(showPairing: $showPairing)
                }
            }
            .modifier(AgentsBar())
            .navigationDestination(for: String.self) { id in
                AgentDetailScreen(id: id)
            }
        }
    }
}

/// A wide window: the sessions on the left, and on the right the one that was picked from them.
///
/// What the list has picked is what the tab has open (`agentPath` holds that one session, as it does on
/// the phone), so a session opened from a notification or a banner is the one the list shows as picked.
private struct AgentsColumns: View {
    @Environment(AppStore.self) private var store
    @Binding var showPairing: Bool

    var body: some View {
        let open = store.agentPath.last
        // Before the Macs have answered nobody knows whether one is paired: the list stays until they say
        // so, as it does for everyone who has one.
        let hasList = store.hasPairedMacs || !store.hasState
        ListDetailColumns(hasList: hasList, opened: open) {
            AgentsList(selection: selection)
                .modifier(AgentsBar())
        } detail: {
            if !hasList {
                OnboardingView(showPairing: $showPairing)
            } else if let open {
                AgentDetailScreen(id: open)
                    // Each session has its own page: what is typed or shown on one is not carried to the next.
                    .id(open)
            } else {
                DetailPlaceholder(
                    title: "Select a session", symbol: "person.2.wave.2",
                    message: "Pick one from the list to follow it and talk to it."
                )
            }
        }
    }

    private var selection: Binding<String?> {
        Binding(
            get: { store.agentPath.last },
            set: { id in
                if let id {
                    if store.agentPath.last != id { store.agentPath = [id] }
                } else if !store.agentPath.isEmpty {
                    store.agentPath = []
                }
            }
        )
    }
}

/// The title and the buttons in the bar of the list of sessions.
private struct AgentsBar: ViewModifier {
    @Environment(AppStore.self) private var store

    func body(content: Content) -> some View {
        content
            .navigationTitle("Agents")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if store.hasPairedMacs {
                    ToolbarItem(placement: .principal) { SyncTitle(title: "Agents") }
                }
                // The only thing to filter by is the Mac.
                if store.macs.count > 1 {
                    ToolbarItem(placement: .topBarLeading) { AgentFilterMenu() }
                }
                if !store.taskMacs.isEmpty {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button {
                            store.presentNewTask()
                        } label: {
                            Image(systemName: "square.and.pencil")
                        }
                        .accessibilityLabel("New task")
                        .accessibilityIdentifier("new-task")
                    }
                }
            }
    }
}

private struct AgentsList: View {
    @Environment(AppStore.self) private var store
    /// Set where the list is the left column of two: a row is picked, not pushed.
    var selection: Binding<String?>?
    @State private var brokerTrouble = false
    @State private var loadingTooLong = false

    var body: some View {
        let sections = store.agentSections()
        let tasks = store.openTasks
        let readiness = store.listReadiness
        List {
            if store.agentFilter != .all {
                FilterNote(name: filterName) { store.agentFilter = .all }
            }
            ConnectionNotices(scope: .agents, brokerTrouble: brokerTrouble)
            if sections.isEmpty, tasks.isEmpty, readiness == .loading, !loadingTooLong {
                SkeletonRows()
            }
            if !tasks.isEmpty {
                Section {
                    ForEach(tasks) { task in
                        Button { store.presentTaskProgress(task.id) } label: { TaskRow(task: task) }
                            .buttonStyle(.plain)
                            .accessibilityIdentifier("task-row-\(task.id)")
                    }
                } header: {
                    Text("Starting").font(.footnote.weight(.semibold)).textCase(nil)
                }
            }
            ForEach(sections) { section in
                Section {
                    ForEach(section.items) { item in
                        row(item)
                    }
                } header: {
                    AgentSectionTitle(id: section.id, count: section.items.count)
                }
            }
        }
        .listStyle(.insetGrouped)
        .emptyState {
            // An empty list only means "no sessions" once the Macs have answered.
            if sections.isEmpty, tasks.isEmpty {
                switch readiness {
                case .ready: EmptyAgents(filtered: store.agentFilter != .all)
                case .unreachable: UnreachableAgents()
                case .loading: if loadingTooLong { SlowAgents() }
                }
            }
        }
        .refreshable { await store.refresh() }
        .raised($brokerTrouble, after: 10, while: store.syncStatus == .reconnecting)
        .raised($loadingTooLong, after: 15, while: readiness == .loading)
    }

    @ViewBuilder
    private func row(_ item: AgentItem) -> some View {
        let content = AgentRow(item: item, showMac: store.macs.count > 1)
        if let selection {
            content.pickable(picked: selection.wrappedValue == item.id) { selection.wrappedValue = item.id }
                .accessibilityIdentifier("agent-row-\(item.agentKey)")
        } else {
            NavigationLink(value: item.id) { content }
                .accessibilityIdentifier("agent-row-\(item.agentKey)")
        }
    }

    private var filterName: String {
        switch store.agentFilter {
        case .all: ""
        case .mac(let id): store.macs.first { $0.id == id }?.name ?? ""
        }
    }
}

/// A task that was sent and has no Agent session yet.
private struct TaskRow: View {
    let task: TaskProgress

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            ZStack {
                if task.isActive {
                    ProgressView()
                } else {
                    Image(systemName: task.stage == "unmatched" ? "questionmark.circle.fill" : "xmark.octagon.fill")
                        .foregroundStyle(task.stage == "unmatched" ? Color.orange : Color.red)
                }
            }
            .frame(width: 36, height: 36)
            VStack(alignment: .leading, spacing: 3) {
                Text(verbatim: task.agentLabel + " · " + task.workspace)
                    .font(.subheadline.weight(.semibold))
                    .lineLimit(1)
                Text(verbatim: task.text)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
                status.font(.caption).foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
            Image(systemName: "chevron.right").font(.footnote.weight(.semibold)).foregroundStyle(.tertiary)
        }
        .padding(.vertical, 4)
        .contentShape(Rectangle())
    }

    private var status: Text {
        switch task.stage {
        case "failed", "lost": Text("Not started")
        case "unmatched": Text("Has not reported in")
        case "launched": Text("Waiting for the session…")
        default: Text("Starting…")
        }
    }
}

private struct AgentSectionTitle: View {
    let id: String
    let count: Int

    var body: some View {
        HStack(spacing: 6) {
            StatusDot(color: color)
            title
            Text("\(count)").foregroundStyle(.secondary)
        }
        .font(.footnote.weight(.semibold))
        .textCase(nil)
    }

    private var title: Text {
        switch id {
        case "waiting": Text("Waiting for you")
        case "working": Text("Working")
        case "ended": Text("Session ended")
        default: Text("Idle")
        }
    }

    private var color: Color {
        switch id {
        case "waiting": .orange
        case "working": .green
        default: .gray
        }
    }
}

private struct EmptyAgents: View {
    @Environment(AppStore.self) private var store
    let filtered: Bool

    var body: some View {
        if filtered {
            ContentUnavailableView("No matching sessions", systemImage: "line.3.horizontal.decrease.circle", description: Text("Change the filter to see the others."))
        } else {
            ContentUnavailableView {
                Label("No Agent sessions", systemImage: "person.2.wave.2")
            } description: {
                Text("Start an Agent on your Mac, or create a task from here.")
            } actions: {
                if !store.taskMacs.isEmpty {
                    Button("New task") { store.presentNewTask() }
                        .buttonStyle(.borderedProminent)
                }
            }
        }
    }
}

/// No Mac can be reached, so no sessions can be listed.
private struct UnreachableAgents: View {
    var body: some View {
        ContentUnavailableView(
            "Your Macs are not reachable", systemImage: "desktopcomputer.trianglebadge.exclamationmark",
            description: Text("Agent sessions show up here while a Mac is connected.")
        )
    }
}

/// A Mac that stays silent for long: say so instead of showing grey bars for ever.
private struct SlowAgents: View {
    var body: some View {
        ContentUnavailableView(
            "Waiting for your Mac", systemImage: "hourglass",
            description: Text("It has not answered yet. Pull down to try again.")
        )
    }
}

// MARK: filter

private struct AgentFilterMenu: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        @Bindable var store = store
        Menu {
            Picker("Show", selection: $store.agentFilter) {
                Label("All", systemImage: "person.2.wave.2").tag(AgentFilter.all)
            }
            .pickerStyle(.inline)
            Picker("Mac", selection: $store.agentFilter) {
                ForEach(store.macs) { mac in
                    Label(mac.name, systemImage: "desktopcomputer").tag(AgentFilter.mac(mac.id))
                }
            }
            .pickerStyle(.inline)
        } label: {
            FilterIcon(active: store.agentFilter != .all)
        }
        .accessibilityLabel("Filter")
        .accessibilityIdentifier("filter-menu")
    }
}
