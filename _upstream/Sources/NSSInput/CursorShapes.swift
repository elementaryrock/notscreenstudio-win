import AppKit
import CoreGraphics
import Foundation
import NSSCore
import os

// MARK: - Fingerprint

/// A size-independent description of a cursor image, used to tell system cursors apart.
///
/// The image is drawn into a tiny `side` x `side` bitmap and reduced to alpha and luminance per pixel.
/// Two cursors match when they have the same aspect, the same hotspot (as a fraction of the image size)
/// and nearly the same pixels. Because everything is relative to the image size, the match still works when
/// the user enlarges the pointer in Accessibility settings or the image comes in a different resolution.
public struct CursorFingerprint: Sendable, Equatable {
    public static let side = 16

    /// Image width / height.
    public var aspect: Double
    /// Hotspot as a fraction of the image size, origin top-left (the `NSCursor.hotSpot` convention).
    public var hotSpotX: Double
    public var hotSpotY: Double
    /// `side * side` pairs of (alpha, luminance), each 0...1.
    public var pixels: [Float]

    public init(aspect: Double, hotSpotX: Double, hotSpotY: Double, pixels: [Float]) {
        self.aspect = aspect
        self.hotSpotX = hotSpotX
        self.hotSpotY = hotSpotY
        self.pixels = pixels
    }

    /// Mean absolute pixel difference, 0 (identical) ... 1. `.infinity` when the sizes differ.
    public func pixelDistance(to other: CursorFingerprint) -> Float {
        guard pixels.count == other.pixels.count, !pixels.isEmpty else { return .infinity }
        var sum: Float = 0
        for i in pixels.indices { sum += abs(pixels[i] - other.pixels[i]) }
        return sum / Float(pixels.count)
    }

    /// True when the aspect and the hotspot agree within the tolerances of `CursorShapeIdentifier`.
    func sameGeometry(as other: CursorFingerprint, tolerance: Double) -> Bool {
        abs(aspect - other.aspect) <= tolerance
            && abs(hotSpotX - other.hotSpotX) <= tolerance
            && abs(hotSpotY - other.hotSpotY) <= tolerance
    }

    /// Fingerprint of an `NSCursor`, or nil when its image cannot be drawn (no representations).
    /// Safe off the main thread: it draws into its own bitmap context and never touches the current
    /// context of another thread.
    public static func make(_ cursor: NSCursor) -> CursorFingerprint? {
        make(image: cursor.image, hotSpot: cursor.hotSpot)
    }

    public static func make(image: NSImage, hotSpot: NSPoint) -> CursorFingerprint? {
        let size = image.size
        guard size.width > 0, size.height > 0, !image.representations.isEmpty else { return nil }
        let n = side
        var buf = [UInt8](repeating: 0, count: n * n * 4)
        let drawn: Bool = buf.withUnsafeMutableBytes { raw -> Bool in
            guard let ctx = CGContext(
                data: raw.baseAddress, width: n, height: n, bitsPerComponent: 8, bytesPerRow: n * 4,
                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            ) else { return false }
            ctx.interpolationQuality = .medium
            NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = NSGraphicsContext(cgContext: ctx, flipped: false)
            image.draw(in: NSRect(x: 0, y: 0, width: n, height: n), from: .zero, operation: .copy, fraction: 1)
            NSGraphicsContext.restoreGraphicsState()
            return true
        }
        guard drawn else { return nil }
        var pixels = [Float]()
        pixels.reserveCapacity(n * n * 2)
        for i in 0..<(n * n) {
            let r = Float(buf[i * 4]), g = Float(buf[i * 4 + 1]), b = Float(buf[i * 4 + 2]), a = Float(buf[i * 4 + 3])
            pixels.append(a / 255)
            pixels.append((r + g + b) / (3 * 255))
        }
        return CursorFingerprint(
            aspect: Double(size.width / size.height),
            hotSpotX: Double(hotSpot.x / size.width),
            hotSpotY: Double(hotSpot.y / size.height),
            pixels: pixels
        )
    }
}

// MARK: - Identification

/// Maps a cursor fingerprint to a `CursorShape` by comparing it with the standard `NSCursor`s.
/// Build it once per recording (`.standard()`), then call `identify` as often as needed.
public struct CursorShapeIdentifier: Sendable {
    public struct Reference: Sendable {
        public var shape: CursorShape
        public var fingerprint: CursorFingerprint
        public init(shape: CursorShape, fingerprint: CursorFingerprint) {
            self.shape = shape
            self.fingerprint = fingerprint
        }
    }

    public let references: [Reference]
    /// Largest difference in aspect and relative hotspot that still counts as the same cursor.
    public var geometryTolerance: Double = 0.03
    /// Largest mean pixel difference that still counts as the same cursor. The closest pair of distinct
    /// standard cursors (pointing hand and closed hand) differs by about 0.018 and also has a different
    /// hotspot; the system cursor and the matching standard cursor differ by 0 on macOS 15.
    public var pixelTolerance: Float = 0.01

    public init(references: [Reference]) {
        self.references = references
    }

    /// The best match, or `.unknown` when nothing is close enough (custom app cursors, for example).
    public func identify(_ fingerprint: CursorFingerprint?) -> CursorShape {
        guard let fingerprint else { return .unknown }
        var best: CursorShape = .unknown
        var bestDistance = Float.infinity
        for ref in references where ref.fingerprint.sameGeometry(as: fingerprint, tolerance: geometryTolerance) {
            let d = ref.fingerprint.pixelDistance(to: fingerprint)
            // Strict `<` keeps the first listed shape on ties (for example column resize == resize left-right).
            if d <= pixelTolerance, d < bestDistance {
                best = ref.shape
                bestDistance = d
            }
        }
        return best
    }

