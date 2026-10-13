// swift-tools-version: 6.0

import Foundation
import PackageDescription

// `SN_KIT_ONLY=1 swift test` drops the app target so SNKit's tests run on a Linux host, where
// SwiftUI and AVFoundation do not exist. xtool also reads this manifest on Linux, so the switch
// is an environment variable, never `#if os(Linux)`.
let kitOnly = Context.environment["SN_KIT_ONLY"] == "1"

// The Rust core, cross-compiled for the iPhone into CoreLib. Until that library is there the
// app builds against FakeCore alone.
let coreLib = "\(Context.packageDirectory)/CoreLib"
let hasCore = !kitOnly && FileManager.default.fileExists(atPath: "\(coreLib)/libstreamnook_lib.a")

let coreTargets: [Target] = hasCore ? [
    // The C header and module map UniFFI generates for the core's exported functions. A system
    // library's module map must sit at its path, so the path is the include folder.
    .systemLibrary(name: "streamnook_libFFI", path: "Sources/streamnook_libFFI/include"),
    // The Swift side UniFFI generates. It is written for Swift 5, so it builds in that mode and
    // the app imports it with @preconcurrency.
    .target(name: "SNCore", dependencies: ["streamnook_libFFI"], path: "Sources/SNCore",
            swiftSettings: [.swiftLanguageMode(.v5)]),
] : []

let package = Package(
    name: "StreamNook",
    platforms: [
        .iOS("26.0"),
        .macOS(.v15),
    ],
    products: kitOnly ? [] : [
        // xtool builds the product named in xtool.yml as the app.
        .library(name: "StreamNook", targets: ["StreamNook"]),
        .library(name: "SNKit", targets: ["SNKit"]),
    ],
    targets: [
        // The bridge to the core, its fake, and the shapes of what the core answers. Foundation
        // only, so it builds and tests on Linux.
        .target(name: "SNKit"),
        .testTarget(name: "SNKitTests", dependencies: ["SNKit"]),
    ] + (kitOnly ? [] : coreTargets + [
        // The iPhone app: SwiftUI and AVFoundation over the core.
        .target(
            name: "StreamNook",
            dependencies: ["SNKit"] + (hasCore ? ["SNCore"] : []),
            linkerSettings: hasCore ? [
                .unsafeFlags(["-L\(coreLib)"]),
                .linkedLibrary("streamnook_lib"),
                .linkedFramework("Security"),
                .linkedFramework("SystemConfiguration"),
                .linkedFramework("CoreFoundation"),
            ] : []
        ),
    ])
)
