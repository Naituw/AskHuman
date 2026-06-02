import SwiftUI

struct SettingsView: View {
    @ObservedObject var viewModel: SettingsViewModel

    var body: some View {
        TabView {
            GeneralTab(viewModel: viewModel)
                .tabItem { Label("General", systemImage: "gearshape") }
            IntegrationTab(viewModel: viewModel)
                .tabItem { Label("集成", systemImage: "puzzlepiece.extension") }
            ChannelTab(viewModel: viewModel)
                .tabItem { Label("Channel", systemImage: "antenna.radiowaves.left.and.right") }
        }
        .frame(width: 580, height: 580)
    }
}
