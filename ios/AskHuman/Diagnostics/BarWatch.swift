import UIKit
import AHCore

/// Looks at how the screen is laid out and what the bars look like once a second, and again right after what moves them
/// (the app coming back, a window appearing), and writes a line to the log for each thing that differs from the last time.
///
/// The report shows the state at the moment of the copy; this is how it got there. It reads and never touches: the same
/// reading `UIKitReport` makes for the report, only the changes are kept.
@MainActor
final class BarWatch {
    private var timer: Timer?
    private var last: [String: String] = [:]
    // What looking costs, so that a report from a real phone says whether the watching itself is a burden: the processor
    // time of the look (a busy machine takes the thread away in the middle of one, which the clock on the wall counts) and
    // the longest the main thread was held up by one.
    private var samples = 0
    private var cpuSpent: TimeInterval = 0
    private var cpuSlowest: TimeInterval = 0
    private var wallSlowest: TimeInterval = 0

    func start() {
        guard timer == nil else { return }
        let timer = Timer(timeInterval: 1, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.sample() }
        }
        timer.tolerance = 0.3
        // Not in the common modes: while a finger is on the screen (scrolling, dragging a page back) the main thread is the
        // user's, and the looking waits.
        RunLoop.main.add(timer, forMode: .default)
        self.timer = timer
        sample()
    }

    /// Looks now, and again after each of `delays` (seconds): what a resume does is not all done at once.
    func sample(soonAfter delays: [Double]) {
        sample()
        for delay in delays {
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
                MainActor.assumeIsolated { self?.sample() }
            }
        }
    }

    func sample() {
        let wallBegan = ProcessInfo.processInfo.systemUptime
        let cpuBegan = Self.cpuTime()
        for (key, value) in UIKitReport.watched() where last[key] != value {
            last[key] = value
            Diagnostics.note("watch", "\(key): \(value)")
        }
        let cpu = Self.cpuTime() - cpuBegan
        // Only a look that stands out is written: the first one that meets a kind of window or view costs more than the rest.
        if cpu > 0.02 { Diagnostics.note("watch", "this look took \(Int((cpu * 1000).rounded())) ms of processor time") }
        samples += 1
        cpuSpent += cpu
        cpuSlowest = max(cpuSlowest, cpu)
        wallSlowest = max(wallSlowest, ProcessInfo.processInfo.systemUptime - wallBegan)
    }

    /// Seconds of processor time this thread has used.
    private static func cpuTime() -> TimeInterval {
        TimeInterval(clock_gettime_nsec_np(CLOCK_THREAD_CPUTIME_ID)) / 1_000_000_000
    }

    /// For the report: how often it looked and how long a look took.
    func cost() -> String {
        guard samples > 0 else { return "no look yet" }
        let milliseconds = { (seconds: TimeInterval) in String(format: "%.2f ms", seconds * 1000) }
        return "bar watch: \(samples) looks, \(milliseconds(cpuSpent / Double(samples))) of processor time on average, "
            + "\(milliseconds(cpuSlowest)) at most; the main thread was held up \(milliseconds(wallSlowest)) at most by one"
    }
}
