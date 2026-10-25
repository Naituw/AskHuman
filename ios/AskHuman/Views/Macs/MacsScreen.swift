import SwiftUI
import AHCore
import AHProtocol
import AHStore

/// The Macs tab: paired Macs, adding one, and the app's settings.
struct MacsScreen: View {
    @Environment(AppStore.self) private var store
    @Environment(PushCoordinator.self) private var push
    @Binding var showPairing: Bool
    @State private var confirmForget = false
    @State private var deviceName = DeviceName.current

    var body: some View {
        @Bindable var store = store
        NavigationStack {
            List {
                Section {
                    ForEach(store.macs) { mac in
                        NavigationLink(value: mac.id) { MacRow(mac: mac) }
                            .accessibilityIdentifier("mac-row-\(mac.id)")
                    }
                    Button {
                        showPairing = true
                    } label: {
                        Label("Add a Mac", systemImage: "plus.circle.fill")
                    }
                    .accessibilityIdentifier("add-mac")
                } header: {
                    Text("Paired Macs")
                } footer: {
                    if store.macs.isEmpty { Text("No Mac is paired yet.") }
                }

                Section("Notifications") {
                    NotificationRow()
                }

                Section {
                    Toggle("Open the next request after sending", isOn: $store.autoAdvance)
                    LabeledContent("This phone is called") {
                        TextField("Name", text: $deviceName)
                            .multilineTextAlignment(.trailing)
                            .submitLabel(.done)
                            .onSubmit { saveDeviceName() }
                    }
                } header: {
                    Text("General")
                } footer: {
                    Text("The name your Macs show for this phone. It applies to pairings you make from now on.")
                }

                if let traffic = store.state.traffic {
                    Section {
                        LabeledContent("Sent", value: Int(traffic.sentBytes).formatted(.file))
                        LabeledContent("Received", value: Int(traffic.receivedBytes).formatted(.file))
                        LabeledContent("Messages", value: "\(traffic.sentMessages + traffic.receivedMessages)")
                        LabeledContent("Of the free allowance", value: Self.share(of: traffic))
                    } header: {
                        Text("This month")
                    } footer: {
                        switch traffic.level {
                        case .ok:
                            Text("Estimated from what this phone sent and received through the broker. The broker's free allowance is 1 GB a month.")
                        case .warning:
                            Text("Close to the broker's free allowance of 1 GB a month. The service pauses when it runs out; nothing is charged.")
                        case .exhausted:
                            Text("By this phone's count the free allowance may be used up, so the broker may refuse connections. Check the usage in the EMQX console.")
                        }
                    }
                }

                Section {
                    LabeledContent("Version", value: Self.version)
                    if let host = store.state.brokerHost {
                        LabeledContent("Broker", value: host)
                    }
                    if store.hasPairedMacs || store.state.brokerHost != nil {
                        Button("Forget all pairings", role: .destructive) { confirmForget = true }
                    }
                } header: {
                    Text("About")
                }
            }
            .navigationTitle("Macs")
            .navigationDestination(for: String.self) { id in MacDetailScreen(macId: id) }
            .confirmationDialog("Forget all pairings?", isPresented: $confirmForget, titleVisibility: .visible) {
                Button("Forget all pairings", role: .destructive) { Task { await store.forgetAll() } }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("This removes every Mac and the broker account from this phone. Your Macs are told, and you can pair again at any time.")
            }
            .onChange(of: deviceName) { _, _ in }
            .onDisappear { saveDeviceName() }
        }
    }

    private func saveDeviceName() {
        DeviceName.current = deviceName
        Task { await store.setDeviceName(DeviceName.current) }
    }

    /// "<1%", "37%": what the traffic is likely to have used of the broker's free allowance.
    private static func share(of traffic: TrafficStats) -> String {
        let share = traffic.quotaShare
        return share < 0.01 ? "<1%" : share.formatted(.percent.precision(.fractionLength(0)))
    }

    private static var version: String {
        let info = Bundle.main.infoDictionary
        let short = info?["CFBundleShortVersionString"] as? String ?? "0"
        let build = info?["CFBundleVersion"] as? String ?? "0"
        return "\(short) (\(build))"
    }
}

private struct MacRow: View {
    let mac: MacState

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "desktopcomputer")
                .font(.title3)
                .foregroundStyle(.white)
                .frame(width: 38, height: 38)
                .background(Color.accentColor.gradient, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            VStack(alignment: .leading, spacing: 3) {
                Text(mac.name).font(.headline)
                HStack(spacing: 6) {
                    StatusDot(color: mac.link.color)
                    Text(mac.link.title).font(.footnote).foregroundStyle(.secondary)
                    if mac.link != .online, let seen = mac.lastSeenAt {
                        Text("· last seen \(Text(seen, style: .relative)) ago")
                    }
                }
                .font(.footnote)
                .foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
            if mac.pendingCount > 0 {
                Text("\(mac.pendingCount)")
                    .font(.footnote.weight(.semibold))
                    .padding(.horizontal, 8)
                    .padding(.vertical, 2)
                    .background(Color.accentColor.opacity(0.15), in: Capsule())
            }
        }
        .padding(.vertical, 2)
    }
}

private struct NotificationRow: View {
    @Environment(AppStore.self) private var store
    @Environment(PushCoordinator.self) private var push

    var body: some View {
        switch push.authorization {
        case .authorized:
            LabeledContent("Notifications") {
                HStack(spacing: 6) {
                    StatusDot(color: statusColor)
                    Text(statusText)
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("push-status")
            }
        case .notDetermined, .unknown:
            VStack(alignment: .leading, spacing: 8) {
                Text("Turn on notifications so a new question reaches you even when AskHuman is closed.").font(.footnote).foregroundStyle(.secondary)
                Button("Turn on notifications") { Task { await push.requestAuthorization() } }
            }
        case .denied:
            VStack(alignment: .leading, spacing: 8) {
                Label("Notifications are off for AskHuman", systemImage: "bell.slash.fill").foregroundStyle(.orange)
                Text("Without them you only see new questions while the app is open.").font(.footnote).foregroundStyle(.secondary)
                Button("Open Settings") { push.openSystemSettings() }
            }
        }
    }

    private var statusText: LocalizedStringKey {
        switch store.state.push {
        case .registered: "On"
        case .registering: "Registering…"
        case .noToken: "Waiting for Apple"
        case .noRelay: "No push relay configured"
        case .failed: "Registration failed"
        }
    }

    private var statusColor: Color {
        switch store.state.push {
        case .registered: .green
        case .registering, .noToken: .orange
        case .noRelay, .failed: .red
        }
    }
}
