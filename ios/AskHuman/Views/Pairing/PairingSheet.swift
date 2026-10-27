import SwiftUI
import VisionKit
import AHCore
import AHProtocol

/// Pairing with a Mac: scan or paste the code, compare the check number, allow notifications.
struct PairingSheet: View {
    @Environment(AppStore.self) private var store
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Group {
                if let pairing = store.state.pairing {
                    PairingProgress(pairing: pairing, close: close)
                } else {
                    PairingInput()
                }
            }
            .navigationTitle("Pair a Mac")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(isDone ? "Done" : "Cancel") { close() }
                        .accessibilityIdentifier("pairing-close")
                }
            }
        }
        .interactiveDismissDisabled(store.state.pairing != nil && !isTerminal)
    }

    private var isDone: Bool {
        if case .done? = store.state.pairing?.stage { return true }
        return false
    }

    private var isTerminal: Bool {
        switch store.state.pairing?.stage {
        case .done, .failed: true
        default: false
        }
    }

    private func close() {
        Task {
            if store.state.pairing != nil {
                if isDone || isTerminal { await store.finishPairing() } else { await store.cancelPairing() }
            }
            dismiss()
        }
    }
}

// MARK: input

private struct PairingInput: View {
    @Environment(AppStore.self) private var store
    @State private var code = ""
    @State private var scanning = false

    private var canScan: Bool { DataScannerViewController.isSupported && DataScannerViewController.isAvailable }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Scan the code from your Mac").font(.title3.weight(.bold))
                    Text("On the Mac, open AskHuman Settings → Experimental → iOS app channel and choose “Pair a new phone”. It shows a QR code and a pairing code.")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
                if canScan {
                    Button {
                        scanning = true
                    } label: {
                        Label("Scan QR code", systemImage: "qrcode.viewfinder").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .accessibilityIdentifier("scan-qr")
                }
                VStack(alignment: .leading, spacing: 10) {
                    Text(canScan ? "Or paste the pairing code" : "Paste the pairing code").font(.subheadline.weight(.semibold))
                    TextField("ahpair1.…", text: $code, axis: .vertical)
                        .lineLimit(2...5)
                        .font(.system(.footnote, design: .monospaced))
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .padding(10)
                        .background(Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                        .accessibilityIdentifier("pairing-code")
                    HStack {
                        PasteButton(payloadType: String.self) { strings in
                            if let text = strings.first { code = text.trimmingCharacters(in: .whitespacesAndNewlines) }
                        }
                        .buttonBorderShape(.capsule)
                        Spacer()
                        pairButton
                    }
                }
                .card()
                Text("The code works once and expires after five minutes. Your Mac asks you to confirm before anything is shared.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            .padding()
        }
        .background(Color(.systemGroupedBackground))
        .fullScreenCover(isPresented: $scanning) {
            QRScannerScreen { scanned in
                scanning = false
                start(scanned)
            }
        }
    }

    @ViewBuilder
    private var pairButton: some View {
        let empty = code.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        if canScan {
            Button("Pair") { start(code) }
                .buttonStyle(.bordered)
                .disabled(empty)
                .accessibilityIdentifier("pair-button")
        } else {
            Button("Pair") { start(code) }
                .buttonStyle(.borderedProminent)
                .disabled(empty)
                .accessibilityIdentifier("pair-button")
        }
    }

    private func start(_ text: String) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        Task { await store.startPairing(code: trimmed) }
    }
}

// MARK: progress

private struct PairingProgress: View {
    let pairing: PairingState
    let close: () -> Void
    @Environment(AppStore.self) private var store
    @Environment(PushCoordinator.self) private var push

    var body: some View {
        ScrollView {
            VStack(spacing: 22) {
                switch pairing.stage {
                case .connecting, .waitingForMac, .macConfirming, .finishing:
                    waiting
                case .done:
                    done
                case .failed(let failure):
                    failed(failure)
                }
            }
            .padding(24)
            .frame(maxWidth: .infinity)
        }
        .background(Color(.systemGroupedBackground))
    }

    private var waiting: some View {
        VStack(spacing: 22) {
            Image(systemName: "desktopcomputer.and.iphone").font(.system(size: 56)).foregroundStyle(.tint).padding(.top, 24)
            if let sas = pairing.sas {
                VStack(spacing: 10) {
                    Text("Check this number").font(.title3.weight(.bold))
                    Text(sas)
                        .font(.system(size: 44, weight: .bold, design: .monospaced))
                        .tracking(2)
                        .accessibilityIdentifier("pairing-sas")
                    Text("Your Mac shows the same number. Click Allow on the Mac only if they match.")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                }
                .card()
            } else {
                Text(pairing.stage == .connecting ? "Connecting…" : "Asking your Mac…").font(.title3.weight(.bold))
            }
            HStack(spacing: 8) {
                ProgressView()
                Text(statusLine).foregroundStyle(.secondary)
            }
            if !pairing.macName.isEmpty {
                Text(pairing.macName).font(.footnote).foregroundStyle(.tertiary)
            }
        }
    }

