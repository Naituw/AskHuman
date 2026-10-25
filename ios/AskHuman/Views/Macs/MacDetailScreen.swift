import SwiftUI
import AHCore
import AHProtocol

struct MacDetailScreen: View {
    let macId: String
    @Environment(AppStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var renaming = false
    @State private var alias = ""
    @State private var confirmUnpair = false

    var body: some View {
        if let mac = store.macs.first(where: { $0.id == macId }) {
            List {
                Section {
                    LabeledContent("Name") {
                        Button(mac.name) {
                            alias = mac.alias ?? ""
                            renaming = true
                        }
                    }
                    if mac.alias != nil {
                        LabeledContent("Mac's own name", value: mac.macName)
                    }
                    LabeledContent("Status") {
                        HStack(spacing: 6) {
                            StatusDot(color: mac.link.color)
                            Text(mac.link.title)
                        }
                    }
                    if let seen = mac.lastSeenAt {
                        LabeledContent("Last seen") { Text("\(Text(seen, style: .relative)) ago") }
                    }
                    LabeledContent("Paired") { Text(mac.pairedAt, format: .dateTime.year().month().day()) }
                } footer: {
                    Text("The name is only changed on this phone.")
                }

                Section {
                    LabeledContent("Waiting for you", value: "\(mac.pendingCount)")
                    if let slot = mac.slot {
                        if slot.autoActivation {
                            LabeledContent("Questions go to") {
                                Text(slot.active ? "This phone" : "Another channel")
                            }
                        } else {
                            Label("Every question is sent to your phone.", systemImage: "iphone.radiowaves.left.and.right")
                                .font(.footnote)
                        }
                    }
                } header: {
                    Text("Questions")
                }

                Section("Push notifications") {
                    LabeledContent("Registered on this Mac") { Text(pushText(mac.pushStatus)) }
                }

                Section {
                    Button("Unpair this Mac", role: .destructive) { confirmUnpair = true }
                } footer: {
                    Text("The Mac is told, and its keys are removed from this phone. You can pair again at any time.")
                }
            }
            .navigationTitle(mac.name)
            .navigationBarTitleDisplayMode(.inline)
            .alert("Rename Mac", isPresented: $renaming) {
                TextField(mac.macName, text: $alias)
                Button("Save") { Task { await store.rename(mac, to: alias) } }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("Leave it empty to use the name the Mac gives itself.")
            }
            .confirmationDialog("Unpair \(mac.name)?", isPresented: $confirmUnpair, titleVisibility: .visible) {
                Button("Unpair", role: .destructive) {
                    Task {
                        await store.unpair(mac)
                        dismiss()
                    }
                }
                Button("Cancel", role: .cancel) {}
            }
        } else {
            ContentUnavailableView("This Mac is no longer paired", systemImage: "desktopcomputer.slash")
        }
    }

    private func pushText(_ status: String?) -> LocalizedStringKey {
        switch status {
        case "ok": "Yes"
        case "missing": "Not yet"
        case "gone": "Expired — refreshing"
        case "error": "Error"
        default: "Unknown"
        }
    }
}
