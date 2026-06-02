import AppKit

/// 并行启动所有启用的 Channel，采用首个终态结果（抢答）。
@MainActor
final class ChannelCoordinator {
    private var channels: [InteractionChannel] = []
    private var finished = false
    private var result: ChannelResult?

    /// 阻塞运行，返回首个 Channel 的终态结果。
    func run(request: AskRequest, channels: [InteractionChannel], theme: ThemeMode) -> ChannelResult {
        self.channels = channels

        AppBootstrap.run(theme: theme) { [weak self] in
            guard let self else { return }
            for channel in self.channels {
                channel.start(request: request) { [weak self] res in
                    self?.handleResult(res)
                }
            }
        }

        return result ?? .cancel(sourceChannelId: "none")
    }

    private func handleResult(_ res: ChannelResult) {
        guard !finished else { return }
        finished = true
        result = res
        for channel in channels where channel.id != res.sourceChannelId {
            channel.cancelByOtherChannel()
        }
        AppBootstrap.stop()
    }
}
