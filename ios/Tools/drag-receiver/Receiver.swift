import UIKit
import SwiftUI
import UniformTypeIdentifiers
import CryptoKit

/// Simulator-only drop target: verifies actual cross-process files without sending messages or using an account.
@main
struct ReceiverApp: App {
    var body: some Scene { WindowGroup { ReceiverSurface().ignoresSafeArea() } }
}

private struct ReceiverSurface: UIViewRepresentable {
    func makeUIView(context: Context) -> ReceiverView { ReceiverView() }
    func updateUIView(_ view: ReceiverView, context: Context) {}
}

final class ReceiverView: UIView, UIDropInteractionDelegate {
    private let label = UILabel()

    init() {
        super.init(frame: .zero)
        backgroundColor = .systemBackground
        label.text = "Drop a file here"
        label.numberOfLines = 0
        label.textAlignment = .center
        label.accessibilityIdentifier = "drop-result"
        label.translatesAutoresizingMaskIntoConstraints = false
        addSubview(label)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 20),
            label.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -20),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
        addInteraction(UIDropInteraction(delegate: self))
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    func dropInteraction(_ interaction: UIDropInteraction, canHandle session: UIDropSession) -> Bool {
        label.text = "Offered: " + session.items.flatMap { $0.itemProvider.registeredTypeIdentifiers }.joined(separator: ", ")
        return !session.items.isEmpty
    }

    func dropInteraction(_ interaction: UIDropInteraction, sessionDidUpdate session: UIDropSession) -> UIDropProposal {
        UIDropProposal(operation: .copy)
    }

    func dropInteraction(_ interaction: UIDropInteraction, performDrop session: UIDropSession) {
        for item in session.items {
            let provider = item.itemProvider
            guard let type = provider.registeredTypeIdentifiers.first else { continue }
            label.text = "Loading promised file…"
            provider.loadFileRepresentation(forTypeIdentifier: type) { url, error in
                var result: String
                do {
                    if let error { throw error }
                    guard let url else { throw CocoaError(.fileNoSuchFile) }
                    let data = try Data(contentsOf: url)
                    let digest = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
                    let document = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
                    try data.write(to: document.appendingPathComponent(url.lastPathComponent))
                    result = "Received \(url.lastPathComponent)\n\(type)\n\(data.count) bytes\n\(digest)"
                    try result.write(to: document.appendingPathComponent("receipt.txt"), atomically: true, encoding: .utf8)
                } catch { result = "Failed: \(error.localizedDescription)" }
                let received = result
                DispatchQueue.main.async { self.label.text = received }
            }
        }
    }
}
