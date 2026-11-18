import XCTest
@testable import AHProtocol
@testable import AHStore
@testable import AHTransport
@testable import AHCore

/// The phone's side of docs/specs/ios-traffic-reduction.md: it asks for compressed lists, reads them like any other
/// message, and a stream that cannot be inflated costs it one message and nothing else.
final class TrafficReductionTests: XCTestCase {
    private func sessions(_ count: Int, titlePrefix: String = "Refactor the module") -> [String: AgentView] {
        var views: [String: AgentView] = [:]
        for index in 0..<count {
            let key = "claude:s\(index)"
            views[key] = sampleAgent(key, title: "\(titlePrefix) \(index)", state: index % 2 == 0 ? "working" : "idle")
        }
        return views
    }

    func testListsThatArriveCompressedAreReadLikeAnyOther() async throws {
        let h = Harness()
        h.mac.agents = sessions(14)
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 14 }
        XCTAssertGreaterThan(h.mac.compressedSent, 0, "the snapshot of fourteen sessions was worth compressing")
        let titles = Set(await h.state().agents.map(\.view.title))
        XCTAssertEqual(titles, Set((0..<14).map { "Refactor the module \($0)" }))

        // A delta that is big enough is compressed too, and applied in order.
        let before = h.mac.compressedSent
        var renamed: [AgentView] = []
        for index in 0..<14 { renamed.append(sampleAgent("claude:s\(index)", title: "A new title of the module \(index)")) }
        await h.mac.changeAgents(upsert: renamed)
        await eventually("the delta") { await h.state().agents.allSatisfy { $0.view.title.hasPrefix("A new title") } }
        XCTAssertGreaterThan(h.mac.compressedSent, before)
    }

    func testAMacThatDoesNotCompressIsReadAsBefore() async throws {
        let h = Harness()
        h.mac.compressesLists = false
        h.mac.agents = sessions(14)
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 14 }
        XCTAssertEqual(h.mac.compressedSent, 0)
        XCTAssertEqual(h.mac.hello(of: h.phoneId)?.compression, true, "the phone asked all the same")
    }

    func testAnUninflatableStreamCostsTheMessageAndNothingElse() async throws {
        let h = Harness()
        h.mac.agents = sessions(2)
        await h.pair()
        await eventually("the Agent list") { await h.state().agents.count == 2 }
        let phone = h.phoneId

        // Properly encrypted, and still a bomb, a stream cut short, and noise: dropped one by one.
        await h.mac.sendDeflated(AHEnvelope.compress(Data(count: 16 * 1024 * 1024)), to: phone)
        let packed = AHEnvelope.compress(Data(String(repeating: "a stable title, ", count: 200).utf8))
        await h.mac.sendDeflated(packed.prefix(packed.count / 2), to: phone)
        await h.mac.sendDeflated(Data(repeating: 0xFF, count: 64), to: phone)
        try await Task.sleep(nanoseconds: 100_000_000)

        // The stream goes on where it was: the next delta applies, and nobody asked for a new snapshot.
        let resyncsBefore = h.mac.received(Wire.T.resync).count
        await h.mac.changeAgents(upsert: [sampleAgent("pi:s9", title: "Still reachable")])
        await eventually("the next delta") { await h.state().agents.contains { $0.agentKey == "pi:s9" } }
        XCTAssertEqual(h.mac.received(Wire.T.resync).count, resyncsBefore)
        let link = await h.state().macs.first?.link
        XCTAssertEqual(link, .online)
    }
}
