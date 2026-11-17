import CoreMedia
import Foundation
import NSSCore
import NSSRender

/// Non-interactive launch check (`NotScreenStudio --smoke-test`). It builds `AppModel` with every real
/// dependency, renders one frame through the shared renderer, and exits 0 on success or 1 with a message.
/// It needs no permissions, no window and no camera. Without a Metal device it still passes, as long as the
/// CPU fallback is the renderer in use (and says so).
enum SmokeTest {
    @MainActor
    static func runAndExit() -> Never {
        do {
            let report = try run()
            print("smoke-test OK: \(report)")
            exit(0)
        } catch {
            FileHandle.standardError.write(Data("smoke-test FAILED: \(error)\n".utf8))
            exit(1)
        }
    }

    @MainActor
    static func run() throws -> String {
        let app = AppModel()
        let deps = app.editorDependencies
        let usesMetal = deps.renderer is MetalRenderer
        if !usesMetal && MetalRenderer.isSupported {
            throw NSSError.renderFailed(app.rendererError ?? "Metal is available but the renderer is not MetalRenderer")
        }
        let screen = try PixelBuffers.make(width: 640, height: 360)
        let out = try PixelBuffers.make(width: 480, height: 270)
        try deps.renderer.render(screen: screen, camera: nil, state: FrameState(time: 0), style: Style(), into: out)
        // The same factory the editor uses must handle a non-16:9 source.
        let factory = deps.motionFactoryForAspect?(9.0 / 16.0) ?? deps.motionFactory
        _ = factory(CursorTrack.empty, Timeline(), CursorStyle(), 1).frameState(at: 0)
        return "renderer=\(usesMetal ? "Metal" : "passthrough") sources=wired"
    }
}
