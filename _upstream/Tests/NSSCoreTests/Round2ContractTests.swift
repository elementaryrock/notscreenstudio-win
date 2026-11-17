import Foundation
import Testing
@testable import NSSCore

@Suite struct Round2ContractTests {
    private func dec<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try JSONDecoder().decode(T.self, from: Data(json.utf8))
    }

    @Test func cursorShapeUnknownDecodes() throws {
        #expect(try dec([CursorShape].self, #"["iBeam","somethingNew"]"#) == [.iBeam, .unknown])
        let data = try JSONEncoder().encode([CursorShape.pointingHand])
        #expect(String(decoding: data, as: UTF8.self) == #"["pointingHand"]"#)
    }

    @Test func shapeLookup() {
        let t = CursorTrack(shapes: [
            CursorShapeChange(t: 1, shape: .iBeam), CursorShapeChange(t: 2, shape: .pointingHand),
            CursorShapeChange(t: 3, shape: .arrow),
        ])
        #expect(t.shape(at: 0) == .arrow)
        #expect(t.shape(at: 1) == .iBeam)
        #expect(t.shape(at: 1.99) == .iBeam)
        #expect(t.shape(at: 2) == .pointingHand)
        #expect(t.shape(at: 100) == .arrow)
        #expect(CursorTrack().shape(at: 5) == .arrow)
    }

    @Test func oldCursorJSONDecodes() throws {
        let t = try dec(CursorTrack.self, #"{"samples":[{"t":0,"x":0.1,"y":0.2}],"clicks":[],"sampleRate":120}"#)
        #expect(t.shapes.isEmpty && t.samples.count == 1)
        let s = try dec(CursorState.self, #"{"position":{"x":0.5,"y":0.5}}"#)
        #expect(s.shape == .arrow && s.pressScale == 1)
        let rt = CursorState(shape: .crosshair, pressScale: 0.8)
        #expect(try JSONDecoder().decode(CursorState.self, from: JSONEncoder().encode(rt)) == rt)
    }

    @Test func bubbleMigrations() throws {
        func bubble(_ shapeJSON: String) throws -> CameraBubble {
            try dec(CameraBubble.self, #"{"visible":true,"shape":\#(shapeJSON),"size":0.2}"#)
        }
        let circle = try bubble(#"{"circle":{}}"#)
        #expect(circle.shape == .circle)
        let rr = try bubble(#"{"roundedRect":{"radius":0.2}}"#)
        #expect(rr.shape == .square && abs(rr.roundness - 0.4) < 1e-9)
        let sq = try bubble(#"{"squircle":{}}"#)
        #expect(sq.shape == .square && sq.roundness == 0.6)
        let rect = try bubble(#"{"rectangle":{}}"#)
        #expect(rect.shape == .wide && rect.roundness == 0)
        let none = try dec(CameraBubble.self, #"{"size":0.2}"#)
        #expect(none.zoomedScale == 0.6 && none.border == nil)
        let new = CameraBubble(shape: .tall, roundness: 0.7, zoomedScale: 0.5)
        #expect(try JSONDecoder().decode(CameraBubble.self, from: JSONEncoder().encode(new)) == new)
    }

    @Test func bubbleGeometry() {
        let sq = CameraBubbleGeometry.layout(shape: .square, roundness: 1, width: 100)
        #expect(sq.height == 100 && sq.cornerRadius == 50)
        let tall = CameraBubbleGeometry.layout(shape: .tall, roundness: 1, width: 300)
        #expect(tall.height == 400 && tall.cornerRadius == 150)
        let wide = CameraBubbleGeometry.layout(shape: .wide, roundness: 0, width: 160)
        #expect(abs(wide.height - 90) < 1e-9 && wide.cornerRadius == 0)
        #expect(CameraBubbleGeometry.layout(shape: .circle, roundness: 0, width: 80).cornerRadius == 40)
    }

    @Test func sizeMultiplier() {
        #expect(bubbleSizeMultiplier(cameraScale: 1) == 1)
        #expect(bubbleSizeMultiplier(cameraScale: 0.5) == 1)
        #expect(abs(bubbleSizeMultiplier(cameraScale: 2) - 0.6) < 1e-12)
        #expect(abs(bubbleSizeMultiplier(cameraScale: 4) - 0.6) < 1e-12)
        #expect(abs(bubbleSizeMultiplier(cameraScale: 1.5) - 0.8) < 1e-12)
        #expect(CameraBubble(zoomedScale: 1).sizeMultiplier(cameraScale: 2) == 1)
    }

    @Test func recordingAdditions() throws {
        #expect(RecordingOptions(source: .display(id: 1)).countdownSeconds == 3)
        let old = try dec(RecordingOptions.self, #"{"source":{"display":{"id":1}},"captureSystemAudio":false}"#)
        #expect(old.countdownSeconds == 3)
        let a = try dec(RecordingAssets.self, #"{"sourcePixelSize":{"width":10,"height":10},"frameRate":60,"duration":1}"#)
        #expect(a.thumbnailFile == nil)
    }

    @Test func inMemoryLibrary() async throws {
        let ref = ProjectReference(url: URL(fileURLWithPath: "/tmp/a.nssproj"))
        let old = ProjectSummary(name: "old", createdAt: Date(timeIntervalSince1970: 1), duration: 1, bundle: ref)
        let new = ProjectSummary(name: "new", createdAt: Date(timeIntervalSince1970: 9), duration: 2, bundle: ref)
        let lib = InMemoryProjectLibrary([old, new])
        #expect(try await lib.summaries().map(\.name) == ["new", "old"])
        try await lib.rename(old, to: "renamed")
        try await lib.delete(new)
        #expect(try await lib.summaries().map(\.name) == ["renamed"])
    }
}
