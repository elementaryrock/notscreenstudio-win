import Foundation
import Testing
@testable import NSSCore

@Suite struct NSSCoreTests {
    @Test func projectRoundTripsThroughJSON() throws {
        let assets = RecordingAssets(cameraFile: "camera.mov", sourcePixelSize: PixelSize(width: 2560, height: 1440), frameRate: 60, duration: 12.5)
        var project = Project(name: "Demo", assets: assets)
        project.timeline.zoomBlocks = [ZoomBlock(start: 1, end: 3, scale: 2, focus: .manual(NormalizedPoint(x: 0.3, y: 0.7)))]
        project.style.camera.shape = .tall
        let data = try JSONEncoder().encode(project)
        let back = try JSONDecoder().decode(Project.self, from: data)
        #expect(back == project)
        #expect(back.schemaVersion == Project.currentSchemaVersion)
    }

    @Test func aspectRatioOutputSizes() {
        #expect(AspectRatio.r16x9.outputSize(for: .p1080) == PixelSize(width: 1920, height: 1080))
        #expect(AspectRatio.r16x9.outputSize(for: .p2160) == PixelSize(width: 3840, height: 2160))
        #expect(AspectRatio.r1x1.outputSize(for: .p1080) == PixelSize(width: 1080, height: 1080))
        #expect(AspectRatio.r9x16.outputSize(for: .p2160) == PixelSize(width: 2160, height: 3840))
    }

    @Test func cursorTrackInterpolates() {
        let track = CursorTrack(samples: [CursorSample(t: 0, x: 0, y: 0), CursorSample(t: 1, x: 1, y: 0.5)])
        #expect(track.sampleRate == 120)
        let p = track.rawPosition(at: 0.5)
        #expect(p == NormalizedPoint(x: 0.5, y: 0.25))
        #expect(track.rawPosition(at: 5) == NormalizedPoint(x: 1, y: 0.5))
        #expect(CursorTrack.empty.rawPosition(at: 0) == nil)
    }

    @Test func exportPresetDefaults() {
        let p = ExportPreset()
        #expect(p.fps == 60)
        #expect(p.outputSize(for: .r9x16) == PixelSize(width: 1080, height: 1920))
    }

    @Test func olderProjectJSONWithoutNewerFieldsStillDecodes() throws {
        let json = """
        {"name":"Old","assets":{"sourcePixelSize":{"width":100,"height":50},"frameRate":60,"duration":2},
         "style":{"padding":0.1,"cursor":{"size":2}},"timeline":{}}
        """
        let p = try JSONDecoder().decode(Project.self, from: Data(json.utf8))
        #expect(p.name == "Old")
        #expect(p.assets.startHostTime == 0)
        #expect(p.assets.screenFile == RecordingAssets.defaultScreenFile)
        #expect(p.style.padding == 0.1)
        #expect(p.style.cursor.size == 2)
        #expect(p.style.cursor.smoothing == CursorStyle().smoothing)
        #expect(p.style.camera == CameraBubble())
        #expect(p.timeline.autoZoomEnabled)
    }

    @Test func startHostTimeRoundTrips() throws {
        let a = RecordingAssets(sourcePixelSize: PixelSize(width: 1, height: 1), frameRate: 60, duration: 1, startHostTime: 12345.678)
        let back = try JSONDecoder().decode(RecordingAssets.self, from: JSONEncoder().encode(a))
        #expect(back.startHostTime == 12345.678)
    }

    @Test func cameraWithoutBorderRoundTrips() throws {
        var p = Project(name: "n", assets: RecordingAssets(sourcePixelSize: PixelSize(width: 1, height: 1), frameRate: 60, duration: 1))
        p.style.camera.border = nil
        let back = try JSONDecoder().decode(Project.self, from: JSONEncoder().encode(p))
        #expect(back.style.camera.border == nil)
        #expect(back == p)
    }
}
