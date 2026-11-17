// swift-tools-version: 6.0
import PackageDescription

let settings: [SwiftSetting] = [.swiftLanguageMode(.v6)]

func library(_ name: String, _ deps: [String]) -> Target {
    let dependencies: [Target.Dependency] = deps.map { Target.Dependency.target(name: $0) }
    return Target.target(name: name, dependencies: dependencies, swiftSettings: settings)
}

func tests(_ name: String, _ deps: [String]) -> Target {
    let dependencies: [Target.Dependency] = deps.map { Target.Dependency.target(name: $0) }
    return Target.testTarget(name: name + "Tests", dependencies: dependencies, swiftSettings: settings)
}

let allModules: [String] = ["NSSCore", "NSSCapture", "NSSInput", "NSSProject", "NSSMotion", "NSSRender", "NSSExport"]
let uiDeps: [String] = allModules
let appDeps: [String] = allModules + ["NSSUI"]

var targets: [Target] = []
targets.append(library("NSSCore", []))
targets.append(library("NSSCapture", ["NSSCore"]))
targets.append(library("NSSInput", ["NSSCore"]))
targets.append(library("NSSProject", ["NSSCore"]))
targets.append(library("NSSMotion", ["NSSCore"]))
targets.append(
    Target.target(
        name: "NSSRender",
        dependencies: [Target.Dependency.target(name: "NSSCore")],
        exclude: ["RENDERING.md"],
        swiftSettings: settings
    )
)
targets.append(library("NSSExport", ["NSSCore", "NSSRender", "NSSMotion"]))
targets.append(library("NSSUI", uiDeps))
targets.append(tests("NSSCore", ["NSSCore"]))
targets.append(tests("NSSCapture", ["NSSCapture", "NSSCore"]))
targets.append(tests("NSSInput", ["NSSInput", "NSSCore"]))
targets.append(tests("NSSProject", ["NSSProject", "NSSCore"]))
targets.append(tests("NSSMotion", ["NSSMotion", "NSSCore"]))
targets.append(tests("NSSRender", ["NSSRender", "NSSCore"]))
targets.append(tests("NSSExport", ["NSSExport", "NSSCore", "NSSRender", "NSSMotion"]))
targets.append(tests("NSSUI", ["NSSUI", "NSSCore"]))
// Cross-module end-to-end tests: synthetic bundle -> store -> motion -> Metal -> MP4, plus preview timing.
targets.append(tests("NSSIntegration", allModules + ["NSSUI"]))
targets.append(
    Target.executableTarget(
        name: "NotScreenStudio",
        dependencies: appDeps.map { Target.Dependency.target(name: $0) },
        swiftSettings: settings
    )
)

let package = Package(
    name: "notscreenstudio",
    platforms: [.macOS(.v15)],
    products: [.executable(name: "NotScreenStudio", targets: ["NotScreenStudio"])],
    targets: targets
)
