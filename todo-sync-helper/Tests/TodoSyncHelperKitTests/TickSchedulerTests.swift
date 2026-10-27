import XCTest

@testable import TodoSyncHelperKit

final class TickSchedulerTests: XCTestCase {
    private let timing = TickScheduler.Timing(fileDebounceMs: 1_000, settleDebounceMs: 2_000, pushDebounceMs: 200, minGapMs: 1_000)

    func testNothingPendingMeansNoRun() {
        let s = TickScheduler(timing: timing)
        XCTAssertNil(s.takeRun(now: 0))
        XCTAssertNil(s.nextDue(now: 0))
    }

    func testPollRunsImmediatelyAndPushAfterItsShortDebounce() {
        let s = TickScheduler(timing: timing)
        s.trigger(.poll, now: 100)
        XCTAssertEqual(s.takeRun(now: 100), .poll)

        let t = TickScheduler(timing: timing)
        t.trigger(.push, now: 100)
        XCTAssertNil(t.takeRun(now: 250))
        XCTAssertEqual(t.nextDue(now: 250), 300)
        XCTAssertEqual(t.takeRun(now: 300), .push)
    }

    func testFileEventsDebounceOnTheTrailingEdge() {
        let s = TickScheduler(timing: timing)
        s.trigger(.file, now: 0)
        s.trigger(.file, now: 600)
        s.trigger(.file, now: 1_200)  // each write pushes the deadline out
        XCTAssertNil(s.takeRun(now: 2_100))
        XCTAssertEqual(s.nextDue(now: 2_100), 2_200)
        XCTAssertEqual(s.takeRun(now: 2_200), .file)
    }

    func testSettleEventsKeepTheFirstDeadline() {
        let s = TickScheduler(timing: timing)
        s.trigger(.wake, now: 0)
        s.trigger(.wake, now: 1_500)  // a burst does not postpone the run
        XCTAssertEqual(s.takeRun(now: 2_000), .wake)
    }

    func testTheMostInformativeDueReasonNamesTheRun() {
        let s = TickScheduler(timing: timing)
        s.trigger(.poll, now: 0)
        s.trigger(.wake, now: 0)
        s.trigger(.push, now: 0)
        XCTAssertEqual(s.takeRun(now: 5_000), .push)
        // all three were covered by that one run
        s.finished(now: 5_100)
        XCTAssertNil(s.nextDue(now: 5_100))
    }

    func testTriggersDuringARunProduceExactlyOneFollowUp() {
        let s = TickScheduler(timing: timing)
        s.trigger(.poll, now: 0)
        XCTAssertEqual(s.takeRun(now: 0), .poll)
        XCTAssertTrue(s.running)
        s.trigger(.file, now: 100)
        s.trigger(.push, now: 150)
        XCTAssertNil(s.takeRun(now: 10_000), "never two ticks at once")
        XCTAssertNil(s.nextDue(now: 10_000), "the finish callback re-arms the timer")
        s.finished(now: 3_000)
        XCTAssertEqual(s.nextDue(now: 3_000), 4_000, "min gap after the end of the previous tick")
        XCTAssertNil(s.takeRun(now: 3_500))
        XCTAssertEqual(s.takeRun(now: 4_000), .push)
        s.finished(now: 4_100)
        XCTAssertNil(s.nextDue(now: 4_100))
    }

    func testAPendingFileEditIsCoveredByARunThatStartsFirst() {
        let s = TickScheduler(timing: timing)
        s.trigger(.file, now: 0)  // due at 1000
        s.trigger(.poll, now: 500)
        XCTAssertEqual(s.takeRun(now: 500), .poll)
        s.finished(now: 800)
        XCTAssertNil(s.nextDue(now: 800), "the poll tick already reads the edited file")
    }

