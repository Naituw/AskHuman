import XCTest
@testable import AHCore

/// What "Copy diagnostics" is made of (docs/specs/ios-developer-mode.md §12): a log that anything can write a line to, and
/// sections that read their state when the report is made.
@MainActor
final class DiagnosticsTests: XCTestCase {
    /// A clock the test moves by hand.
    private final class Clock: @unchecked Sendable {
        var date = Date(timeIntervalSince1970: 1_800_000_000)
        func advance(_ seconds: TimeInterval) { date.addTimeInterval(seconds) }
    }

    private func make() -> (log: Diagnostics, clock: Clock) {
        let clock = Clock()
        return (Diagnostics(now: { clock.date }, processStart: clock.date), clock)
    }

    // MARK: the log

    func testTheLogKeepsTheNewestLinesAndTheReportSaysHowManyWentAway() {
        let (log, _) = make()
        let total = Diagnostics.capacity + 25
        for index in 0..<total { log.note("test", "entry-\(index)-end") }

        let entries = log.entries()
        XCTAssertEqual(entries.count, Diagnostics.capacity)
        XCTAssertEqual(entries.first?.text, "entry-25-end", "the oldest 25 were pushed out")
        XCTAssertEqual(entries.last?.text, "entry-\(total - 1)-end")

        let report = log.report(eventLimit: 10)
        XCTAssertTrue(report.contains("(10 lines, oldest first; \(total - 10) older ones are gone;"))
        XCTAssertTrue(report.contains("entry-\(total - 1)-end"))
        XCTAssertFalse(report.contains("entry-\(total - 11)-end"), "only the last ten are in the report")
    }

    func testALineIsOneLineAndNotLong() {
        let (log, _) = make()
        log.note("test", "first\nsecond\r\nthird")
        log.note("test", String(repeating: "x", count: Diagnostics.lineLimit * 2))

        let entries = log.entries()
        XCTAssertEqual(entries[0].text, "first ⏎ second ⏎ third")
        XCTAssertEqual(entries[1].text.count, Diagnostics.lineLimit)
        XCTAssertTrue(entries[1].text.hasSuffix("…"))
    }

    func testEveryLineSaysHowLongAfterTheStartItWasWritten() {
        let (log, clock) = make()
        clock.advance(1.5)
        log.note("test", "a moment later")
        XCTAssertEqual(log.entries().first?.uptime ?? 0, 1.5, accuracy: 0.0001)
        XCTAssertTrue(log.report().contains("+1.500 "), "the report shows the seconds with milliseconds")
        XCTAssertTrue(log.report().contains("[test] a moment later"))
    }

    nonisolated private static func writeFromManyThreads(_ log: Diagnostics, count: Int) {
        DispatchQueue.concurrentPerform(iterations: count) { log.note("thread", "n\($0)") }
    }

    func testLinesWrittenFromManyThreadsAreAllKept() {
        let (log, _) = make()
        Self.writeFromManyThreads(log, count: 300)
        XCTAssertEqual(log.entries().count, 300)
        XCTAssertEqual(Set(log.entries().map(\.text)).count, 300)
    }

    func testTheSharedLogTakesLinesFromAnywhere() {
        let token = "token-" + UUID().uuidString
        Diagnostics.note("test", token)
        XCTAssertTrue(Diagnostics.shared.entries().contains { $0.category == "test" && $0.text == token })
    }

    // MARK: the sections

    func testSectionsComeInTheOrderTheyWereAddedAndAreReadWhenTheReportIsMade() throws {
        let (log, _) = make()
        var reading = "the first reading"
        log.register("Alpha") { reading }
        log.register("Beta") { "  \n" }
        reading = "the second reading"

        let report = log.report()
        let alpha = try XCTUnwrap(report.range(of: "## Alpha\nthe second reading"), "a section reads its state at the copy, not when it was added")
        let beta = try XCTUnwrap(report.range(of: "## Beta\n(nothing to say)"), "a section with nothing to say says so")
        XCTAssertLessThan(alpha.lowerBound, beta.lowerBound)
        XCTAssertFalse(report.contains("the first reading"))
        XCTAssertTrue(report.contains("## Recent events"), "the log comes last")
    }

    func testRegisteringATitleAgainReplacesItsSectionWhereItWas() throws {
        let (log, _) = make()
        log.register("Alpha") { "stale body" }
        log.register("Beta") { "beta body" }
        log.register("Alpha") { "fresh body" }

        let report = log.report()
        XCTAssertFalse(report.contains("stale body"))
        let alpha = try XCTUnwrap(report.range(of: "## Alpha\nfresh body"))
        let beta = try XCTUnwrap(report.range(of: "## Beta"))
        XCTAssertLessThan(alpha.lowerBound, beta.lowerBound, "the replaced section keeps its place")
        XCTAssertEqual(report.components(separatedBy: "## Alpha").count - 1, 1)
    }

    // MARK: the report

    func testTheReportSaysWhenItWasMadeAndHowLongTheProcessHasRun() {
        let (log, clock) = make()
        clock.advance(3725)
        let report = log.report()
        XCTAssertTrue(report.hasPrefix("AskHuman diagnostics\n"))
        XCTAssertTrue(report.contains("up 1h02m"), report)
        XCTAssertTrue(report.contains("## Recent events (0 lines"))
        XCTAssertTrue(report.contains("(none)"))
    }

    func testDurationsAreShortAndReadable() {
        XCTAssertEqual(Diagnostics.duration(5.9), "5s")
        XCTAssertEqual(Diagnostics.duration(65), "1m05s")
        XCTAssertEqual(Diagnostics.duration(3725), "1h02m")
        XCTAssertEqual(Diagnostics.duration(-3), "0s")
    }
}
