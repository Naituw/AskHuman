// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "speech-demo",
    platforms: [.macOS("26.0")],
    targets: [
        .target(
            name: "SpeechKit",
            path: "Sources/SpeechKit",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        .executableTarget(
            name: "speech-demo",
            dependencies: ["SpeechKit"],
            path: "Sources/speech-demo",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        .executableTarget(
            name: "SpeechGUI",
            dependencies: ["SpeechKit"],
            path: "Sources/SpeechGUI",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)