    private var statusLine: LocalizedStringKey {
        switch pairing.stage {
        case .connecting: "Reaching the broker"
        case .waitingForMac: "Waiting for your Mac"
        case .macConfirming: "Waiting for you to allow it on the Mac"
        case .finishing: "Finishing"
        default: ""
        }
    }

    private var done: some View {
        VStack(spacing: 18) {
            Image(systemName: "checkmark.circle.fill").font(.system(size: 64)).foregroundStyle(.green).padding(.top, 24)
            Text("Paired with \(pairing.macName)").font(.title2.weight(.bold)).multilineTextAlignment(.center)
            Text("Questions and confirmations from this Mac now arrive on this phone.")
                .font(.subheadline)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            if push.authorization == .notDetermined || push.authorization == .unknown {
                VStack(alignment: .leading, spacing: 10) {
                    Label("Get notified", systemImage: "bell.badge.fill").font(.headline)
                    Text("Allow notifications so a new question reaches you even when AskHuman is closed. Notification text is end-to-end encrypted and only readable on this phone.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                    Button {
                        Task {
                            await push.requestAuthorization()
                            close()
                        }
                    } label: {
                        Text("Turn on notifications").frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    Button("Not now", action: close).frame(maxWidth: .infinity)
                }
                .card()
            } else {
                Button(action: close) { Text("Done").frame(maxWidth: .infinity) }
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
            }
        }
    }

    private func failed(_ failure: PairingFailure) -> some View {
        VStack(spacing: 18) {
            Image(systemName: "xmark.octagon.fill").font(.system(size: 56)).foregroundStyle(.red).padding(.top, 24)
            Text("Pairing did not work").font(.title2.weight(.bold))
            Text(message(for: failure)).font(.subheadline).foregroundStyle(.secondary).multilineTextAlignment(.center)
            Button {
                Task { await store.finishPairing() }
            } label: {
                Text("Try again").frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
            .accessibilityIdentifier("pairing-retry")
        }
    }

    private func message(for failure: PairingFailure) -> String {
        switch failure {
        case .invalidCode: String(localized: "That is not a valid pairing code. Copy it again from the Mac.")
        case .expired: String(localized: "The pairing code has expired. Create a new one on the Mac.")
        case .differentBroker: String(localized: "This code belongs to a different broker account than the Macs you already paired. Forget all pairings in the Macs tab first.")
        case .brokerUnreachable: String(localized: "The broker could not be reached. Check your network and try again.")
        case .authFailed: String(localized: "The broker refused the credentials in the code.")
        case .rejected(let reason):
            reason == "cancelled" ? String(localized: "The pairing was cancelled on the Mac.") : String(localized: "The Mac refused the pairing.")
        case .timeout: String(localized: "The Mac did not answer in time. Make sure AskHuman is running on it.")
        case .failed(let reason): reason
        }
    }
}

// MARK: scanner

struct QRScannerScreen: View {
    let onCode: (String) -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScannerView(onCode: onCode)
                .ignoresSafeArea()
                .navigationTitle("Scan QR code")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } }
                }
        }
    }
}

private struct ScannerView: UIViewControllerRepresentable {
    let onCode: (String) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(onCode: onCode) }

    func makeUIViewController(context: Context) -> DataScannerViewController {
        let controller = DataScannerViewController(
            recognizedDataTypes: [.barcode(symbologies: [.qr])], qualityLevel: .balanced, recognizesMultipleItems: false,
            isHighFrameRateTrackingEnabled: false, isPinchToZoomEnabled: true, isGuidanceEnabled: true, isHighlightingEnabled: true
        )
        controller.delegate = context.coordinator
        try? controller.startScanning()
        return controller
    }

    func updateUIViewController(_ controller: DataScannerViewController, context: Context) {}

    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        let onCode: (String) -> Void
        private var delivered = false

        init(onCode: @escaping (String) -> Void) { self.onCode = onCode }

        func dataScanner(_ dataScanner: DataScannerViewController, didAdd addedItems: [RecognizedItem], allItems: [RecognizedItem]) {
            guard !delivered else { return }
            for case .barcode(let barcode) in addedItems {
                if let text = barcode.payloadStringValue, text.hasPrefix("ahpair1.") {
                    delivered = true
                    onCode(text)
                    return
                }
            }
        }
    }
}
