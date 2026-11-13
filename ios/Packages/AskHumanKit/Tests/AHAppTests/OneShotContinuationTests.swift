import XCTest
@testable import AHCore

final class OneShotContinuationTests: XCTestCase {
    private final class Tally: @unchecked Sendable {
        private let lock = NSLock()
        private var count = 0
        func increment() { lock.lock(); count += 1; lock.unlock() }
        var value: Int { lock.lock(); defer { lock.unlock() }; return count }
    }

    func testOnlyTheFirstResultIsDelivered() async throws {
        let value: Int = try await withCheckedThrowingContinuation { continuation in
            let once = OneShotContinuation(continuation)
            XCTAssertTrue(once.resume(with: .success(1)))
            XCTAssertFalse(once.resume(with: .success(2)))
            XCTAssertFalse(once.resume(with: .failure(CancellationError())))
        }
        XCTAssertEqual(value, 1)
    }

    func testACancellationThatComesFirstIsNotFollowedByTheValue() async {
        do {
            let _: Int = try await withCheckedThrowingContinuation { continuation in
                let once = OneShotContinuation(continuation)
                once.resume(with: .failure(CancellationError()))
                once.resume(with: .success(3))
            }
            XCTFail("the cancellation came first")
        } catch {
            XCTAssertTrue(error is CancellationError)
        }
    }

    func testCallbacksThatRaceDeliverExactlyOnce() async throws {
        for _ in 0..<100 {
            let wins = Tally()
            let group = DispatchGroup()
            let value: Int = try await withCheckedThrowingContinuation { continuation in
                let once = OneShotContinuation(continuation)
                for index in 0..<8 {
                    group.enter()
                    DispatchQueue.global().async {
                        if once.resume(with: .success(index)) { wins.increment() }
                        group.leave()
                    }
                }
            }
            group.wait()
            XCTAssertTrue((0..<8).contains(value))
            XCTAssertEqual(wins.value, 1)
        }
    }
}
