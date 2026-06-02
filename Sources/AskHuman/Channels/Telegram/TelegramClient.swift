import Foundation

enum TelegramError: Error, CustomStringConvertible {
    case emptyToken
    case emptyChatId
    case invalidChatId
    case invalidURL
    case api(String)
    case network(String)
    case badResponse

    var description: String {
        switch self {
        case .emptyToken: return "Bot Token 不能为空"
        case .emptyChatId: return "Chat ID 不能为空"
        case .invalidChatId: return "Chat ID 格式无效，请输入有效的数字 ID"
        case .invalidURL: return "无效的 API URL"
        case .api(let msg): return "Telegram API 错误: \(msg)"
        case .network(let msg): return "网络错误: \(msg)"
        case .badResponse: return "无法解析 Telegram 响应"
        }
    }
}

@MainActor
struct TelegramClient {
    let token: String
    let chatId: Int64
    let apiBaseUrl: String

    init(token: String, chatIdString: String, apiBaseUrl: String) throws {
        let trimmedToken = token.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedChat = chatIdString.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedToken.isEmpty else { throw TelegramError.emptyToken }
        guard !trimmedChat.isEmpty else { throw TelegramError.emptyChatId }
        if trimmedChat.hasPrefix("@") { throw TelegramError.invalidChatId }
        guard let id = Int64(trimmedChat) else { throw TelegramError.invalidChatId }
        self.token = trimmedToken
        self.chatId = id
        let base = apiBaseUrl.trimmingCharacters(in: .whitespacesAndNewlines)
        self.apiBaseUrl = base.isEmpty ? "https://api.telegram.org" : base
    }

    // MARK: - API

    /// 发送消息，返回 message_id
    @discardableResult
    func sendMessage(
        text: String,
        parseMode: String? = nil,
        replyMarkup: [String: Any]? = nil
    ) async throws -> Int {
        var params: [String: Any] = ["chat_id": chatId, "text": text]
        if let parseMode { params["parse_mode"] = parseMode }
        if let replyMarkup { params["reply_markup"] = replyMarkup }
        let result = try await request(method: "sendMessage", params: params)
        return (result["message_id"] as? Int) ?? 0
    }

    func getUpdates(offset: Int) async throws -> [[String: Any]] {
        let params: [String: Any] = ["offset": offset, "timeout": 0]
        let data = try await rawRequest(method: "getUpdates", params: params)
        guard let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw TelegramError.badResponse
        }
        if let ok = obj["ok"] as? Bool, ok, let result = obj["result"] as? [[String: Any]] {
            return result
        }
        throw TelegramError.api((obj["description"] as? String) ?? "getUpdates 失败")
    }

    func answerCallbackQuery(id: String) async {
        _ = try? await request(method: "answerCallbackQuery", params: ["callback_query_id": id])
    }

    func editMessageReplyMarkup(messageId: Int, replyMarkup: [String: Any]) async {
        let params: [String: Any] = [
            "chat_id": chatId,
            "message_id": messageId,
            "reply_markup": replyMarkup
        ]
        _ = try? await request(method: "editMessageReplyMarkup", params: params)
    }

    /// 发送测试消息验证配置
    func testConnection() async throws -> String {
        let text = "🤖 HumanInLoop 测试消息\n\n这是一条测试消息，表示 Telegram Bot 配置成功！"
        _ = try await sendMessage(text: text)
        return "测试消息发送成功！Telegram Bot 配置正确。"
    }

    // MARK: - 底层请求

    @discardableResult
    private func request(method: String, params: [String: Any]) async throws -> [String: Any] {
        let data = try await rawRequest(method: method, params: params)
        guard let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw TelegramError.badResponse
        }
        if let ok = obj["ok"] as? Bool, ok {
            return (obj["result"] as? [String: Any]) ?? [:]
        }
        throw TelegramError.api((obj["description"] as? String) ?? "\(method) 失败")
    }

    private func rawRequest(method: String, params: [String: Any]) async throws -> Data {
        guard let url = URL(string: "\(apiBaseUrl)/bot\(token)/\(method)") else {
            throw TelegramError.invalidURL
        }
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try JSONSerialization.data(withJSONObject: params)
        req.timeoutInterval = 30

        do {
            let (data, _) = try await URLSession.shared.data(for: req)
            return data
        } catch {
            throw TelegramError.network(error.localizedDescription)
        }
    }
}
