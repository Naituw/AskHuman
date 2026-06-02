import Foundation

/// Telegram Channel：发送提问 + 长轮询接收回复（与原版行为一致，不接收图片）
@MainActor
final class TelegramChannel: InteractionChannel {
    let id = "telegram"

    private let config: TelegramChannelConfig
    private var client: TelegramClient?
    private var pollTask: Task<Void, Never>?

    private var predefinedOptions: [String] = []
    private var selectedOptions: [String] = []
    private var userInput: String = ""
    private var optionsMessageId: Int = 0
    private var operationMessageId: Int = 0
    private var finished = false

    private let sendButtonText = "↗️发送"

    init(config: TelegramChannelConfig) {
        self.config = config
    }

    func start(request: AskRequest, completion: @escaping (ChannelResult) -> Void) {
        guard let client = try? TelegramClient(
            token: config.botToken,
            chatIdString: config.chatId,
            apiBaseUrl: config.apiBaseUrl
        ) else {
            FileHandle.standardError.write(Data("警告: Telegram 配置无效，已跳过该 Channel\n".utf8))
            return
        }
        self.client = client
        self.predefinedOptions = request.predefinedOptions

        pollTask = Task { [weak self] in
            await self?.runSession(request: request, client: client, completion: completion)
        }
    }

    func cancelByOtherChannel() {
        pollTask?.cancel()
        pollTask = nil
    }

    // MARK: - 会话

    private func runSession(
        request: AskRequest,
        client: TelegramClient,
        completion: @escaping (ChannelResult) -> Void
    ) async {
        // 1. 发送选项消息
        let processed = request.isMarkdown
            ? TelegramMarkdown.process(request.message)
            : request.message
        let inlineMarkup = request.predefinedOptions.isEmpty
            ? nil
            : inlineKeyboard(options: request.predefinedOptions, selected: [])
        do {
            optionsMessageId = try await client.sendMessage(
                text: processed,
                parseMode: request.isMarkdown ? "MarkdownV2" : nil,
                replyMarkup: inlineMarkup
            )
        } catch {
            // MarkdownV2 转义失败时回退为纯文本
            optionsMessageId = (try? await client.sendMessage(
                text: request.message,
                parseMode: nil,
                replyMarkup: inlineMarkup
            )) ?? 0
        }

        // 2. 发送操作消息（含「发送」按钮）
        operationMessageId = (try? await client.sendMessage(
            text: "在键盘上点「发送」完成回复，或直接回复文字补充说明",
            parseMode: nil,
            replyMarkup: replyKeyboard()
        )) ?? 0

        // 3. 长轮询
        var offset = 0
        while !Task.isCancelled {
            do {
                let updates = try await client.getUpdates(offset: offset)
                for update in updates {
                    if let updateId = update["update_id"] as? Int {
                        offset = updateId + 1
                    }
                    if await handleUpdate(update, client: client, completion: completion) {
                        return
                    }
                }
            } catch {
                try? await Task.sleep(nanoseconds: 5_000_000_000)
                continue
            }
            try? await Task.sleep(nanoseconds: 1_000_000_000)
        }
    }

    /// 处理一条更新，返回 true 表示已终结（发送）
    private func handleUpdate(
        _ update: [String: Any],
        client: TelegramClient,
        completion: @escaping (ChannelResult) -> Void
    ) async -> Bool {
        // callback_query：切换选项
        if let cb = update["callback_query"] as? [String: Any] {
            if let chatOK = callbackChatMatches(cb), !chatOK { return false }
            if let data = cb["data"] as? String, data.hasPrefix("toggle:") {
                let option = String(data.dropFirst("toggle:".count))
                toggleOption(option)
                await client.editMessageReplyMarkup(
                    messageId: optionsMessageId,
                    replyMarkup: inlineKeyboard(options: predefinedOptions, selected: selectedOptions)
                )
            }
            if let cbId = cb["id"] as? String {
                await client.answerCallbackQuery(id: cbId)
            }
            return false
        }

        // message：文本回复 / 发送
        if let message = update["message"] as? [String: Any] {
            guard messageChatMatches(message, client: client) else { return false }
            if let msgId = message["message_id"] as? Int, msgId <= operationMessageId {
                return false
            }
            if let text = message["text"] as? String {
                if text == sendButtonText {
                    resolveIfNeeded(completion: completion)
                    return true
                } else {
                    userInput = text
                }
            }
        }
        return false
    }

    private func resolveIfNeeded(completion: @escaping (ChannelResult) -> Void) {
        guard !finished else { return }
        finished = true
        completion(ChannelResult(
            action: .send,
            selectedOptions: selectedOptions,
            userInput: userInput.isEmpty ? nil : userInput,
            images: [],
            sourceChannelId: id
        ))
    }

    private func toggleOption(_ option: String) {
        if let idx = selectedOptions.firstIndex(of: option) {
            selectedOptions.remove(at: idx)
        } else {
            selectedOptions.append(option)
        }
    }

    // MARK: - 键盘构造

    private func inlineKeyboard(options: [String], selected: [String]) -> [String: Any] {
        var rows: [[[String: Any]]] = []
        var i = 0
        while i < options.count {
            var row: [[String: Any]] = []
            for option in options[i..<min(i + 2, options.count)] {
                let text = selected.contains(option) ? "✅ \(option)" : option
                row.append(["text": text, "callback_data": "toggle:\(option)"])
            }
            rows.append(row)
            i += 2
        }
        return ["inline_keyboard": rows]
    }

    private func replyKeyboard() -> [String: Any] {
        [
            "keyboard": [[["text": sendButtonText]]],
            "resize_keyboard": true,
            "one_time_keyboard": true
        ]
    }

    // MARK: - chat 过滤

    private func callbackChatMatches(_ cb: [String: Any]) -> Bool? {
        guard let message = cb["message"] as? [String: Any],
              let chat = message["chat"] as? [String: Any],
              let chatId = chat["id"] as? Int64 ?? (chat["id"] as? Int).map(Int64.init) else {
            return nil
        }
        return chatId == client?.chatId
    }

    private func messageChatMatches(_ message: [String: Any], client: TelegramClient) -> Bool {
        guard let chat = message["chat"] as? [String: Any],
              let chatId = chat["id"] as? Int64 ?? (chat["id"] as? Int).map(Int64.init) else {
            return false
        }
        return chatId == client.chatId
    }
}
