import Foundation
import AHStore

// The Inbox's reminder about this month's data use (docs/specs/ios-traffic.md, "用量提醒"). It counts the traffic
// and says nothing about the broker's plan.

extension AppStore {
    /// This month's traffic as the reminder counts it: all known devices when the Macs have sent theirs, else this phone's.
    var trafficThisMonth: TrafficStats? { state.brokerTraffic?.stats ?? state.traffic }

    /// What the Inbox says about this month's data use now. Nil below the first step, with the step off, and while the
    /// reminder for the multiple the use is at has been closed.
    public var trafficReminder: TrafficReminder? {
        guard let stats = trafficThisMonth,
              let due = TrafficReminder.due(bytes: stats.brokerEstimateBytes, step: trafficReminderStep) else { return nil }
        if let closed = closedTrafficReminder, closed.hides(due, inMonth: stats.month) { return nil }
        return due
    }

    /// Closes the reminder on screen. It and the lower ones stay away until the month ends; the next multiple of the step
    /// brings a new one.
    public func closeTrafficReminder() {
        guard let stats = trafficThisMonth, let shown = trafficReminder else { return }
        closedTrafficReminder = ClosedTrafficReminder(month: stats.month, gigabytes: shown.gigabytes)
    }
}
