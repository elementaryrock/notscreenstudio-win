import AppKit
import CoreGraphics
import Foundation
import Testing
import NSSCore
import os
@testable import NSSInput

/// A synthetic cursor image drawn into a bitmap rep (needs no window server).
private func image(width: Int, height: Int, _ draw: (CGContext, CGRect) -> Void) -> NSImage {
    let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height, bitsPerSample: 8, samplesPerPixel: 4,
        hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
    )!
    let ctx = NSGraphicsContext(bitmapImageRep: rep)!
    draw(ctx.cgContext, CGRect(x: 0, y: 0, width: width, height: height))
    ctx.flushGraphics()
    let img = NSImage(size: NSSize(width: width, height: height))
    img.addRepresentation(rep)
    return img
}

private func triangle(_ size: Int) -> NSImage {
    image(width: size, height: size) { c, r in
        c.setFillColor(.black)
        c.move(to: CGPoint(x: 0, y: r.height))
        c.addLine(to: CGPoint(x: 0, y: r.height * 0.2))
        c.addLine(to: CGPoint(x: r.width * 0.7, y: r.height * 0.4))
        c.closePath()
        c.fillPath()
    }
}

private func bar(_ w: Int, _ h: Int) -> NSImage {
    image(width: w, height: h) { c, r in
        c.setFillColor(.black)
        c.fill(CGRect(x: r.width * 0.4, y: 0, width: r.width * 0.2, height: r.height))
    }
}

private func disc(_ size: Int, gray: CGFloat) -> NSImage {
    image(width: size, height: size) { c, r in
        c.setFillColor(CGColor(gray: gray, alpha: 1))
        c.fillEllipse(in: r.insetBy(dx: 2, dy: 2))
    }
}

private func fingerprint(_ img: NSImage, hot: NSPoint) -> CursorFingerprint {
    CursorFingerprint.make(image: img, hotSpot: hot)!
}

/// Plays back a scripted list of samples, one per tick (nil = unreadable), then repeats the last.
private final class ScriptedSampler: Sendable {
    private let script: [CursorShape?]
    private let index = OSAllocatedUnfairLock(initialState: 0)
    init(_ script: [CursorShape?]) { self.script = script }
    var finished: Bool { index.withLock { $0 >= script.count } }
    func next() -> CursorShape? {
        index.withLock { i in
            let v = script[min(i, script.count - 1)]
            i += 1
            return v
        }
    }
}

private let hasWindowServer = CGMainDisplayID() != 0 && CGDisplayBounds(CGMainDisplayID()).width > 0

@Suite struct CursorShapeRecorderTests {
    @Test func emitsOnlyOnChangeAndSkipsUnreadableSamples() {
        let script = ScriptedSampler([.arrow, .arrow, .iBeam, nil, .iBeam, .pointingHand, .unknown, .unknown, .arrow])
        let time = OSAllocatedUnfairLock(initialState: 10.0)
        let rec = CursorShapeRecorder(sampler: { script.next() }, clock: { time.withLock { $0 } })
        for _ in 0..<9 {
            rec.tick()
            time.withLock { $0 += 1.0 / 30 }
        }
        let shapes = rec.finish(recordingStartHostTime: 9).map(\.shape)
        #expect(shapes == [.arrow, .iBeam, .pointingHand, .unknown, .arrow])
    }

    @Test func finishRebasesAndKeepsTheShapeInEffectAtTheOrigin() {
        let script = ScriptedSampler([.arrow, .iBeam, .pointingHand, .arrow])
        let times = OSAllocatedUnfairLock(initialState: [99.0, 99.5, 100.2345678, 101.0])
        let rec = CursorShapeRecorder(sampler: { script.next() }, clock: { times.withLock { $0.removeFirst() } })
        for _ in 0..<4 { rec.tick() }
        let out = rec.finish(recordingStartHostTime: 100)
        #expect(out == [
            CursorShapeChange(t: 0, shape: .iBeam),
            CursorShapeChange(t: 0.2346, shape: .pointingHand),
            CursorShapeChange(t: 1, shape: .arrow),
        ])
    }

    @Test func allChangesBeforeOriginLeaveOneAtZero() {
        let script = ScriptedSampler([.arrow, .crosshair])
        let times = OSAllocatedUnfairLock(initialState: [1.0, 2.0])
        let rec = CursorShapeRecorder(sampler: { script.next() }, clock: { times.withLock { $0.removeFirst() } })
        rec.tick()
        rec.tick()
        #expect(rec.finish(recordingStartHostTime: 5) == [CursorShapeChange(t: 0, shape: .crosshair)])
        #expect(CursorShapeRecorder(sampler: { nil }, clock: { 0 }).finish(recordingStartHostTime: 0).isEmpty)
    }

    @Test func loggerStoresShapesFromAnInjectedSampler() async throws {
        let script = ScriptedSampler([.arrow, .arrow, .iBeam, .iBeam, .pointingHand])
        let logger = SystemInputLogger(
            geometry: FakeGeometry(rect: CGRect(x: 0, y: 0, width: 100, height: 100)),
            monitorClicks: false,
            shapeRate: 200,
            shapeSampler: { script.next() }
        )
        try await logger.start(source: .display(id: 1))
        // Wait until the script has played out (bounded; no timing assertion).
        for _ in 0..<200 {
            try await Task.sleep(for: .milliseconds(10))
            if script.finished { break }
        }
        let track = await logger.stop(recordingStartHostTime: 0)
        #expect(track.shapes.map(\.shape) == [.arrow, .iBeam, .pointingHand])
        #expect(track.shapes.map(\.t) == track.shapes.map(\.t).sorted())

        let off = SystemInputLogger(
            geometry: FakeGeometry(rect: CGRect(x: 0, y: 0, width: 100, height: 100)),
            monitorClicks: false, logCursorShapes: false, shapeSampler: { .iBeam }
        )
        try await off.start(source: .display(id: 1))
        #expect(await off.stop(recordingStartHostTime: 0).shapes.isEmpty)
    }
}

