import Foundation
import CryptoKit
import AHProtocol
import AHStore

// Push registration (plan §7): the device token goes to the relay once, the relay returns an opaque
// handle, and the handle goes to every Mac over the encrypted channel.

extension PhoneEngine {
    /// Renew the handle when less than this is left of its lifetime.
    static let handleRenewWindow: TimeInterval = 30 * 24 * 3600

    public func updatePushToken(_ token: Data?) async {
        deviceToken = token
        if token == nil {
            pushStatus = .noToken
        } else {
            await registerPushIfNeeded()
            for id in macs.keys.sorted() where macs[id]?.sync == .synced { await sendPushHandle(to: id) }
        }
        publish()
    }

    func tokenDigest(_ token: Data) -> String {
        SHA256.hash(data: token).map { String(format: "%02x", $0) }.joined()
    }

    /// Makes sure `vault.push` holds a fresh handle for the current token.
    func registerPushIfNeeded(force: Bool = false) async {
        guard let token = deviceToken else {
            pushStatus = .noToken
            return
        }
        let relayUrl = (activeSettings?.relayUrl ?? "").trimmingCharacters(in: .whitespaces)
        guard !relayUrl.isEmpty else {
            pushStatus = .noRelay
            return
        }
        let digest = tokenDigest(token)
        let now = clock.now()
        if !force, let current = vault.push, current.tokenDigest == digest, current.env == config.pushEnv,
           Date(timeIntervalSince1970: TimeInterval(current.expiresAt)).timeIntervalSince(now) > Self.handleRenewWindow {
            pushStatus = .registered(expiresAt: Date(timeIntervalSince1970: TimeInterval(current.expiresAt)))
            return
        }
        pushStatus = .registering
        publish()
        do {
            let handle = try await relay.register(relayUrl: relayUrl, deviceToken: token, env: config.pushEnv, bundleId: config.bundleId)
            vault.push = PushRegistration(handle: handle.handle, env: config.pushEnv, expiresAt: handle.expiresAt, tokenDigest: digest)
            pushStatus = .registered(expiresAt: Date(timeIntervalSince1970: TimeInterval(handle.expiresAt)))
            pushSent = [:]
        } catch {
            pushStatus = .failed((error as? LocalizedError)?.errorDescription ?? "\(error)")
        }
    }

    /// Gives a Mac the current handle (once per handle and session).
    func sendPushHandle(to macId: String) async {
        guard let registration = vault.push, pushSent[macId] != registration.handle else { return }
        pushSent[macId] = registration.handle
        await send(PushRegister(handle: registration.handle, env: registration.env, expiresAt: registration.expiresAt), to: macId)
    }

    func onPushState(_ macId: String, _ state: PushState) {
        macs[macId]?.pushStatus = state.status
        guard state.status == "gone" || state.status == "missing" else { return }
        // The relay or the Mac lost the handle: renew and resend.
        pushSent[macId] = nil
        Task {
            await self.registerPushIfNeeded(force: state.status == "gone")
            await self.sendPushHandle(to: macId)
            self.publish()
        }
    }

}
