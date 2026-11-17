import Foundation
import NSSCore
import NSSExport
import Observation

/// What changed, so the preview only rebuilds what it must.
public enum EditorChange: Sendable, Equatable {
    case style
    case timeline
    case project
}

/// Services the editor needs. Real modules are injected through these protocols.
public struct EditorDependencies: Sendable {
    public var store: any ProjectStore
    public var motionFactory: MotionModelFactory
    public var renderer: any FrameRenderer
    public var exporter: any Exporter
    /// Optional: when set, the editor makes the home-screen thumbnail for a project the first time it opens.
    public var library: (any ProjectLibrary)?

    /// Optional: builds a factory for the real source aspect (width / height) of the recording. When set, the
    /// preview uses it instead of `motionFactory`, so auto-zoom framing is right for non-16:9 sources.
    public var motionFactoryForAspect: (@Sendable (_ sourceAspect: Double) -> MotionModelFactory)?

    public init(
        store: any ProjectStore, motionFactory: @escaping MotionModelFactory, renderer: any FrameRenderer, exporter: any Exporter,
        motionFactoryForAspect: (@Sendable (_ sourceAspect: Double) -> MotionModelFactory)? = nil,
        library: (any ProjectLibrary)? = nil
    ) {
        self.library = library
        self.motionFactoryForAspect = motionFactoryForAspect
        self.store = store
        self.motionFactory = motionFactory
        self.renderer = renderer
        self.exporter = exporter
    }
}

/// View model for one open project. All edits go through `apply` so they are undoable, refresh the
/// preview and trigger an autosave.
@MainActor
@Observable
public final class EditorModel {
    public private(set) var project: Project
    public let reference: ProjectReference
    public let track: CursorTrack
    public let dependencies: EditorDependencies
    public let undoManager = UndoManager()

    /// Playhead in seconds. Written by the preview / scrubbing, read by the timeline.
    public var playhead: MediaTime = 0
    /// Where the playhead is drawn. Follows `playhead` exactly while playing or scrubbing and glides after
    /// jumps (see `PlayheadEasing`). Advanced by the preview's display link.
    public private(set) var displayPlayhead: MediaTime = 0
    /// True while the user drags the playhead; the drawn playhead then follows the mouse with no easing.
    public private(set) var isScrubbing = false
    /// Set when `saveNow()` fails, so the view can show it instead of swallowing it.
    public var saveError: String?
    public var isPlaying = false
    public var selectedZoomID: UUID?
    /// When set, the next click in the preview picks the manual focus for this zoom block.
    public var pickingFocusFor: UUID?
    /// Bumped on every undo/redo-able change so menus can refresh.
    public private(set) var revision = 0

    @ObservationIgnored private var easing = PlayheadEasing()
    /// The project as last written (or loaded). `saveNow()` skips the write when nothing differs from it.
    @ObservationIgnored private var savedSnapshot: Project
    @ObservationIgnored public var changeHandler: ((EditorChange) -> Void)?
    @ObservationIgnored private var gestureBase: Project?
    @ObservationIgnored private var gestureName = ""
    @ObservationIgnored private var lastEdit: (key: String?, time: Date) = (nil, .distantPast)
    /// Injectable for tests.
    @ObservationIgnored var clock: () -> Date = { Date() }
    static let coalescingWindow: TimeInterval = 0.6

    public init(project: Project, reference: ProjectReference, track: CursorTrack, dependencies: EditorDependencies) {
        var project = project
        let loaded = project
        // Auto-zooms become stored blocks once, the first time a project opens (see `ZoomBlockEditing`).
        if let migrated = ZoomBlockEditing.materializingAutoZooms(
            project.timeline, track: track, duration: project.assets.duration, sourceAspect: project.assets.sourcePixelSize.aspect
        ) {
            project.timeline = migrated
        }
        self.project = project
        // The migration is part of loading, not a user edit: it is autosaved below, and `saveNow()` on an
        // untouched project still writes nothing.
        self.savedSnapshot = project
        self.reference = reference
        self.track = track
        self.dependencies = dependencies
        undoManager.groupsByEvent = false
        undoManager.levelsOfUndo = 200
        if project != loaded { dependencies.store.scheduleAutosave(project, to: reference) }
    }

    public var duration: MediaTime { project.assets.duration }
    public var trim: TrimRange { project.timeline.trim }
    public var trimStart: MediaTime { project.timeline.trim.start }
    public var trimEnd: MediaTime { project.timeline.trim.resolvedEnd(duration: duration) }
    public var sourceAspect: Double { project.assets.sourcePixelSize.aspect }

    public var canUndo: Bool { _ = revision; return undoManager.canUndo }
    public var canRedo: Bool { _ = revision; return undoManager.canRedo }

    // MARK: Editing core

    /// Applies `mutate` as one undoable step. Inside a gesture (`beginGesture`) the change is applied
    /// live and recorded as a single undo step when the gesture ends.
    public func apply(_ name: String, coalescingKey: String? = nil, _ mutate: (inout Project) -> Void) {
        var next = project
        mutate(&next)
        guard next != project else { return }
        let now = clock()
        defer { lastEdit = (coalescingKey, now) }
        if gestureBase != nil {
            commit(next)
        } else if let key = coalescingKey, lastEdit.key == key, now.timeIntervalSince(lastEdit.time) < Self.coalescingWindow, undoManager.canUndo {
            // Controls without begin/end callbacks (ColorPicker): rapid edits of one control share the undo step already registered.
            commit(next)
        } else {
            replaceProject(with: next, actionName: name)
        }
    }

