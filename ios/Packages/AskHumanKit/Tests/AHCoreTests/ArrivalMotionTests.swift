import XCTest
import AHCore

final class ArrivalMotionTests: XCTestCase {
    private struct Sample: Decodable {
        var elapsed: Double
        var origin: ArrivalMotion.Point
        var target: ArrivalMotion.Point
        var frame: ArrivalMotion.Frame
    }

    func testTheApprovedMacFramesMatchForUpperAndLowerDestinations() throws {
        var url = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { url.deleteLastPathComponent() }
        let samples = try JSONDecoder().decode([Sample].self, from: Data(contentsOf: url.appendingPathComponent("Fixtures/arrival-motion.json")))
        XCTAssertEqual(samples.count, 51)
        for sample in samples {
            let actual = ArrivalMotion.frame(at: sample.elapsed, origin: sample.origin, target: sample.target)
            let expected = sample.frame
            let values = [(actual.x, expected.x), (actual.y, expected.y), (actual.scale, expected.scale),
                          (actual.stretchX, expected.stretchX), (actual.stretchY, expected.stretchY),
                          (actual.opacity, expected.opacity), (actual.angle, expected.angle), (actual.morph, expected.morph),
                          (actual.plateOpacity, expected.plateOpacity), (actual.haloOpacity, expected.haloOpacity)]
            for (value, reference) in values {
                XCTAssertEqual(value, reference, accuracy: 0.000_001, "time \(sample.elapsed), target \(sample.target)")
            }
            XCTAssertEqual(actual.done, expected.done)
        }
    }

    func testFlightStartsContinuouslyAndEndsExactlyAtTheDestination() {
        let origin = ArrivalMotion.Point(x: 210, y: 330), target = ArrivalMotion.Point(x: 180, y: 122)
        let before = ArrivalMotion.frame(at: ArrivalMotion.flightAt - 0.1, origin: origin, target: target)
        let after = ArrivalMotion.frame(at: ArrivalMotion.flightAt + 0.1, origin: origin, target: target)
        XCTAssertEqual(before.x, after.x, accuracy: 0.01)
        XCTAssertEqual(before.y, after.y, accuracy: 0.1)
        let landed = ArrivalMotion.frame(at: ArrivalMotion.landedAt, origin: origin, target: target)
        XCTAssertEqual(landed.x, target.x, accuracy: 0.000_001)
        XCTAssertEqual(landed.y, target.y, accuracy: 0.000_001)
        XCTAssertTrue(landed.done)
        XCTAssertEqual(landed.morph, 1)
        XCTAssertEqual(landed.plateOpacity, 0)
    }
}
