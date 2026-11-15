import Foundation

/// How often the Inbox says how much data this month has used: at every `rawValue` GB, or never.
///
/// The reminder knows nothing about the broker's plan. It only counts, so it stays true whatever the
/// plan is: the free tier pauses at the first GB, a paid one goes on.
public enum TrafficReminderStep: Int, CaseIterable, Sendable {
    case off = 0
    case gb1 = 1
    case gb2 = 2
    case gb5 = 5
    case gb10 = 10

    public static let standard = TrafficReminderStep.gb1
}

/// What the Inbox says about this month's data use: close to a multiple of the step, or past it.
public struct TrafficReminder: Equatable, Sendable {
    /// The multiple of the step the use is close to or past, in GB: 1, 2, 3 … for a 1 GB step, 2, 4, 6 … for a 2 GB one.
    public var gigabytes: Int
    /// The use has reached `gigabytes`. Otherwise it is only close: in the last fifth before it.
    public var reached: Bool

    public init(gigabytes: Int, reached: Bool) {
        self.gigabytes = gigabytes
        self.reached = reached
    }

    /// A GB is the broker's: 1024³ bytes.
    public static let gigabyte: UInt64 = 1 << 30

    /// The reminder for `bytes` used this month. Nil below the lead of the first multiple, and when the step is off.
    ///
    /// The last fifth before each multiple counts as close to it, so the first reminder of a 1 GB step comes at 80%,
    /// early enough to read before a free tier runs out. Between a multiple and the lead of the next one, the use has
    /// reached the multiple it passed.
    public static func due(bytes: UInt64, step: TrafficReminderStep) -> TrafficReminder? {
        guard step != .off else { return nil }
        let unit = UInt64(step.rawValue) * gigabyte
        let whole = Int(clamping: bytes / unit)
        // `unit - unit / 5` is the first whole byte at or past 80% of the step.
        if bytes % unit >= unit - unit / 5 {
            return TrafficReminder(gigabytes: (whole + 1) * step.rawValue, reached: false)
        }
        guard whole > 0 else { return nil }
        return TrafficReminder(gigabytes: whole * step.rawValue, reached: true)
    }
}

/// The highest reminder the reader closed, and in which month.
///
/// It hides that reminder and every lower one for the rest of the month. The next multiple is a new reminder: closing
/// one is how a reader says "I saw it", and the point of a step is that the next one comes back.
public struct ClosedTrafficReminder: Equatable, Sendable {
    public var month: String
    public var gigabytes: Int

    public init(month: String, gigabytes: Int) {
        self.month = month
        self.gigabytes = gigabytes
    }

    public func hides(_ reminder: TrafficReminder, inMonth current: String) -> Bool {
        month == current && reminder.gigabytes <= gigabytes
    }

    /// The form kept in the defaults: `2026-11|2`.
    public var stored: String { "\(month)|\(gigabytes)" }

    public init?(stored: String) {
        let parts = stored.split(separator: "|", omittingEmptySubsequences: false)
        guard parts.count == 2, !parts[0].isEmpty, let gigabytes = Int(parts[1]), gigabytes > 0 else { return nil }
        self.init(month: String(parts[0]), gigabytes: gigabytes)
    }
}
