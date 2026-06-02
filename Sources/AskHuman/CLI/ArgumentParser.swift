import Foundation

struct ParsedAskArgs {
    var message: String
    var options: [String]
    var isMarkdown: Bool
}

enum ArgumentError: Error, CustomStringConvertible {
    case missingMessage
    case multipleMessages
    case missingOptionValue(String)
    case unknownFlag(String)

    var description: String {
        switch self {
        case .missingMessage:
            return "缺少提问内容"
        case .multipleMessages:
            return "仅允许一个提问内容参数"
        case .missingOptionValue(let flag):
            return "\(flag) 选项缺少参数值"
        case .unknownFlag(let flag):
            return "未知选项: \(flag)"
        }
    }
}

enum ArgumentParser {
    /// 解析提问模式的参数（不含程序名，不含已被识别的子命令）
    static func parseAsk(_ args: [String]) throws -> ParsedAskArgs {
        var message: String?
        var options: [String] = []
        var isMarkdown = true

        var i = 0
        while i < args.count {
            let arg = args[i]
            switch arg {
            case "-o", "--option":
                guard i + 1 < args.count else {
                    throw ArgumentError.missingOptionValue(arg)
                }
                options.append(args[i + 1])
                i += 2
            case "--no-markdown":
                isMarkdown = false
                i += 1
            default:
                if arg.hasPrefix("-") {
                    throw ArgumentError.unknownFlag(arg)
                }
                if message != nil {
                    throw ArgumentError.multipleMessages
                }
                message = arg
                i += 1
            }
        }

        guard let message else {
            throw ArgumentError.missingMessage
        }

        return ParsedAskArgs(message: message, options: options, isMarkdown: isMarkdown)
    }
}
