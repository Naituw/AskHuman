import Foundation
import AHProtocol
import AHCore

extension MockBackend {
    /// Makes every Mac say that it leaves the live activity of a working Agent nobody watches (its tool, step count and
    /// last activity time) out of the Agents list, as the newest Macs do (`caps.leanAgents`; docs/specs/
    /// ios-traffic-reduction.md §3.2). The phone then shows such a row without them.
    public func offerLeanAgents() {
        for index in state.macs.indices { state.macs[index].caps.leanAgents = true }
        publish()
    }
}
