import Foundation
import NSSCore

/// Zoom block editing. Every zoom (auto or manual) is a stored `ZoomBlock`; each edit is one undo step
/// and rebuilds the preview's motion model at once.
extension EditorModel {
    @discardableResult
    public func addZoomBlock(at time: MediaTime? = nil) -> UUID? {
        let t = time ?? playhead
        guard let r = ZoomBlockEditing.adding(at: t, to: project.timeline.zoomBlocks, duration: duration) else { return nil }
        apply("Add Zoom") { $0.timeline.zoomBlocks = r.blocks }
        selectedZoomID = r.id
        return r.id
    }

    public func moveZoomBlock(_ id: UUID, by delta: MediaTime, from base: [ZoomBlock]) {
        apply("Move Zoom") { $0.timeline.zoomBlocks = ZoomBlockEditing.moving(id, by: delta, in: base, duration: $0.assets.duration) }
    }

    public func resizeZoomBlock(_ id: UUID, edge: ZoomBlockEditing.Edge, to t: MediaTime, from base: [ZoomBlock]) {
        apply("Resize Zoom") { $0.timeline.zoomBlocks = ZoomBlockEditing.resizing(id, edge: edge, to: t, in: base, duration: $0.assets.duration) }
    }

    public func deleteZoomBlock(_ id: UUID) {
        apply("Delete Zoom") { $0.timeline.zoomBlocks = ZoomBlockEditing.deleting(id, from: $0.timeline.zoomBlocks) }
        clearZoomSelectionIfGone()
    }

    public func setZoomScale(_ id: UUID, _ scale: Double) {
        apply("Zoom Level") { $0.timeline.zoomBlocks = ZoomBlockEditing.settingScale(id, to: scale, in: $0.timeline.zoomBlocks) }
    }

    public func setZoomFocus(_ id: UUID, _ focus: ZoomFocus) {
        apply("Zoom Focus") { $0.timeline.zoomBlocks = ZoomBlockEditing.settingFocus(id, to: focus, in: $0.timeline.zoomBlocks) }
    }

    public func setZoomEase(_ id: UUID, easeIn: MediaTime? = nil, easeOut: MediaTime? = nil) {
        apply("Zoom Ease") { $0.timeline.zoomBlocks = ZoomBlockEditing.settingEase(id, easeIn: easeIn, easeOut: easeOut, in: $0.timeline.zoomBlocks) }
    }

    public var autoZoomCount: Int { project.timeline.zoomBlocks.filter(\.isAuto).count }

    /// Removes every auto zoom, keeping the manual ones. One undo step.
    public func removeAllAutoZooms() {
        apply("Remove Auto Zooms") { $0.timeline.zoomBlocks = ZoomBlockEditing.removingAuto(from: $0.timeline.zoomBlocks) }
        clearZoomSelectionIfGone()
    }

    /// Replaces the auto zooms with fresh ones from the recorded clicks; manual zooms stay. One undo step.
    public func regenerateAutoZooms() {
        let track = track, aspect = sourceAspect
        apply("Regenerate Auto Zooms") { p in
            p.timeline.zoomBlocks = ZoomBlockEditing.regeneratingAuto(
                in: p.timeline.zoomBlocks, track: track, duration: p.assets.duration, sourceAspect: aspect)
            p.timeline.autoZoomEnabled = false
        }
        clearZoomSelectionIfGone()
    }

    public var selectedZoom: ZoomBlock? {
        guard let id = selectedZoomID else { return nil }
        return project.timeline.zoomBlocks.first { $0.id == id }
    }

    private func clearZoomSelectionIfGone() {
        let ids = Set(project.timeline.zoomBlocks.map(\.id))
        if let s = selectedZoomID, !ids.contains(s) { selectedZoomID = nil }
        if let f = pickingFocusFor, !ids.contains(f) { pickingFocusFor = nil }
    }
}
