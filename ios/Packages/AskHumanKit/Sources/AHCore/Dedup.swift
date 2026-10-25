import Foundation
import AHProtocol

/// Remembers recently seen message ids (at most `Wire.dedupWindowCount`, at most
/// `Wire.dedupWindowSecs` old) so a message delivered twice is handled once.
struct DedupWindow {
    private var order: [(id: String, at: Date)] = []
    private var seen: Set<String> = []

    /// `true` when `id` is new.
    mutating func insert(_ id: String, now: Date) -> Bool {
        while let first = order.first,
              order.count > Wire.dedupWindowCount || now.timeIntervalSince(first.at) > TimeInterval(Wire.dedupWindowSecs) {
            seen.remove(first.id)
            order.removeFirst()
        }
        guard seen.insert(id).inserted else { return false }
        order.append((id, now))
        return true
    }
}
