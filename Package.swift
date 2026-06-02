// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "AskHuman",
    platforms: [
        .macOS(.v13)
    ],
    targets: [
        .executableTarget(
            name: "AskHuman",
            path: "Sources/AskHuman"
        ),
        .testTarget(
            name: "AskHumanTests",
            dependencies: ["AskHuman"],
            path: "Tests/AskHumanTests"
        )
    ]
)
