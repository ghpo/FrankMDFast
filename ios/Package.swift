// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "FrankMDFast",
    platforms: [.iOS(.v17)],
    products: [
        .library(name: "FrankMDFast", targets: ["FrankMDFast"])
    ],
    targets: [
        .systemLibrary(
            name: "CFrankMD",
            path: "Sources/CFrankMD"
        ),
        .target(
            name: "FrankMDFast",
            dependencies: ["CFrankMD"],
            path: "Sources/FrankMDFast",
            linkerSettings: [.unsafeFlags(["-L../Frameworks"])]
        )
    ]
)
