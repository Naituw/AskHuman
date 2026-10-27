import SwiftUI
import AHCore
import AHProtocol

/// The Agents tab: every session on the paired Macs, the ones that need you first.
struct AgentsScreen: View {
    @Environment(AppStore.self) private var store
    @Binding var showPairing: Bool

    var body: some View {
        @Bindable var store = store
        NavigationStack(path: $store.agentPath) {
            Group {
                if store.hasPairedMacs {
                    AgentsList()
                } else {
                    OnboardingView(showPairing: $showPairing)
                }
            }
            .navigationTitle("Agents")
            .navigationBarTitleDisplayMode(.inline)
            .navigationDestination(for: String.self) { id in
                AgentDetailScreen(id: id)
            }
            .toolbar {
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
        .sheet(isPresented: Binding(get: { store.isTaskSheetPresented }, set: { if !$0 { store.dismissNewTask() } })) {
            NewTaskSheet()
        }
    }
}

private struct AgentsList: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        let sections = store.agentSections()
        let tasks = store.openTasks
        List {
            OfflineMacNotices()
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
                        NavigationLink(value: item.id) {
                            AgentRow(item: item, showMac: store.macs.count > 1)
                        }
                        .accessibilityIdentifier("agent-row-\(item.agentKey)")
                    }
                } header: {
                    AgentSectionTitle(id: section.id, count: section.items.count)
                }
            }
        }
        .listStyle(.insetGrouped)
        .overlay {
            if sections.isEmpty, tasks.isEmpty { EmptyAgents(filtered: store.agentFilter != .all) }
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            if !store.state.agents.isEmpty || store.agentFilter != .all { AgentFilterBar() }
        }
        .refreshable { await store.refresh() }
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
        case "ended": Text("Finished")
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
        } else if store.macs.allSatisfy({ $0.link != .online }) {
            ContentUnavailableView("Your Macs are not reachable", systemImage: "desktopcomputer.trianglebadge.exclamationmark", description: Text("Agent sessions show up here while a Mac is connected."))
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

// MARK: filter

private struct AgentFilterBar: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                chip(Text("All"), filter: .all, id: "all")
                chip(Text("Following"), filter: .followed, id: "followed")
                if store.macs.count > 1 {
                    ForEach(store.macs) { mac in
                        chip(Text(mac.name), filter: .mac(mac.id), id: "mac-" + mac.id)
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
        }
        .background(.bar)
    }

    private func chip(_ label: Text, filter: AgentFilter, id: String) -> some View {
        let selected = store.agentFilter == filter
        return Button {
            store.agentFilter = filter
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
        .accessibilityIdentifier("agent-filter-" + id)
    }
}

/// Macs whose sessions are missing from the list because they cannot be reached.
private struct OfflineMacNotices: View {
    @Environment(AppStore.self) private var store

    var body: some View {
        let offline = store.macs.filter { $0.link != .online }
        if !offline.isEmpty, offline.count < store.macs.count || !store.state.agents.isEmpty {
            Section {
                ForEach(offline) { mac in
                    Label {
                        Text("\(mac.name) is not reachable. Its sessions are hidden until it is back.").font(.footnote)
                    } icon: {
                        Image(systemName: "desktopcomputer.trianglebadge.exclamationmark").foregroundStyle(.gray)
                    }
                    .listRowBackground(Color.gray.opacity(0.1))
                }
            }
        }
    }
}
