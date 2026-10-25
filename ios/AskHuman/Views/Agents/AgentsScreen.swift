import SwiftUI

struct AgentsScreen: View {
    var body: some View {
        NavigationStack {
            ContentUnavailableView(
                "Agents", systemImage: "person.2.wave.2",
                description: Text("Following and watching your Agents from this phone is coming soon.")
            )
            .navigationTitle("Agents")
        }
    }
}
