#if DEBUG
import UIKit

/// Prototype aid (docs/plans/ios-session-history.md, R1): the page's motion written to a file, a line for every frame in
/// which something changed, so that a run can be read back and the pull's hold and arrival measured frame by frame.
///
/// Only with `-AHPullProbe <path>`, and not in a release build.
@MainActor
final class PullProbe {
    static let shared: PullProbe? = {
        guard let path = UserDefaults.standard.string(forKey: "AHPullProbe"), !path.isEmpty else { return nil }
        return PullProbe(path: path)
    }()

    private let handle: FileHandle
    private var observer: CFRunLoopObserver?
    private var sample: (() -> String)?
    private var last = ""

    private init?(path: String) {
        FileManager.default.createFile(atPath: path, contents: nil)
        guard let handle = FileHandle(forWritingAtPath: path) else { return nil }
        self.handle = handle
    }

    /// Starts (or replaces) the sampling: `sample` is asked for a line after every frame, and a line that differs from the
    /// last is written.
    func run(_ sample: @escaping () -> String) {
        self.sample = sample
        guard observer == nil else { return }
        // After Core Animation's own commit (order 2_000_000): what is read is what that frame shows.
        let observer = CFRunLoopObserverCreateWithHandler(nil, CFRunLoopActivity.beforeWaiting.rawValue, true, 2_100_000) { [weak self] _, _ in
            MainActor.assumeIsolated { self?.tick() }
        }
        CFRunLoopAddObserver(CFRunLoopGetMain(), observer, .commonModes)
        self.observer = observer
    }

    /// A line of its own, for what happens between frames (the hold begins, the hold is let go).
    func note(_ text: String) {
        write("# " + text)
    }

    private func tick() {
        guard let line = sample?(), !line.isEmpty, line != last else { return }
        last = line
        write(line)
    }

    private func write(_ line: String) {
        // The system's clock, which a UI test in its own process reads the same: its own moments can be set against the lines.
        let stamp = String(format: "%.3f ", CACurrentMediaTime())
        handle.write(Data((stamp + line + "\n").utf8))
    }
}
#endif
