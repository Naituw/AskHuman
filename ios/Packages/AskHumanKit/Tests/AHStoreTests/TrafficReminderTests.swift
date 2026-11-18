import XCTest
@testable import AHStore

final class TrafficReminderTests: XCTestCase {
    private let gb = TrafficReminder.gigabyte

    /// The first whole byte that counts as close to `gigabytes` GB for a step of `step` GB: 80% of the way from the
    /// multiple before it.
    private func lead(_ gigabytes: UInt64, step: UInt64 = 1) -> UInt64 { gigabytes * gb - step * gb / 5 }

    private func due(_ bytes: UInt64, _ step: TrafficReminderStep = .gb1) -> TrafficReminder? {
        TrafficReminder.due(bytes: bytes, step: step)
    }

    func testNothingIsDueUntilTheLastFifthBeforeTheFirstStep() {
        XCTAssertNil(due(0))
        XCTAssertNil(due(gb / 2))
        XCTAssertNil(due(lead(1) - 1), "one byte short of 80%")
    }

    func testTheLastFifthBeforeAStepIsCloseToIt() {
        XCTAssertEqual(due(lead(1)), TrafficReminder(gigabytes: 1, reached: false), "80% is the first byte that counts")
        XCTAssertEqual(due(gb - 1), TrafficReminder(gigabytes: 1, reached: false))
        // The 940 MB of the quota scenario, which the Inbox shows as close to the first GB.
        XCTAssertEqual(due(940_000_000), TrafficReminder(gigabytes: 1, reached: false))
    }

    func testPastAStepTheUseHasReachedIt() {
        XCTAssertEqual(due(gb), TrafficReminder(gigabytes: 1, reached: true))
        XCTAssertEqual(due(gb + gb / 2), TrafficReminder(gigabytes: 1, reached: true))
        XCTAssertEqual(due(lead(2) - 1), TrafficReminder(gigabytes: 1, reached: true), "still the first GB, one byte before the next lead")
    }

    func testEveryGigabyteBringsItsOwnReminder() {
        XCTAssertEqual(due(lead(2)), TrafficReminder(gigabytes: 2, reached: false))
        XCTAssertEqual(due(2 * gb), TrafficReminder(gigabytes: 2, reached: true))
        XCTAssertEqual(due(lead(3)), TrafficReminder(gigabytes: 3, reached: false))
        XCTAssertEqual(due(7 * gb + gb / 3), TrafficReminder(gigabytes: 7, reached: true))
    }

    func testALargerStepKeepsTheSameLeadOfAFifthOfItself() {
        XCTAssertNil(due(lead(2, step: 2) - 1, .gb2))
        XCTAssertEqual(due(lead(2, step: 2), .gb2), TrafficReminder(gigabytes: 2, reached: false), "1.6 GB")
        XCTAssertEqual(due(2 * gb, .gb2), TrafficReminder(gigabytes: 2, reached: true))
        XCTAssertEqual(due(3 * gb, .gb2), TrafficReminder(gigabytes: 2, reached: true), "between 2 and the lead of 4, it has reached 2")
        XCTAssertEqual(due(lead(4, step: 2), .gb2), TrafficReminder(gigabytes: 4, reached: false), "3.6 GB")

        XCTAssertNil(due(4 * gb - 1, .gb5), "80% of 5 GB is 4 GB")
        XCTAssertEqual(due(4 * gb, .gb5), TrafficReminder(gigabytes: 5, reached: false))
        XCTAssertEqual(due(5 * gb, .gb5), TrafficReminder(gigabytes: 5, reached: true))
        XCTAssertEqual(due(9 * gb, .gb5), TrafficReminder(gigabytes: 10, reached: false))

        XCTAssertNil(due(8 * gb - 1, .gb10))
        XCTAssertEqual(due(8 * gb, .gb10), TrafficReminder(gigabytes: 10, reached: false))
        XCTAssertEqual(due(25 * gb, .gb10), TrafficReminder(gigabytes: 20, reached: true))
    }

    func testOffNeverReminds() {
        XCTAssertNil(due(0, .off))
        XCTAssertNil(due(50 * gb, .off))
    }

    func testAnAbsurdUseDoesNotOverflow() {
        let reminder = due(.max)
        XCTAssertNotNil(reminder)
        XCTAssertGreaterThan(reminder?.gigabytes ?? 0, 1_000_000_000)
    }

    func testAClosedReminderHidesItselfAndTheLowerOnesOfItsMonthOnly() {
        let closed = ClosedTrafficReminder(month: "2026-11", gigabytes: 1)
        let close = TrafficReminder(gigabytes: 1, reached: false)
        let past = TrafficReminder(gigabytes: 1, reached: true)
        XCTAssertTrue(closed.hides(close, inMonth: "2026-11"))
        XCTAssertTrue(closed.hides(past, inMonth: "2026-11"), "past the GB it was closed for, it stays closed")
        XCTAssertFalse(closed.hides(TrafficReminder(gigabytes: 2, reached: false), inMonth: "2026-11"), "the next GB comes back")
        XCTAssertFalse(closed.hides(close, inMonth: "2026-12"), "a new month starts over")
        XCTAssertTrue(ClosedTrafficReminder(month: "2026-11", gigabytes: 4).hides(TrafficReminder(gigabytes: 2, reached: true), inMonth: "2026-11"))
    }

    func testAClosedReminderIsKeptAsText() {
        let closed = ClosedTrafficReminder(month: "2026-11", gigabytes: 2)
        XCTAssertEqual(closed.stored, "2026-11|2")
        XCTAssertEqual(ClosedTrafficReminder(stored: closed.stored), closed)
        for broken in ["", "2026-11", "|2", "2026-11|", "2026-11|0", "2026-11|-1", "2026-11|x", "2026-11|2|3"] {
            XCTAssertNil(ClosedTrafficReminder(stored: broken), broken)
        }
    }
}
