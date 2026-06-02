import AppKit

@MainActor
enum AskFlow {
    /// 提问主流程：解析参数 -> 运行 Channel -> 输出结果 -> 退出
    static func run(_ args: [String]) -> Never {
        let parsed: ParsedAskArgs
        do {
            parsed = try ArgumentParser.parseAsk(args)
        } catch {
            FileHandle.standardError.write(Data("错误: \(error)\n\n".utf8))
            print(Help.helpText())
            exit(1)
        }

        let config = ConfigStore.load()
        let request = AskRequest(
            message: parsed.message,
            predefinedOptions: parsed.options,
            isMarkdown: parsed.isMarkdown
        )

        var channels: [InteractionChannel] = []
        if config.channels.popup.enabled {
            channels.append(PopupChannel(config: config))
        }
        if config.channels.telegram.enabled {
            channels.append(TelegramChannel(config: config.channels.telegram))
        }
        // 没有任何 Channel 启用时，兜底使用本地弹窗
        if channels.isEmpty {
            channels.append(PopupChannel(config: config))
        }

        let coordinator = ChannelCoordinator()
        let result = coordinator.run(request: request, channels: channels, theme: config.general.theme)

        switch result.action {
        case .cancel:
            print(OutputFormatter.cancelOutput())
            exit(0)
        case .send:
            let imagePaths: [String]
            do {
                imagePaths = try ImageWriter.save(result.images, requestId: request.id)
            } catch {
                FileHandle.standardError.write(Data("错误: \(error)\n".utf8))
                exit(1)
            }
            let output = OutputFormatter.sendOutput(
                selectedOptions: result.selectedOptions,
                userInput: result.userInput,
                imagePaths: imagePaths
            )
            print(output)
            exit(0)
        }
    }
}
