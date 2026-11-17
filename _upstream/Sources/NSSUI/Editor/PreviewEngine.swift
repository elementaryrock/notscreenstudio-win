import AVFoundation
import AppKit
import NSSCore
import NSSRender
import QuartzCore

/// Wraps a non-Sendable value so it can cross to the render queue. Safe because each wrapped value is
/// handed to exactly one queue block at a time.
struct Unchecked<T>: @unchecked Sendable { let value: T }

/// Lock-protected current motion model; swapped when the timeline or cursor style changes.
final class MotionBox: @unchecked Sendable {
    private let lock = NSLock()
    private var model: any MotionModel
    init(_ model: any MotionModel) { self.model = model }
    func get() -> any MotionModel { lock.lock(); defer { lock.unlock() }; return model }
    func set(_ m: any MotionModel) { lock.lock(); model = m; lock.unlock() }
}

struct IdentityMotion: MotionModel {
    func frameState(at t: MediaTime) -> FrameState { FrameState(time: t) }
}

/// Drives playback and preview rendering. `tick(targetTimestamp:)` is called from the view's display link.
/// Per tick: read playhead, pull frames from `AVPlayerItemVideoOutput`, then render on a background queue into
/// a small pool of reused IOSurface-backed buffers. If the renderer is still busy the tick is dropped.
@MainActor
public final class PreviewEngine {
    public static let poolSize = 3

    private let model: EditorModel
    private let screen: PlayerVideoTrack
    private let camera: PlayerVideoTrack?
    private let mic: AVPlayer?
    private let renderer: any FrameRenderer
    private let motion: MotionBox
    private let renderQueue = DispatchQueue(label: "nss.preview.render", qos: .userInteractive)

    private var pool: [CVPixelBuffer] = []
    private var poolIndex = 0
    private var poolSize = PixelSize(width: 0, height: 0)
    private var renderBusy = false
    private var dirty = true
    private var awaitingFrame = 0
    private var motionBuilding = false
    private var motionPending = false
    /// Number of motion builds started (diagnostics / tests).
    private(set) var motionBuildCount = 0
    /// True when no motion build is running or queued, so `tick` renders with the model for the latest timeline.
    var motionIsCurrent: Bool { !motionBuilding && !motionPending }
    /// Last playhead the players were seeked to (paused playhead edits must match it).
    private(set) var lastSeekTarget: MediaTime = 0
    private var tickCount = 0
    private var syncCheckCount = 0
    private var itemEnded = false
    private var endObserver: NSObjectProtocol?

    /// Called on the main actor with each finished buffer (the view sets it as layer contents).
    public var present: ((CVPixelBuffer) -> Void)?
    /// Camera state used for the frame currently on screen (for click -> source mapping).
    public private(set) var displayedCamera = CameraState.identity
    public private(set) var droppedFrames = 0
    public private(set) var lastRenderError: String?

