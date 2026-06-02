import Foundation

/// 一次提问请求
struct AskRequest {
    let id: String
    let message: String
    let predefinedOptions: [String]
    let isMarkdown: Bool

    init(message: String, predefinedOptions: [String], isMarkdown: Bool, id: String = UUID().uuidString) {
        self.id = id
        self.message = message
        self.predefinedOptions = predefinedOptions
        self.isMarkdown = isMarkdown
    }
}

/// 图片附件
struct ImageAttachment: Sendable {
    /// base64 数据（可带 data: 前缀，落盘时会清理）
    let data: String
    let mediaType: String
    let filename: String?
}

/// Channel 的终态动作
enum ChannelAction: Sendable {
    case send
    case cancel
}

/// 某个 Channel 给出的最终回答
struct ChannelResult: Sendable {
    let action: ChannelAction
    let selectedOptions: [String]
    let userInput: String?
    let images: [ImageAttachment]
    let sourceChannelId: String

    static func cancel(sourceChannelId: String) -> ChannelResult {
        ChannelResult(
            action: .cancel,
            selectedOptions: [],
            userInput: nil,
            images: [],
            sourceChannelId: sourceChannelId
        )
    }
}
