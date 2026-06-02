import SwiftUI

struct IntegrationTab: View {
    @ObservedObject var viewModel: SettingsViewModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                promptCard
                hookCard
            }
            .padding()
        }
    }

    private var promptCard: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Label("参考提示词", systemImage: "text.quote")
                        .font(.headline)
                    Spacer()
                    Button(viewModel.promptCopied ? "已复制" : "复制", systemImage: "doc.on.doc") {
                        viewModel.copyPrompt()
                    }
                }
                Text("把以下提示词加入你的 AI 助手，引导它通过 AskHuman 与你交互。")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                ScrollView {
                    Text(Prompts.cliReference)
                        .font(.system(.caption, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(8)
                }
                .frame(height: 180)
                .background(
                    RoundedRectangle(cornerRadius: 6)
                        .fill(Color.secondary.opacity(0.08))
                )
            }
            .padding(6)
        }
    }

    private var hookCard: some View {
        GroupBox {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Label("Cursor Hook", systemImage: "bolt.fill")
                        .font(.headline)
                    Spacer()
                    statusBadge
                }
                Text("安装后会在 ~/.cursor/hooks.json 注册 preToolUse 钩子：检测到 Shell 调用 AskHuman 时自动把超时延长到 24 小时，避免长时间等待被强制取消。移除时仅删除本应用注入的条目。")
                    .font(.caption)
                    .foregroundStyle(.secondary)

                HStack {
                    if viewModel.hookInstalled {
                        Button("移除", systemImage: "trash") {
                            viewModel.uninstallHook()
                        }
                    } else {
                        Button("安装", systemImage: "square.and.arrow.down") {
                            viewModel.installHook()
                        }
                    }
                    Button("打开 hooks.json", systemImage: "folder") {
                        viewModel.revealHooks()
                    }
                    .disabled(!viewModel.hooksJSONExists)
                    Spacer()
                }

                if let msg = viewModel.hookMessage {
                    Text(msg)
                        .font(.caption)
                        .foregroundStyle(viewModel.hookError ? Color.red : Color.green)
                }
            }
            .padding(6)
        }
    }

    private var statusBadge: some View {
        HStack(spacing: 5) {
            Circle()
                .fill(viewModel.hookInstalled ? Color.green : Color.orange)
                .frame(width: 8, height: 8)
            Text(viewModel.hookInstalled ? "已安装" : "未安装")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }
}
