// swift-tools-version: 5.9
import PackageDescription

// The helper app itself is built by Xcode (project.yml, XcodeGen) because it needs entitlements and a
// provisioning profile. This package exists so that the platform-independent part of the helper (wire
// codec, RPC dispatcher, tick scheduler, error mapping, socket server) can be unit-tested with plain
// `swift test`, without signing and without touching iCloud.
let package = Package(
    name: "TodoSyncHelper",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "TodoSyncHelperKit", targets: ["TodoSyncHelperKit"])
    ],
    targets: [
        .target(name: "TodoSyncHelperKit", path: "Kit"),
        .testTarget(
            name: "TodoSyncHelperKitTests",
            dependencies: ["TodoSyncHelperKit"],
            path: "Tests/TodoSyncHelperKitTests"
        ),
    ]
)
