import Foundation
#if canImport(Darwin)
import Darwin
#endif

/// What the developer mode's "Copy diagnostics" puts on the pasteboard (docs/specs/ios-developer-mode.md §12): what
/// the app wrote down about itself while it ran, and the parts that look at its state at the moment the report is made.
///
/// There are two ways to add to it, and nothing else is needed to add a thing to look at:
///
/// - **The log.** `Diagnostics.note("category", "what happened")` writes one line into a ring of the last few hundred,
///   from any thread. It records all the time, not only while the developer mode is on: what is worth understanding has
///   usually happened already by the time anyone asks.
/// - **A section.** `Diagnostics.shared.register("Title") { "state, read now" }` adds a part to the report. The body runs on
///   the main actor, when the report is made, so it can look at anything the screens can; registering the same title
///   again replaces the section.
///
/// Neither takes what the user wrote or what an Agent said: names of types, counts, states, sizes and times only. The
/// text is pasted into chats, so nothing that identifies a request, a project or a Mac (their names, paths, ids) belongs
/// here either.
public final class Diagnostics: @unchecked Sendable {
    public static let shared = Diagnostics()

    /// One line of the log.
    public struct Entry: Equatable, Sendable {
        /// Seconds since the process started.
        public var uptime: TimeInterval
        public var at: Date
        public var category: String
        public var text: String
    }

    /// How many lines the log keeps. The oldest go first.
    public static let capacity = 500
    /// How long one line may be. A line says a state; a dump belongs in a section.
    public static let lineLimit = 360

    private struct Section {
        var title: String
        var body: @MainActor () -> String
    }

    private let lock = NSLock()
    private var log: [Entry] = []
    private var dropped = 0
    private var sections: [Section] = []
    private let now: @Sendable () -> Date
    private let processStart: Date

    /// `now` and `processStart` are for tests; the shared log reads the clock and asks the system when the process began.
    public init(now: @escaping @Sendable () -> Date = { Date() }, processStart: Date? = nil) {
        self.now = now
        self.processStart = processStart ?? Self.systemProcessStart() ?? now()
    }

    // MARK: the log

    /// Writes a line. Category is a short word for where it comes from (`app`, `scene`, `window`, `nav` …).
    public func note(_ category: String, _ text: String) {
        let at = now()
        let entry = Entry(
            uptime: at.timeIntervalSince(processStart), at: at, category: category,
            text: Self.oneLine(text)
        )
        lock.lock()
        log.append(entry)
        if log.count > Self.capacity {
            let extra = log.count - Self.capacity
            log.removeFirst(extra)
            dropped += extra
        }
        lock.unlock()
    }

    /// The same, for code that has no reason to hold on to the log. The text is only made when the line is written.
    public static func note(_ category: String, _ text: @autoclosure () -> String) {
        shared.note(category, text())
    }

    /// What the log holds now, oldest first.
    public func entries() -> [Entry] {
        lock.lock()
        defer { lock.unlock() }
        return log
    }

    // MARK: the sections

    /// Adds a part to the report (or replaces the one with this title). Parts come out in the order they were added.
    @MainActor
    public func register(_ title: String, body: @escaping @MainActor () -> String) {
        lock.lock()
        defer { lock.unlock() }
        if let index = sections.firstIndex(where: { $0.title == title }) {
            sections[index].body = body
        } else {
            sections.append(Section(title: title, body: body))
        }
    }

    // MARK: the report

    /// The text to copy: when and from what it was made, the sections, then the log (the last `eventLimit` lines).
    @MainActor
    public func report(eventLimit: Int = 250) -> String {
        let made = now()
        lock.lock()
        let parts = sections
        let kept = log
        let droppedCount = dropped
        lock.unlock()

        var out: [String] = []
        out.append("AskHuman diagnostics")
        out.append("made \(Self.clock(made, zone: true))   process started \(Self.clock(processStart, zone: false))   up \(Self.duration(made.timeIntervalSince(processStart)))")
        for part in parts {
            let body = part.body().trimmingCharacters(in: .whitespacesAndNewlines)
            out.append("")
            out.append("## \(part.title)")
            out.append(body.isEmpty ? "(nothing to say)" : body)
        }
        let shown = kept.suffix(max(0, eventLimit))
        out.append("")
        let older = droppedCount + (kept.count - shown.count)
        out.append("## Recent events (\(shown.count) lines, oldest first\(older > 0 ? "; \(older) older ones are gone" : ""); +seconds since the process started)")
        if shown.isEmpty { out.append("(none)") }
        for entry in shown {
            out.append("+\(String(format: "%.3f", entry.uptime)) \(Self.clock(entry.at, zone: false)) [\(entry.category)] \(entry.text)")
        }
        return out.joined(separator: "\n")
    }

    // MARK: pieces

    static func oneLine(_ text: String) -> String {
        var line = text.replacingOccurrences(of: "\r\n", with: " ⏎ ")
            .replacingOccurrences(of: "\n", with: " ⏎ ")
            .replacingOccurrences(of: "\r", with: " ⏎ ")
        if line.count > lineLimit { line = String(line.prefix(lineLimit - 1)) + "…" }
        return line
    }

    /// `14:03:09.512`, or with the date and the offset from UTC when `zone` is set: `2026-11-07 14:03:09.512 UTC+08:00`.
    static func clock(_ date: Date, zone: Bool) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = .current
        let parts = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second, .nanosecond], from: date)
        let millis = (parts.nanosecond ?? 0) / 1_000_000
        let time = String(format: "%02ld:%02ld:%02ld.%03ld", parts.hour ?? 0, parts.minute ?? 0, parts.second ?? 0, millis)
        guard zone else { return time }
        let day = String(format: "%04ld-%02ld-%02ld", parts.year ?? 0, parts.month ?? 0, parts.day ?? 0)
        let offset = TimeZone.current.secondsFromGMT(for: date)
        let offsetText = String(format: "%@%02ld:%02ld", offset < 0 ? "-" : "+", abs(offset) / 3600, abs(offset) % 3600 / 60)
        return "\(day) \(time) UTC\(offsetText)"
    }

    static func duration(_ seconds: TimeInterval) -> String {
        let total = max(0, Int(seconds))
        if total >= 3600 { return String(format: "%ldh%02ldm", total / 3600, total % 3600 / 60) }
        if total >= 60 { return String(format: "%ldm%02lds", total / 60, total % 60) }
        return "\(total)s"
    }

    /// When the process began, from the kernel (the log's own clock only starts when something first writes to it).
    private static func systemProcessStart() -> Date? {
        #if canImport(Darwin)
        var info = kinfo_proc()
        var size = MemoryLayout<kinfo_proc>.stride
        var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, getpid()]
        guard sysctl(&mib, UInt32(mib.count), &info, &size, nil, 0) == 0, size > 0 else { return nil }
        let started = info.kp_proc.p_un.__p_starttime
        guard started.tv_sec > 0 else { return nil }
        return Date(timeIntervalSince1970: TimeInterval(started.tv_sec) + TimeInterval(started.tv_usec) / 1_000_000)
        #else
        return nil
        #endif
    }
}
