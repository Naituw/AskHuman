import Foundation
import AHProtocol
import AHCore

extension MockBackend {
    /// Makes every request take notes on its message, and every Mac say that it does, as the newest Macs do. For UI tests
    /// of the answer bar on the requests of the other scenarios, whose forms are longer than the ones of the
    /// `annotations` scenario.
    public func allowAnnotations() {
        for index in state.inbox.indices where state.inbox[index].view?.ask != nil {
            state.inbox[index].view?.ask?.annotatable = true
        }
        for index in state.macs.indices { state.macs[index].caps.annotations = true }
        publish()
    }
}