@Suite struct CursorShapeIdentifierTests {
    let identifier = CursorShapeIdentifier(references: [
        .init(shape: .arrow, fingerprint: fingerprint(triangle(24), hot: NSPoint(x: 4, y: 4))),
        .init(shape: .iBeam, fingerprint: fingerprint(bar(12, 24), hot: NSPoint(x: 6, y: 12))),
        .init(shape: .openHand, fingerprint: fingerprint(disc(32, gray: 0), hot: NSPoint(x: 16, y: 16))),
        .init(shape: .closedHand, fingerprint: fingerprint(disc(32, gray: 1), hot: NSPoint(x: 16, y: 16))),
        .init(shape: .pointingHand, fingerprint: fingerprint(disc(32, gray: 0), hot: NSPoint(x: 13, y: 8))),
    ])

    @Test func identifiesExactAndRescaledImages() {
        #expect(identifier.identify(fingerprint(triangle(24), hot: NSPoint(x: 4, y: 4))) == .arrow)
        // Same cursor at twice the size (Retina rep or a larger Accessibility pointer).
        #expect(identifier.identify(fingerprint(triangle(48), hot: NSPoint(x: 8, y: 8))) == .arrow)
        #expect(identifier.identify(fingerprint(bar(12, 24), hot: NSPoint(x: 6, y: 12))) == .iBeam)
    }

    @Test func hotspotAndPixelsBothDisambiguate() {
        // Same image, different hotspot.
        #expect(identifier.identify(fingerprint(disc(32, gray: 0), hot: NSPoint(x: 16, y: 16))) == .openHand)
        #expect(identifier.identify(fingerprint(disc(32, gray: 0), hot: NSPoint(x: 13, y: 8))) == .pointingHand)
        // Same geometry, different pixels.
        #expect(identifier.identify(fingerprint(disc(32, gray: 1), hot: NSPoint(x: 16, y: 16))) == .closedHand)
    }

    @Test func unknownForCustomCursorsAndUnreadableImages() {
        #expect(identifier.identify(fingerprint(disc(32, gray: 0.5), hot: NSPoint(x: 16, y: 16))) == .unknown)
        #expect(identifier.identify(fingerprint(triangle(24), hot: NSPoint(x: 20, y: 20))) == .unknown)
        #expect(identifier.identify(fingerprint(bar(24, 12), hot: NSPoint(x: 12, y: 6))) == .unknown)
        #expect(identifier.identify(nil) == .unknown)
        #expect(CursorShapeIdentifier(references: []).identify(fingerprint(triangle(24), hot: .zero)) == .unknown)
        #expect(CursorFingerprint.make(image: NSImage(size: NSSize(width: 10, height: 10)), hotSpot: .zero) == nil)
    }

    // MARK: Real system cursors (skip without a window server)

    @MainActor
    @Test(.enabled(if: hasWindowServer))
    func standardCursorsAreAllDistinguishable() throws {
        _ = NSApplication.shared  // standard cursor images load through the window server connection
        let id = CursorShapeIdentifier.standard()
        try #require(id.references.contains { $0.shape == .arrow })
        var firstShapeForImage: [CursorShape] = []
        for (shape, cursor) in CursorShapeIdentifier.standardCursors() {
            guard let fp = CursorFingerprint.make(cursor) else { continue }
            let got = id.identify(fp)
            // A cursor that reuses an earlier image (column resize == resize left-right) maps to that shape.
            #expect(got == shape || firstShapeForImage.contains(got), "\(shape) identified as \(got)")
            firstShapeForImage.append(shape)
        }
    }

    @MainActor
    @Test(.enabled(if: hasWindowServer))
    func systemSamplerReadsTheLiveCursor() {
        _ = NSApplication.shared
        guard let sampler = SystemCursorShapeSampler.make() else { return }
        // Whatever the cursor is right now, a read either fails (nil) or yields a shape; it never crashes.
        _ = sampler()
    }
}