    public init(model: EditorModel) {
        self.model = model
        let base = model.reference.url
        let assets = model.project.assets
        screen = PlayerVideoTrack(url: base.appendingPathComponent(assets.screenFile), muted: false)
        camera = assets.cameraFile.map { PlayerVideoTrack(url: base.appendingPathComponent($0), muted: true) }
        mic = assets.micFile.map {
            let p = AVPlayer(url: base.appendingPathComponent($0))
            p.automaticallyWaitsToMinimizeStalling = false
            return p
        }
        renderer = model.dependencies.renderer
        motion = MotionBox(IdentityMotion())
        endObserver = NotificationCenter.default.addObserver(
            forName: AVPlayerItem.didPlayToEndTimeNotification, object: screen.player.currentItem, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.itemEnded = true }
        }
        rebuildMotion()
        seek(to: model.trimStart)
    }

    /// Where playback really ends: the trim end, or earlier if the screen file is shorter than the recorded duration.
    private var effectiveEnd: MediaTime {
        let d = screen.player.currentItem?.duration.seconds ?? .nan
        return d.isFinite && d > 0 ? min(model.trimEnd, d - 0.02) : model.trimEnd
    }

    // MARK: Model changes

    public func modelChanged(_ change: EditorChange) {
        if change == .timeline { rebuildMotion() }
        // Trim edits and undo/redo can move or strand the playhead: keep the players on the same time.
        let clamped = TrimEditing.clamp(model.playhead, to: model.trim, duration: model.duration)
        if clamped != model.playhead { model.playhead = clamped }
        if !model.isPlaying, abs(clamped - lastSeekTarget) > 1e-6 || clamped != model.playhead { seek(to: clamped) }
        dirty = true
    }

    /// At most one build runs at a time; requests that arrive meanwhile collapse into one follow-up build
    /// that reads the latest project state when it starts (stale builds are never started).
    private func rebuildMotion() {
        if motionBuilding {
            motionPending = true
            return
        }
        motionBuilding = true
        motionBuildCount += 1
        let factory = model.motionFactoryForCurrentAspect
        let track = model.track
        let timeline = model.project.timeline
        let style = model.project.style.cursor
        let duration = model.duration
        let box = motion
        Task { @MainActor [weak self] in
            let built = await Task.detached(priority: .userInitiated) { factory(track, timeline, style, duration) }.value
            guard let self else { return }
            box.set(built)
            self.dirty = true
            self.motionBuilding = false
            if self.motionPending {
                self.motionPending = false
                self.rebuildMotion()
            }
        }
    }

    // MARK: Transport

    public func play() {
        itemEnded = false
        if model.playhead >= effectiveEnd - 0.01 { seek(to: model.trimStart) }
        itemEnded = false
        let t = CMTime(seconds: model.playhead, preferredTimescale: 600)
        let players = allPlayers
        if screen.player.currentItem?.status == .readyToPlay {
            let host = CMTimeAdd(CMClockGetTime(CMClockGetHostTimeClock()), CMTime(seconds: 0.15, preferredTimescale: 600))
            for p in players where p.currentItem?.status == .readyToPlay { p.setRate(1, time: t, atHostTime: host) }
        } else {
            for p in players { p.seek(to: t, toleranceBefore: .zero, toleranceAfter: .zero); p.play() }
        }
        model.isPlaying = true
    }

    public func pause() {
        for p in allPlayers { p.pause() }
        model.isPlaying = false
        model.playhead = clampedPlayerTime()
        lastSeekTarget = model.playhead
        dirty = true
    }

    public func seek(to time: MediaTime) {
        let t = TrimEditing.clamp(time, to: model.trim, duration: model.duration)
        let cm = CMTime(seconds: t, preferredTimescale: 600)
        for p in allPlayers { p.seek(to: cm, toleranceBefore: .zero, toleranceAfter: .zero) }
        model.playhead = t
        lastSeekTarget = t
        awaitingFrame = 30
        dirty = true
    }

    public func step(frames: Int) {
        if model.isPlaying { pause() }
        seek(to: model.playhead + Double(frames) * model.frameDuration)
    }

    public func teardown() {
        for p in allPlayers { p.pause() }
        if let o = endObserver { NotificationCenter.default.removeObserver(o) }
        endObserver = nil
        present = nil
    }

    private var allPlayers: [AVPlayer] {
        var a = [screen.player]
        if let c = camera { a.append(c.player) }
        if let m = mic { a.append(m) }
        return a
    }

    private func clampedPlayerTime() -> MediaTime {
        let s = screen.player.currentTime().seconds
        return s.isFinite ? TrimEditing.clamp(s, to: model.trim, duration: model.duration) : model.playhead
    }

    // MARK: Per-frame

    public func tick(dt: Double = 1.0 / 60.0) {
        tickCount += 1
        defer { model.advanceDisplayPlayhead(dt: dt) }
        var t = model.playhead
        if model.isPlaying {
            let raw = screen.player.currentTime().seconds
            t = raw.isFinite ? raw : t
            if t >= effectiveEnd || itemEnded {
                t = min(max(t, model.trimStart), model.trimEnd)
                pause()
                model.playhead = t
            } else {
                model.playhead = t
                correctDrift(master: t)
            }
        }

        var gotNew = screen.pull(at: t)
        if camera?.pull(at: t) == true { gotNew = true }
        if gotNew { awaitingFrame = 0 }
        let needsRender = model.isPlaying || dirty || gotNew || awaitingFrame > 0
        if awaitingFrame > 0 { awaitingFrame -= 1 }
        guard needsRender, let screenFrame = screen.lastFrame else { return }

        ensurePool()
        guard !pool.isEmpty else { return }
        if renderBusy {
            droppedFrames += 1
            dirty = true
            return
        }
        dirty = false
        renderBusy = true
        let out = Unchecked(value: pool[poolIndex])
        poolIndex = (poolIndex + 1) % pool.count
        let frameTime = t
        let job = Unchecked(value: (screenFrame, camera?.lastFrame))
        let style = model.project.style.resolvingAssets(relativeTo: model.reference.url)
        let renderer = renderer
        let box = motion
        renderQueue.async { [weak self] in
            let state = box.get().frameState(at: frameTime)
            var failure: String?
            do {
                try renderer.render(screen: job.value.0, camera: style.camera.visible ? job.value.1 : nil, state: state, style: style, into: out.value)
            } catch {
                failure = String(describing: error)
            }
            DispatchQueue.main.async {
                MainActor.assumeIsolated {
                    guard let self else { return }
                    self.renderBusy = false
                    self.lastRenderError = failure
                    if failure == nil {
                        self.displayedCamera = state.camera
                        self.present?(out.value)
                    }
                }
            }
        }
    }

    private func correctDrift(master: MediaTime) {
        syncCheckCount += 1
        guard syncCheckCount % 30 == 0 else { return }
        let target = CMTime(seconds: master, preferredTimescale: 600)
        for p in [camera?.player, mic].compactMap({ $0 }) {
            let d = abs(p.currentTime().seconds - master)
            if d.isFinite, d > 0.08 { p.seek(to: target, toleranceBefore: .zero, toleranceAfter: .zero) }
        }
    }

    /// (Re)creates the output pool only when the size changes (aspect ratio edit), never per frame.
    private func ensurePool() {
        let size = Self.previewSize(for: model.project.style.aspectRatio)
        guard size != poolSize || pool.isEmpty else { return }
        pool = (0..<Self.poolSize).compactMap { _ in try? PixelBuffers.make(width: size.width, height: size.height) }
        poolSize = size
        poolIndex = 0
    }

    /// 2/3 of 1080p output, keeping the chosen aspect ratio.
    public static func previewSize(for aspect: AspectRatio) -> PixelSize {
        let full = aspect.outputSize(for: .p1080)
        return PixelSize(width: full.width * 2 / 3, height: full.height * 2 / 3)
    }
}
