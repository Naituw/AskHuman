import Foundation

/// 通信 Channel 抽象。
/// 每个 Channel 在用户给出最终回答（发送/取消）时回调一次 `completion`。
@MainActor
protocol InteractionChannel: AnyObject {
    var id: String { get }

    /// 发起本 Channel 的询问
    func start(request: AskRequest, completion: @escaping (ChannelResult) -> Void)

    /// 其他 Channel 已抢答，收尾本 Channel（关闭窗口 / 停止轮询），不再回调结果
    func cancelByOtherChannel()
}
