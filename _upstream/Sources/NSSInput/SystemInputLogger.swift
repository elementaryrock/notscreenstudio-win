import AppKit
import CoreGraphics
import Foundation
import NSSCore
import QuartzCore

/// Real `InputLogger`: samples the cursor at `sampleRate` Hz (120 by default) on a dedicated
/// user-interactive `DispatchSourceTimer` and records mouse button events with `NSEvent` monitors.
///
/// ## Permissions (macOS 15)
/// None. The cursor is read with `CGEvent(source: nil)?.location`, and global `NSEvent` monitors for
/// *mouse* events (move, down, up) are not gated. Only global monitors for *keyboard* events need
/// Accessibility / Input Monitoring, and this logger never listens to the keyboard. Display and window
/// bounds are readable without Screen Recording permission. Note that a global monitor does not see events
/// delivered to this app's own windows, so a local monitor is installed too. The cursor shape is read with
/// `NSCursor.currentSystem`, which needs no permission either (see `SystemCursorShapeSampler`).
///
/// ## Cursor shapes
/// The shape (arrow, I-beam, pointing hand, ...) is sampled at `shapeRate` Hz (30 by default) on its own
/// utility queue, separate from the 120 Hz position timer, and stored in `CursorTrack.shapes` as change events.
///
/// ## Clock
/// Samples and clicks are stamped on the `CACurrentMediaTime()` host clock, the same clock as
/// `RecordingAssets.startHostTime`. `stop(recordingStartHostTime:)` rebases them so t = 0 is the first screen frame.
public actor SystemInputLogger: InputLogger {
    private let schedule: SamplingSchedule
    private let geometry: any ScreenGeometryProvider
    private let monitorClicks: Bool
    private let shapeRate: Double
    private let shapeSampler: CursorShapeSampler?
    private let logCursorShapes: Bool
    private var recorder: CursorRecorder?
    private var shapeRecorder: CursorShapeRecorder?
    private var shapeTimer: DispatchSourceTimer?
    private var timer: DispatchSourceTimer?
    private var refreshTimer: DispatchSourceTimer?
    private var monitors: ClickMonitors?
    private let queue = DispatchQueue(label: "notscreenstudio.input.sampler", qos: .userInteractive)
    private let refreshQueue = DispatchQueue(label: "notscreenstudio.input.geometry", qos: .utility)
    private let shapeQueue = DispatchQueue(label: "notscreenstudio.input.cursorShape", qos: .utility)

    /// - Parameters:
    ///   - logCursorShapes: record `CursorTrack.shapes`. When false, `shapes` stays empty (drawn as the arrow).
    ///   - shapeRate: cursor shape sampling rate in Hz.
    ///   - shapeSampler: reads the current shape; nil uses `SystemCursorShapeSampler.make()`, built at `start`.
    public init(
        sampleRate: Double = 120,
        geometry: any ScreenGeometryProvider = SystemScreenGeometry(),
        monitorClicks: Bool = true,
        logCursorShapes: Bool = true,
        shapeRate: Double = 30,
        shapeSampler: CursorShapeSampler? = nil
    ) {
        self.schedule = SamplingSchedule(rate: sampleRate)
        self.geometry = geometry
        self.monitorClicks = monitorClicks
        self.logCursorShapes = logCursorShapes
        self.shapeRate = SamplingSchedule(rate: shapeRate).rate
        self.shapeSampler = shapeSampler
    }

    public func start(source: CaptureSource) async throws {
        guard recorder == nil else { throw NSSError.unsupported("InputLogger already running") }
        let recorder = CursorRecorder(
            source: source,
            geometry: geometry,
            schedule: schedule,
            position: { CGEvent(source: nil)?.location },
            clock: { CACurrentMediaTime() }
        )
        guard geometry.globalRect(for: source) != nil else {
            throw NSSError.invalidProject("Capture source \(source) cannot be located on screen")
        }
        self.recorder = recorder

        if monitorClicks {
            let primaryHeight = geometry.primaryDisplayHeight
            let installed = await MainActor.run { ClickMonitors(recorder: recorder, primaryHeight: primaryHeight) }
            // `stop` (or another `start`) may have run while we were suspended. Do not leave monitors behind.
            guard self.recorder === recorder else {
                await MainActor.run { installed.remove() }
                throw CancellationError()
            }
            monitors = installed
        }

        recorder.tick()  // position at t ~ 0 without waiting a full period
        let timer = DispatchSource.makeTimerSource(flags: .strict, queue: queue)
        timer.schedule(
            deadline: .now() + .nanoseconds(schedule.intervalNanoseconds),
            repeating: .nanoseconds(schedule.intervalNanoseconds),
            leeway: .nanoseconds(0)
        )
        timer.setEventHandler { recorder.tick() }
        timer.resume()
        self.timer = timer

        // Window frames are looked up on their own low-rate queue so they cannot add jitter to sampling.
        let refresh = DispatchSource.makeTimerSource(queue: refreshQueue)
        refresh.schedule(deadline: .now() + recorder.refreshInterval, repeating: recorder.refreshInterval)
        refresh.setEventHandler { recorder.refreshRect() }
        refresh.resume()
        refreshTimer = refresh

        if logCursorShapes {
            startShapeSampling()
        }
    }

    private func startShapeSampling() {
        // The standard-cursor fingerprints are built once, here, not per sample.
        guard let sampler = shapeSampler ?? SystemCursorShapeSampler.make() else { return }
        let shapes = CursorShapeRecorder(sampler: sampler, clock: { CACurrentMediaTime() })
        shapeRecorder = shapes
        let interval = SamplingSchedule(rate: shapeRate).intervalNanoseconds
        let timer = DispatchSource.makeTimerSource(queue: shapeQueue)
        timer.schedule(deadline: .now(), repeating: .nanoseconds(interval), leeway: .milliseconds(2))
        timer.setEventHandler { shapes.tick() }
        timer.resume()
        shapeTimer = timer
    }

    public func stop(recordingStartHostTime: Double) async -> CursorTrack {
        // Tear down synchronously (before any suspension) so a concurrent `start` sees a consistent state.
        timer?.cancel()
        timer = nil
        refreshTimer?.cancel()
        refreshTimer = nil
        shapeTimer?.cancel()
        shapeTimer = nil
        let shapes = shapeRecorder
        shapeRecorder = nil
        let monitors = self.monitors
        self.monitors = nil
        let recorder = self.recorder
        self.recorder = nil
        if let monitors {
            await MainActor.run { monitors.remove() }
        }
        guard let recorder else { return .empty }
        var track = recorder.finish(recordingStartHostTime: recordingStartHostTime, sampleRate: schedule.rate)
        // Wait for an in-flight shape tick so the recorder is read after its last write.
        shapeQueue.sync {}
        track.shapes = shapes?.finish(recordingStartHostTime: recordingStartHostTime) ?? []
        return track
    }
}