    func testMinGapStopsAnEventStormFromSpinning() {
        let s = TickScheduler(timing: timing)
        var starts = 0
        var now: Int64 = 0
        for _ in 0..<50 {
            s.trigger(.poll, now: now)
            if s.takeRun(now: now) != nil {
                starts += 1
                s.finished(now: now + 10)
            }
            now += 100
        }
        // 50 events over 5 s with a 1 s gap: a handful of runs, never one per event.
        XCTAssertLessThanOrEqual(starts, 6)
        XCTAssertGreaterThanOrEqual(starts, 4)
    }

    func testClearDropsWhatIsPending() {
        let s = TickScheduler(timing: timing)
        s.trigger(.poll, now: 0)
        s.clear()
        XCTAssertNil(s.takeRun(now: 0))
    }
}

final class TickCoordinatorTests: XCTestCase {
    private let fast = TickScheduler.Timing(fileDebounceMs: 60, settleDebounceMs: 80, pushDebounceMs: 10, minGapMs: 40)

    private final class Recorder {
        private let lock = NSLock()
        private var reasons: [TickScheduler.Reason] = []
        private var concurrent = 0
        private(set) var maxConcurrent = 0
        var runMs: UInt32 = 30

        func run(_ reason: TickScheduler.Reason, done: @escaping (Int32?) -> Void) {
            lock.lock()
            reasons.append(reason)
            concurrent += 1
            maxConcurrent = max(maxConcurrent, concurrent)
            lock.unlock()
            DispatchQueue.global().asyncAfter(deadline: .now() + .milliseconds(Int(runMs))) { [self] in
                lock.lock()
                concurrent -= 1
                lock.unlock()
                done(0)
            }
        }

        var seen: [TickScheduler.Reason] {
            lock.lock()
            defer { lock.unlock() }
            return reasons
        }
    }

    private func wait(_ ms: Int) async {
        try? await Task.sleep(nanoseconds: UInt64(ms) * 1_000_000)
    }

    func testABurstOfFileEventsStartsOneTick() async {
        let recorder = Recorder()
        let coordinator = TickCoordinator(timing: fast) { recorder.run($0, done: $1) }
        for _ in 0..<10 {
            coordinator.trigger(.file)
            await wait(10)
        }
        await wait(400)
        XCTAssertEqual(recorder.seen, [.file])
        XCTAssertEqual(coordinator.snapshot().ticksRun, 1)
        XCTAssertEqual(coordinator.snapshot().lastReason, "file")
        XCTAssertEqual(coordinator.snapshot().lastExit, 0)
    }

    func testTicksNeverOverlapAndLateTriggersGetOneFollowUp() async {
        let recorder = Recorder()
        recorder.runMs = 120
        let coordinator = TickCoordinator(timing: fast) { recorder.run($0, done: $1) }
        coordinator.trigger(.poll)
        await wait(30)  // the first tick is running
        coordinator.trigger(.push)
        coordinator.trigger(.push)
        coordinator.trigger(.file)
        await wait(600)
        XCTAssertEqual(recorder.maxConcurrent, 1)
        XCTAssertEqual(recorder.seen, [.poll, .push])
    }

    func testDisabledCoordinatorIgnoresTriggers() async {
        let recorder = Recorder()
        let coordinator = TickCoordinator(timing: fast) { recorder.run($0, done: $1) }
        coordinator.setEnabled(false)
        coordinator.trigger(.push)
        await wait(150)
        XCTAssertEqual(recorder.seen, [])
        coordinator.setEnabled(true)
        coordinator.trigger(.push)
        await wait(150)
        XCTAssertEqual(recorder.seen, [.push])
    }

    func testARunnerThatCannotStartDoesNotJamTheQueue() async {
        var attempts = 0
        let lock = NSLock()
        let coordinator = TickCoordinator(timing: fast) { _, done in
            lock.lock()
            attempts += 1
            lock.unlock()
            done(nil)
        }
        coordinator.trigger(.poll)
        await wait(100)
        coordinator.trigger(.poll)
        await wait(200)
        lock.lock()
        let count = attempts
        lock.unlock()
        XCTAssertEqual(count, 2)
        XCTAssertNil(coordinator.snapshot().lastExit)
    }
}
