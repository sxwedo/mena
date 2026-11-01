// swift-tools-version: 6.2
import Foundation
import PackageDescription

let repository = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
let coreDirectory =
    ProcessInfo.processInfo.environment["MENA_CORE_LIB_DIR"]
    ?? repository.appendingPathComponent("target/debug").path

let package = Package(
    name: "MenaDesktop",
    platforms: [.macOS(.v14)],
    products: [.executable(name: "Mena", targets: ["Mena"])],
    dependencies: [
        .package(url: "https://github.com/migueldeicaza/SwiftTerm.git", exact: "1.99.2")
    ],
    targets: [
        .systemLibrary(name: "MenaFFI", path: "Generated/MenaFFI"),
        .target(
            name: "MenaCore",
            dependencies: ["MenaFFI"],
            path: "Generated/MenaCore",
            linkerSettings: [
                .unsafeFlags(["-L", coreDirectory, "-Xlinker", "-rpath", "-Xlinker", coreDirectory]),
                .linkedLibrary("mena"),
            ]
        ),
        .executableTarget(
            name: "Mena",
            dependencies: ["MenaCore", .product(name: "SwiftTerm", package: "SwiftTerm")],
            path: "Sources/Mena",
            resources: [.process("Resources")]
        ),
        .testTarget(name: "MenaTests", dependencies: ["Mena"], path: "Tests/MenaTests"),
    ]
)
