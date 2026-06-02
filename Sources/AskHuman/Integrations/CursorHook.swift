import Foundation

enum CursorHookError: Error, CustomStringConvertible {
    case io(String)

    var description: String {
        switch self {
        case .io(let msg): return msg
        }
    }
}

enum CursorHook {
    static let marker = "humaninloop-timeout.sh"

    /// 钩子脚本内容（与桌面端逻辑一致）。使用 raw 字符串避免转义问题。
    static let scriptContent = #"""
    #!/usr/bin/env bash
    # humaninloop-timeout.sh
    # 由 HumanInLoop (Swift Native) 自动安装 / 移除，请勿手动编辑。
    #
    # 作用：作为 Cursor 的 preToolUse 钩子，检测 Shell 工具调用是否会执行 AskHuman
    # 命令；命中时将工具调用 timeout 提升至 24 小时（86400000ms），防止等待用户回应
    # 时被强制取消。未命中或解析失败时输出空对象 `{}`，保持 fail-open。

    set -u

    # 从 stdin 读取 JSON 输入
    input=$(cat)

    # 解析 .tool_input.command，按 python3 -> jq -> 简单 grep 顺序回退
    command=""
    if command -v python3 >/dev/null 2>&1; then
      command=$(printf '%s' "$input" | python3 -c '
    import json
    import sys

    try:
        data = json.load(sys.stdin)
        print(data.get("tool_input", {}).get("command", ""))
    except Exception:
        print("")
    ')
    elif command -v jq >/dev/null 2>&1; then
      command=$(printf '%s' "$input" | jq -r '.tool_input.command // empty' 2>/dev/null)
    else
      command="$input"
    fi

    # 匹配 AskHuman 调用：兼顾行内任意位置 / 链式命令 / 引号包裹 / 绝对路径前缀
    if [ -n "$command" ] && printf '%s' "$command" \
         | grep -Eq "(^|[[:space:];&|()\`\"'\\]|/)AskHuman([[:space:]]|$|[\"'\\])"; then
      output='{"updated_input": {"timeout": 86400000}}'
    else
      output='{}'
    fi

    printf '%s\n' "$output"

    exit 0
    """#

    // MARK: - 状态

    static func isInstalled() -> Bool {
        guard let preToolUse = readPreToolUse() else { return false }
        return preToolUse.contains { entryHasMarker($0) }
    }

    static var hooksJSONExists: Bool {
        FileManager.default.fileExists(atPath: Paths.cursorHooksJSON.path)
    }

    // MARK: - 安装

    @discardableResult
    static func install() throws -> String {
        let fm = FileManager.default
        let scriptURL = Paths.cursorHookScript
        try fm.createDirectory(at: scriptURL.deletingLastPathComponent(), withIntermediateDirectories: true)

        // 写脚本 + 可执行权限
        guard let data = scriptContent.data(using: .utf8) else {
            throw CursorHookError.io("脚本编码失败")
        }
        try atomicWrite(data, to: scriptURL)
        try fm.setAttributes([.posixPermissions: 0o755], ofItemAtPath: scriptURL.path)

        // 更新 hooks.json
        let root = readHooksRoot() ?? ["version": 1, "hooks": [String: Any]()]
        let updated = upsertEntry(into: root, scriptPath: scriptURL.path)
        try writeHooksRoot(updated)

        return "已安装 Cursor Hook"
    }

    // MARK: - 移除

    @discardableResult
    static func uninstall() throws -> String {
        let fm = FileManager.default

        if let root0 = readHooksRoot() {
            let updated = removeEntries(from: root0)
            try writeHooksRoot(updated)
        }

        // 删除脚本文件本身
        if fm.fileExists(atPath: Paths.cursorHookScript.path) {
            try? fm.removeItem(at: Paths.cursorHookScript)
        }

        return "已移除 Cursor Hook"
    }

    // MARK: - 在 Finder 中定位 hooks.json

    static func revealHooksJSON() {
        let path = Paths.cursorHooksJSON.path
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/open")
        process.arguments = ["-R", path]
        try? process.run()
    }

    // MARK: - 纯函数（可测试）

    /// 在 root 中插入或更新本应用的 preToolUse 条目，保留其他条目与未知字段
    static func upsertEntry(into root: [String: Any], scriptPath: String) -> [String: Any] {
        var root = root
        var hooks = (root["hooks"] as? [String: Any]) ?? [:]
        var preToolUse = (hooks["preToolUse"] as? [[String: Any]]) ?? []
        let entry: [String: Any] = ["command": scriptPath, "matcher": "Shell"]
        if let idx = preToolUse.firstIndex(where: { entryHasMarker($0) }) {
            preToolUse[idx] = entry
        } else {
            preToolUse.append(entry)
        }
        hooks["preToolUse"] = preToolUse
        root["hooks"] = hooks
        return root
    }

    /// 从 root 中移除本应用的 preToolUse 条目，保留其他条目
    static func removeEntries(from root: [String: Any]) -> [String: Any] {
        var root = root
        guard var hooks = root["hooks"] as? [String: Any] else { return root }
        guard var preToolUse = hooks["preToolUse"] as? [[String: Any]] else { return root }
        preToolUse.removeAll { entryHasMarker($0) }
        if preToolUse.isEmpty {
            hooks.removeValue(forKey: "preToolUse")
        } else {
            hooks["preToolUse"] = preToolUse
        }
        root["hooks"] = hooks
        return root
    }

    static func rootHasMarker(_ root: [String: Any]) -> Bool {
        guard let hooks = root["hooks"] as? [String: Any],
              let preToolUse = hooks["preToolUse"] as? [[String: Any]] else {
            return false
        }
        return preToolUse.contains { entryHasMarker($0) }
    }

    // MARK: - 私有工具

    private static func entryHasMarker(_ entry: [String: Any]) -> Bool {
        if let command = entry["command"] as? String {
            return command.contains(marker)
        }
        return false
    }

    private static func readPreToolUse() -> [[String: Any]]? {
        guard let root = readHooksRoot(),
              let hooks = root["hooks"] as? [String: Any],
              let preToolUse = hooks["preToolUse"] as? [[String: Any]] else {
            return nil
        }
        return preToolUse
    }

    private static func readHooksRoot() -> [String: Any]? {
        guard let data = try? Data(contentsOf: Paths.cursorHooksJSON),
              let obj = try? JSONSerialization.jsonObject(with: data),
              let dict = obj as? [String: Any] else {
            return nil
        }
        return dict
    }

    private static func writeHooksRoot(_ root: [String: Any]) throws {
        let data = try JSONSerialization.data(
            withJSONObject: root,
            options: [.prettyPrinted, .sortedKeys]
        )
        try atomicWrite(data, to: Paths.cursorHooksJSON)
    }

    private static func atomicWrite(_ data: Data, to url: URL) throws {
        do {
            try data.write(to: url, options: .atomic)
        } catch {
            throw CursorHookError.io("写入失败 \(url.lastPathComponent): \(error.localizedDescription)")
        }
    }
}
