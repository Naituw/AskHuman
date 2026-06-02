import Foundation

enum OutputFormatter {
    static let cancelStatusText =
        "用户取消了操作，你必须重新询问用户是否确定要取消，直到用户给出明确答复"

    /// 取消路径输出
    static func cancelOutput() -> String {
        "[状态]\n\(cancelStatusText)"
    }

    /// 成功路径输出（图片已落盘，传入路径列表）
    static func sendOutput(selectedOptions: [String], userInput: String?, imagePaths: [String]) -> String {
        var sections: [String] = []

        if !selectedOptions.isEmpty {
            sections.append("[选择的选项]\n\(selectedOptions.joined(separator: ", "))")
        }

        if let input = userInput?.trimmingCharacters(in: .whitespacesAndNewlines), !input.isEmpty {
            sections.append("[用户输入]\n\(input)")
        }

        if !imagePaths.isEmpty {
            sections.append("[图片]\n\(imagePaths.joined(separator: "\n"))")
        }

        if sections.isEmpty {
            sections.append("[用户输入]\n用户确认继续")
        }

        return sections.joined(separator: "\n\n")
    }
}
