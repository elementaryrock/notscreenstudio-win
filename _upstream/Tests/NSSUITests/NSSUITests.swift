import Testing
@testable import NSSUI

@Suite struct NSSUITests {
    @Test @MainActor func viewsConstruct() {
        _ = EditorView()
        let rig = ShellRig()
        defer { rig.cleanup() }
        _ = MenuBarContent(model: rig.model)
    }
}
