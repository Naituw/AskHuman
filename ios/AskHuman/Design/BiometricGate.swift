import LocalAuthentication

/// The "is it really you" check in front of starting an Agent on a Mac.
enum BiometricGate {
    /// Asks for Face ID, falling back to the device passcode. Every call asks again: the approval is
    /// never remembered. Returns `false` when the user fails or cancels. A device with neither a
    /// passcode nor biometrics cannot ask, and the task goes ahead (nothing protects the phone either).
    static func confirm(reason: String) async -> Bool {
        let context = LAContext()
        var unavailable: NSError?
        guard context.canEvaluatePolicy(.deviceOwnerAuthentication, error: &unavailable) else { return true }
        do {
            return try await context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason)
        } catch {
            return false
        }
    }
}