/// Owns the `NSEvent` monitor tokens. Main-actor because AppKit delivers monitor callbacks there.
@MainActor
final class ClickMonitors {
    private var tokens: [Any] = []

    private static let mask: NSEvent.EventTypeMask = [
        .leftMouseDown, .leftMouseUp, .rightMouseDown, .rightMouseUp, .otherMouseDown, .otherMouseUp,
    ]

    init(recorder: CursorRecorder, primaryHeight: Double) {
        let global = NSEvent.addGlobalMonitorForEvents(matching: Self.mask) { event in
            // Global monitors report screen coordinates (bottom-left origin) in locationInWindow.
            let p = CoordinateMath.appKitToTopLeft(event.locationInWindow, primaryDisplayHeight: primaryHeight)
            Self.forward(event, at: p, to: recorder)
        }
        let local = NSEvent.addLocalMonitorForEvents(matching: Self.mask) { event in
            MainActor.assumeIsolated {
                var screen = event.locationInWindow
                if let w = event.window { screen = w.convertPoint(toScreen: screen) }
                let p = CoordinateMath.appKitToTopLeft(screen, primaryDisplayHeight: primaryHeight)
                Self.forward(event, at: p, to: recorder)
            }
            return event
        }
        tokens = [global, local].compactMap { $0 }
    }

    func remove() {
        for t in tokens { NSEvent.removeMonitor(t) }
        tokens = []
    }

    private nonisolated static func forward(_ event: NSEvent, at p: CGPoint, to recorder: CursorRecorder) {
        let button: MouseButton
        let kind: ClickKind
        switch event.type {
        case .leftMouseDown: button = .left; kind = .down
        case .leftMouseUp: button = .left; kind = .up
        case .rightMouseDown: button = .right; kind = .down
        case .rightMouseUp: button = .right; kind = .up
        case .otherMouseDown: button = .other; kind = .down
        case .otherMouseUp: button = .other; kind = .up
        default: return
        }
        // NSEvent.timestamp is seconds since boot. Map it onto the host clock through the offset measured
        // now, so the click keeps its true time even though the monitor callback runs a little late.
        let age = max(0, ProcessInfo.processInfo.systemUptime - event.timestamp)
        let hostTime = CACurrentMediaTime() - (age < 1 ? age : 0)
        recorder.click(at: p, hostTime: hostTime, button: button, kind: kind)
    }
}