    /// Standard cursors in priority order: when two share an image, the first one wins.
    public static func standardCursors() -> [(CursorShape, NSCursor)] {
        [
            (.arrow, .arrow),
            (.iBeam, .iBeam),
            (.iBeamVertical, .iBeamCursorForVerticalLayout),
            (.pointingHand, .pointingHand),
            (.openHand, .openHand),
            (.closedHand, .closedHand),
            (.crosshair, .crosshair),
            (.resizeLeftRight, .resizeLeftRight),
            (.resizeUpDown, .resizeUpDown),
            (.resizeLeft, .resizeLeft),
            (.resizeRight, .resizeRight),
            (.resizeUp, .resizeUp),
            (.resizeDown, .resizeDown),
            // macOS 15 resize cursors. Where they reuse an image above, that entry already wins.
            (.resizeLeftRight, .columnResize),
            (.resizeUpDown, .rowResize),
            (.notAllowed, .operationNotAllowed),
            (.dragCopy, .dragCopy),
            (.dragLink, .dragLink),
            (.contextualMenu, .contextualMenu),
        ]
    }

    /// Fingerprints of the standard cursors. Cursors whose image cannot be drawn are skipped (the process
    /// must be connected to the window server, which an `NSApplication` always is).
    public static func standard() -> CursorShapeIdentifier {
        CursorShapeIdentifier(references: standardCursors().compactMap { shape, cursor in
            CursorFingerprint.make(cursor).map { Reference(shape: shape, fingerprint: $0) }
        })
    }
}

// MARK: - Change recording

/// Returns the shape showing right now, or nil when it cannot be read (that sample is skipped).
public typealias CursorShapeSampler = @Sendable () -> CursorShape?

/// Records cursor shape changes. Free of timers and AppKit so tests can drive it with a fake sampler and
/// clock. `tick()` is called from one queue; state is lock-protected.
final class CursorShapeRecorder: Sendable {
    typealias Clock = @Sendable () -> Double

    private struct State {
        var changes: [CursorShapeChange] = []
    }

    private let sampler: CursorShapeSampler
    private let clock: Clock
    private let state = OSAllocatedUnfairLock(initialState: State())

    init(sampler: @escaping CursorShapeSampler, clock: @escaping Clock) {
        self.sampler = sampler
        self.clock = clock
    }

    /// Samples once and records an event only when the shape differs from the last recorded one.
    func tick() {
        guard let shape = sampler() else { return }
        let now = clock()
        state.withLock { s in
            if s.changes.last?.shape != shape {
                s.changes.append(CursorShapeChange(t: now, shape: shape))
            }
        }
    }

    func changeCount() -> Int { state.withLock { $0.changes.count } }

    /// Rebases to the recording origin (host time of the first screen frame). The shape in effect at the
    /// origin is kept at t = 0; earlier changes are dropped. Times are rounded like the cursor samples.
    func finish(recordingStartHostTime origin: Double) -> [CursorShapeChange] {
        let changes = state.withLock { $0.changes }
        var out: [CursorShapeChange] = []
        out.reserveCapacity(changes.count)
        var carry: CursorShape?
        for c in changes {
            let t = c.t - origin
            if t <= 0 {
                carry = c.shape
                continue
            }
            if let shape = carry {
                out.append(CursorShapeChange(t: 0, shape: shape))
                carry = nil
            }
            let r = (t * 1e4).rounded() / 1e4
            if out.last?.shape != c.shape { out.append(CursorShapeChange(t: r, shape: c.shape)) }
        }
        if out.isEmpty, let shape = carry { out.append(CursorShapeChange(t: 0, shape: shape)) }
        return out
    }
}

// MARK: - System sampler

/// Reads the system-wide cursor with `NSCursor.currentSystem` (`+[NSCursor currentSystemCursor]`).
///
/// ## Permissions (macOS 15)
/// No TCC prompt and no entitlement. The call returns the cursor the window server is showing for
/// whichever app is frontmost; it was verified on macOS 15.7 to return the live system cursor. The app
/// already holds Screen Recording for the capture itself, so even if a later macOS gated this call behind
/// that permission, recording would be unaffected. Apple marks the property "to be deprecated" and
/// documents that it may return nil in a future macOS: then every sample is skipped, `CursorTrack.shapes`
/// stays empty and playback draws the arrow, exactly like a Round 1 recording.
///
/// ## Cost
/// About 0.2 ms per call plus a 16 x 16 draw, so it runs at about 30 Hz on its own utility queue, never in
/// the 120 Hz position loop.
public enum SystemCursorShapeSampler {
    /// Builds the standard-cursor fingerprints once and returns a sampler that identifies the current
    /// system cursor. Returns nil when the standard cursors cannot be fingerprinted (no window server
    /// connection), in which case shape logging is skipped.
    public static func make() -> CursorShapeSampler? {
        let identifier = CursorShapeIdentifier.standard()
        guard identifier.references.contains(where: { $0.shape == .arrow }) else { return nil }
        return {
            guard let cursor = NSCursor.currentSystem else { return nil }
            return identifier.identify(CursorFingerprint.make(cursor))
        }
    }
}
