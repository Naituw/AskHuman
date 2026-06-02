import SwiftUI

struct GeneralTab: View {
    @ObservedObject var viewModel: SettingsViewModel

    var body: some View {
        Form {
            Section("外观") {
                Picker("主题", selection: $viewModel.config.general.theme) {
                    Text("跟随系统").tag(ThemeMode.system)
                    Text("浅色").tag(ThemeMode.light)
                    Text("深色").tag(ThemeMode.dark)
                }
                .pickerStyle(.segmented)
            }

            Section("弹窗行为") {
                Toggle("窗口置顶", isOn: $viewModel.config.general.alwaysOnTop)
            }
        }
        .formStyle(.grouped)
        .padding()
    }
}