    /// Start a continuous edit (slider drag, block drag). Call `endGesture` when done.
    public func beginGesture(_ name: String) {
        guard gestureBase == nil else { return }
        gestureBase = project
        gestureName = name
    }

    public func endGesture() {
        guard let base = gestureBase else { return }
        gestureBase = nil
        guard base != project else { return }
        registerUndo(restoring: base, actionName: gestureName)
    }

    private func replaceProject(with next: Project, actionName: String) {
        registerUndo(restoring: project, actionName: actionName)
        commit(next)
    }

    private func registerUndo(restoring old: Project, actionName: String) {
        // One explicit group per user edit (groupsByEvent is off so behaviour doesn't depend on the run loop).
        undoManager.beginUndoGrouping()
        undoManager.registerUndo(withTarget: self) { target in
            MainActor.assumeIsolated {
                target.restore(old, actionName: actionName)
            }
        }
        undoManager.setActionName(actionName)
        undoManager.endUndoGrouping()
        revision += 1
    }

    /// Restores `state`; the inverse is registered while undoing/redoing, which makes the step redoable (and vice versa).
    private func restore(_ state: Project, actionName: String) {
        let current = project
        undoManager.registerUndo(withTarget: self) { target in
            MainActor.assumeIsolated {
                target.restore(current, actionName: actionName)
            }
        }
        undoManager.setActionName(actionName)
        commit(state)
        revision += 1
    }

    private func commit(_ next: Project) {
        let old = project
        var next = next
        next.modifiedAt = Date()
        project = next
        playhead = TrimEditing.clamp(playhead, to: next.timeline.trim, duration: next.assets.duration)
        let change: EditorChange
        if old.timeline != next.timeline || old.style.cursor != next.style.cursor || old.assets != next.assets {
            change = .timeline
        } else if old.style != next.style {
            change = .style
        } else {
            change = .project
        }
        changeHandler?(change)
        dependencies.store.scheduleAutosave(next, to: reference)
    }

    public func undo() { undoManager.undo(); revision += 1 }
    public func redo() { undoManager.redo(); revision += 1 }

    // MARK: Style (inspector bindings)

    public func setStyle<V: Equatable>(_ keyPath: WritableKeyPath<Style, V>, _ value: V, name: String = "Change Style", coalescingKey: String? = nil) {
        apply(name, coalescingKey: coalescingKey) { $0.style[keyPath: keyPath] = value }
    }

    // MARK: Project name

    /// Trimmed, non-empty names only; an empty name keeps the old one.
    public func setProjectName(_ raw: String) {
        let name = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty, name != project.name else { return }
        apply("Rename Project") { $0.name = name }
    }

    // MARK: Trim

    public func setTrimStart(_ t: MediaTime) {
        apply("Trim") { p in
            p.timeline.trim = TrimEditing.settingStart(t, in: p.timeline.trim, duration: p.assets.duration)
        }
        playhead = TrimEditing.clamp(playhead, to: trim, duration: duration)
    }

    public func setTrimEnd(_ t: MediaTime) {
        apply("Trim") { p in
            p.timeline.trim = TrimEditing.settingEnd(t, in: p.timeline.trim, duration: p.assets.duration)
        }
        playhead = TrimEditing.clamp(playhead, to: trim, duration: duration)
    }

    // MARK: Preview interactions

    /// Called with an output-normalized click point in the preview when picking a manual focus.
    /// Returns true when the click was consumed.
    public func pickFocus(atOutput p: NormalizedPoint, camera: CameraState) -> Bool {
        guard let id = pickingFocusFor else { return false }
        let aspect = project.style.aspectRatio.ratio
        if let s = PreviewGeometry.sourcePoint(forOutput: p, outputAspect: aspect, sourceAspect: sourceAspect, padding: project.style.padding, camera: camera) {
            setZoomFocus(id, .manual(s))
        }
        pickingFocusFor = nil
        return true
    }

    public func moveCameraBubble(toOutput p: NormalizedPoint) {
        let s = project.style
        let c = PreviewGeometry.clampedBubbleCenter(p, size: s.camera.size, outputAspect: s.aspectRatio.ratio)
        apply("Move Camera") { $0.style.camera.position = c }
    }

    // MARK: Playhead

    public func seek(to t: MediaTime) {
        playhead = TrimEditing.clamp(t, to: trim, duration: duration)
    }

    /// Playhead drag: seeks and makes the drawn playhead follow the pointer exactly.
    public func scrub(to t: MediaTime) {
        isScrubbing = true
        seek(to: t)
    }

    /// Ends a playhead drag. Must also run when the drag is cancelled (no `onEnded`), or the playhead stays stuck to `playhead`.
    public func endScrub() { isScrubbing = false }

    /// Writes the project now unless it is unchanged since it was loaded or last saved. Returns whether a write happened.
    @discardableResult
    public func saveNow() async throws -> Bool {
        let snapshot = project
        guard snapshot != savedSnapshot else { return false }
        try await dependencies.store.save(snapshot, to: reference)
        savedSnapshot = snapshot
        return true
    }

    /// Advances the drawn playhead by one display frame. Called from the display link with the frame interval.
    public func advanceDisplayPlayhead(dt: Double) {
        if isPlaying || isScrubbing {
            easing.snap(to: playhead)
        } else {
            easing.glide(to: playhead)
            easing.advance(by: dt)
        }
        if easing.value != displayPlayhead { displayPlayhead = easing.value }
    }

    /// Frame time step in seconds for frame-stepping.
    var motionFactoryForCurrentAspect: MotionModelFactory {
        dependencies.motionFactoryForAspect?(sourceAspect) ?? dependencies.motionFactory
    }

    public var frameDuration: MediaTime { 1 / max(project.assets.frameRate, 1) }
}
