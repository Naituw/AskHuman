import Foundation
import AHStore

extension MockBackend {
    /// Takes the month's traffic to 1.9 GB, close to the second GB, once the file at `path` exists (`-AHTrafficGate`).
    /// A UI test of the reminder that comes back at the next GB closes the first one, then makes the file: the test
    /// decides when the use grows, instead of racing a timer against a test runner that can be seconds late.
    public func setTrafficGate(path: String) {
        Task { [weak self] in
            let giveUp = Date().addingTimeInterval(180)
            while !FileManager.default.fileExists(atPath: path), Date() < giveUp {
                try? await Task.sleep(nanoseconds: 50_000_000)
            }
            guard FileManager.default.fileExists(atPath: path) else { return }
            await self?.growTraffic()
        }
    }

    func growTraffic() {
        let bytes = TrafficReminder.gigabyte / 10 * 19
        let month = state.traffic?.month ?? "2026-10"
        state.traffic = TrafficStats(
            month: month, sentBytes: bytes / 2, receivedBytes: bytes - bytes / 2, sentMessages: 80_000, receivedMessages: 150_000
        )
        publish()
    }
}
