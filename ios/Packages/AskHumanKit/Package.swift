// swift-tools-version: 5.9
import PackageDescription

// AskHumanKit: everything of the iOS app that can be built and tested without UIKit.
//
//   AHProtocol   envelope, AEAD, HKDF / X25519, verification code, message models, limits
//   AHStore      keychain and file-backed storage
//   AHTransport  MQTT broker client (TLS 8883, WSS 8084 fallback)
//   AHCore       per-Mac sessions and the app-wide observable store
//   AHMarkdown   Markdown parsing into render blocks
//   AHMock       in-memory backend with sample data (previews, screenshots, UI tests)
//
// The notification extension links AHProtocol and AHStore (it only reads keys from the keychain).
let package = Package(
    name: "AskHumanKit",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [
        .library(name: "AHProtocol", targets: ["AHProtocol"]),
        .library(name: "AHStore", targets: ["AHStore"]),
        .library(name: "AHTransport", targets: ["AHTransport"]),
        .library(name: "AHCore", targets: ["AHCore"]),
        .library(name: "AHMarkdown", targets: ["AHMarkdown"]),
        .library(name: "AHMock", targets: ["AHMock"]),
    ],
    dependencies: [
        .package(url: "https://github.com/emqx/CocoaMQTT.git", exact: "2.4.1"),
    ],
    targets: [
        .target(name: "AHProtocol"),
        .target(name: "AHStore", dependencies: ["AHProtocol"]),
        .target(
            name: "AHTransport",
            dependencies: [
                "AHProtocol",
                .product(name: "CocoaMQTT", package: "CocoaMQTT"),
                .product(name: "CocoaMQTTWebSocket", package: "CocoaMQTT"),
            ]
        ),
        .target(name: "AHCore", dependencies: ["AHProtocol", "AHStore", "AHTransport"]),
        .target(name: "AHMarkdown"),
        .target(name: "AHMock", dependencies: ["AHProtocol", "AHCore"]),
        .testTarget(name: "AHProtocolTests", dependencies: ["AHProtocol"]),
        .testTarget(name: "AHStoreTests", dependencies: ["AHStore"]),
        .testTarget(name: "AHTransportTests", dependencies: ["AHTransport", "AHProtocol"]),
        .testTarget(name: "AHCoreTests", dependencies: ["AHCore", "AHProtocol", "AHStore", "AHTransport"]),
    ]
)
