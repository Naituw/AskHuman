import SwiftUI

struct ChannelTab: View {
    @ObservedObject var viewModel: SettingsViewModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                popupCard
                telegramCard
                futureCard
            }
            .padding()
        }
    }

    private var popupCard: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 10) {
                Label("本地弹窗", systemImage: "macwindow")
                    .font(.headline)
                Toggle("启用本地弹窗", isOn: $viewModel.config.channels.popup.enabled)

                if viewModel.config.channels.popup.enabled {
                    Divider()
                    Toggle("记住窗口尺寸", isOn: $viewModel.config.channels.popup.rememberSize)
                    HStack {
                        Text("默认窗口尺寸")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                        Spacer()
                        Stepper(value: $viewModel.config.channels.popup.width, in: 360...1200, step: 20) {
                            Text("宽 \(Int(viewModel.config.channels.popup.width))")
                                .font(.caption.monospacedDigit())
                        }
                        Stepper(value: $viewModel.config.channels.popup.height, in: 360...1400, step: 20) {
                            Text("高 \(Int(viewModel.config.channels.popup.height))")
                                .font(.caption.monospacedDigit())
                        }
                    }
                }
            }
            .padding(6)
        }
    }

    private var telegramCard: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 10) {
                Label("Telegram", systemImage: "paperplane")
                    .font(.headline)
                Toggle("启用 Telegram", isOn: $viewModel.config.channels.telegram.enabled)

                if viewModel.config.channels.telegram.enabled {
                    Divider()
                    LabeledField(label: "Bot Token", text: $viewModel.config.channels.telegram.botToken)
                    LabeledField(label: "Chat ID", text: $viewModel.config.channels.telegram.chatId)
                    LabeledField(label: "API Base URL", text: $viewModel.config.channels.telegram.apiBaseUrl)

                    HStack {
                        Button(viewModel.telegramTesting ? "测试中…" : "测试连接", systemImage: "checkmark.seal") {
                            viewModel.testTelegram()
                        }
                        .disabled(viewModel.telegramTesting)
                        Spacer()
                    }

                    if let msg = viewModel.telegramMessage {
                        Text(msg)
                            .font(.caption)
                            .foregroundStyle(viewModel.telegramError ? Color.red : Color.green)
                    }
                }
            }
            .padding(6)
        }
    }

    private var futureCard: some View {
        GroupBox {
            HStack {
                Image(systemName: "plus.circle.dashed")
                    .foregroundStyle(.secondary)
                Text("更多通信 Channel 敬请期待")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Spacer()
            }
            .padding(6)
        }
    }
}

private struct LabeledField: View {
    let label: String
    @Binding var text: String

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(label)
                .font(.caption)
                .foregroundStyle(.secondary)
            TextField(label, text: $text)
                .textFieldStyle(.roundedBorder)
        }
    }
}
