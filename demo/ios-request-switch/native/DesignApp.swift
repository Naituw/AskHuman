import SwiftUI

/// Standalone design study. No backend, pairing, upload or Agent operations are linked.
@main
struct RequestSwitchDesignApp: App {
    @State private var path = ["request"]

    var body: some Scene {
        WindowGroup {
            NavigationStack(path: $path) {
                List { NavigationLink("修复 Node 22 的 CI", value: "request") }
                    .navigationTitle("收件箱")
                    .navigationDestination(for: String.self) { _ in RequestDesign() }
            }
            .tint(.primary)
            .environment(\.locale, Locale(identifier: "zh_CN"))
        }
    }
}

private struct RequestDesign: View {
    @State private var index = 1
    @State private var drafts: [Int: String] = [:]
    @State private var selections: [Int: Int] = [:]
    @FocusState private var editing: Bool
    private let labels = ["Codex", "Claude Code", "Cursor"]
    private let titles = ["这次先发布到内部 TestFlight 吗？", "要把 CI 矩阵固定在 Node 20 吗？", "先验证哪种迁移方式？"]
    private let messages = ["通知与连接状态已更新，接下来需要确认发布范围。", "测试在 Node 20 上通过，在 Node 22 上有 2 个失败。", "迁移方案已经整理，在线双写和停机迁移都可行。"]
    private let choices = [["先发内部测试", "再补一轮检查"], ["固定在 Node 20", "继续修复 Node 22"], ["在线双写", "停机迁移"]]

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                header
                Text(messages[index]).frame(maxWidth: .infinity, alignment: .leading).padding(16).background(.background, in: .rect(cornerRadius: 16))
                VStack(alignment: .leading, spacing: 12) {
                    Text(titles[index]).font(.headline)
                    ForEach(0..<2) { option in
                        Button { selections[index] = option } label: {
                            HStack(spacing: 12) {
                                Image(systemName: selections[index] == option ? "largecircle.fill.circle" : "circle")
                                    .font(.title3).foregroundStyle(selections[index] == option ? Color.blue : Color(.tertiaryLabel))
                                Text(choices[index][option]).foregroundStyle(.primary)
                                Spacer(minLength: 0)
                            }
                            .padding(12).frame(minHeight: 48)
                            .background(selections[index] == option ? Color.blue.opacity(0.12) : Color(.tertiarySystemFill), in: .rect(cornerRadius: 12))
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(16).background(.background, in: .rect(cornerRadius: 16))
                VStack(alignment: .leading, spacing: 12) {
                    Text("补充说明").font(.headline)
                    TextField("输入补充说明…", text: Binding(get: { drafts[index] ?? "" }, set: { drafts[index] = $0 }), axis: .vertical)
                        .lineLimit(3...6).focused($editing)
                        .padding(12).background(Color(.tertiarySystemFill), in: .rect(cornerRadius: 10))
                    Label("添加照片或文件", systemImage: "paperclip").font(.footnote).foregroundStyle(.secondary)
                }
                .padding(16).background(.background, in: .rect(cornerRadius: 16))
            }
            .padding(16)
        }
        .background(Color(.systemGroupedBackground))
        .scrollDismissesKeyboard(.interactively)
        .actionBar {
            GlassGroup(spacing: 8) {
                HStack(spacing: 12) {
                    Text("已回答 \(selections[index] != nil || !(drafts[index] ?? "").isEmpty ? 1 : 0)/1")
                        .font(.subheadline.weight(.medium))
                        .barStatusChip(fillsHeight: true)
                    Spacer(minLength: 8)
                    Button {} label: { Label("发送", systemImage: "arrow.up").font(.headline) }
                        .barPrimaryButton().tint(.blue)
                }
                .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.horizontal, 16).padding(.vertical, 10)
        }
        .navigationBarTitleDisplayMode(.inline)
        .toolbarRole(.editor)
        .toolbar {
            if UserDefaults.standard.object(forKey: "DesignMoreLast") == nil || UserDefaults.standard.bool(forKey: "DesignMoreLast") {
                requestNavigation
                ToolbarSpacer(.fixed, placement: .topBarTrailing)
                moreActions
            } else {
                moreActions
                ToolbarSpacer(.fixed, placement: .topBarTrailing)
                requestNavigation
            }
        }
        .task {
            if UserDefaults.standard.bool(forKey: "DesignKeyboard") {
                drafts[index] = "先保留 Node 20，Node 22 单独修复。"
                try? await Task.sleep(for: .seconds(1))
                editing = true
            }
        }
    }

    @ToolbarContentBuilder
    private var moreActions: some ToolbarContent {
        ToolbarItem(placement: .topBarTrailing) {
            Menu {
                Button("取消请求", systemImage: "xmark.circle", role: .destructive) {}
            } label: { Label("更多", systemImage: "ellipsis") }
        }
    }

    @ToolbarContentBuilder
    private var requestNavigation: some ToolbarContent {
        ToolbarItemGroup(placement: .topBarTrailing) {
            Button { move(-1) } label: { Label("上一条消息", systemImage: "chevron.left") }
                .disabled(index == 0)
            Button { move(1) } label: { Label("下一条消息", systemImage: "chevron.right") }
                .disabled(index == 2)
        }
    }

    private var header: some View {
        HStack(spacing: 12) {
            Image("agent-" + ["codex", "claude", "cursor"][index])
                .renderingMode(.template).resizable().scaledToFit().foregroundStyle(.white).padding(10)
                .frame(width: 44, height: 44)
                .background([Color(red: 0.35, green: 0.5, blue: 0.95), Color(red: 0.88, green: 0.53, blue: 0.39), Color.black][index], in: .rect(cornerRadius: 12))
            VStack(alignment: .leading, spacing: 3) {
                Text(labels[index]).font(.headline)
                Text(index == 1 ? "relay · Office iMac" : "HumanInLoop · Home MacBook Pro").font(.footnote).foregroundStyle(.secondary)
                Text("今天 17:30").font(.caption).foregroundStyle(.tertiary)
            }
        }
        .padding(.bottom, 2)
    }

    private func move(_ direction: Int) {
        editing = false
        index = min(2, max(0, index + direction))
    }
}
