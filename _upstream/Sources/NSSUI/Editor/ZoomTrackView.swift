import SwiftUI
import NSSCore

/// The Zoom track lane: every stored zoom (auto and manual) as a block, drawn with the shared
/// `TrackBlockBackground` and `TrackBlockStyle` hit zones.
///
/// - Click a block to select it (Delete or Backspace on the timeline removes it).
/// - Drag a block's body to move it, drag either end to resize it. Neighbours, the timeline ends and
///   `ZoomBlockEditing.minLength` clamp both, so blocks never overlap.
/// - Double-click an empty part of the lane to add a zoom there.
struct ZoomTrackView: View {
    let model: EditorModel
    let geometry: TimelineGeometry
    /// Coordinate space of the timeline lanes, whose x origin matches this lane's.
    let space: String
    /// Called when a zoom gets selected (or the selection is cleared), so other tracks can deselect.
    var onSelect: () -> Void = {}

    @State private var dragBase: [ZoomBlock]?
    @State private var dragZone: TrackBlockStyle.HitZone?

    var body: some View {
        let g = geometry
        ZStack(alignment: .topLeading) {
            TrackRail()
                .onTapGesture(count: 2, coordinateSpace: .named(space)) { loc in
                    if let id = model.addZoomBlock(at: g.time(forX: loc.x)) { select(id) }
                }
                .onTapGesture { select(nil) }
            ForEach(model.project.timeline.zoomBlocks) { block in
                blockView(block)
            }
        }
        .overlay(alignment: .leading) {
            if model.project.timeline.zoomBlocks.isEmpty {
                Text("Double-click to add a zoom").editorLabel(.caption).opacity(0.7).padding(.leading, 8).allowsHitTesting(false)
            }
        }
    }

    private func select(_ id: UUID?) {
        model.selectedZoomID = id
        onSelect()
    }

    private func blockView(_ block: ZoomBlock) -> some View {
        let g = geometry
        let x = g.x(for: block.start)
        let w = TrackBlockStyle.displayWidth(g.x(for: block.end) - x)
        let selected = block.id == model.selectedZoomID
        let fixed: Bool = { if case .manual = block.focus { return true } else { return false } }()
        return ZStack {
            TrackBlockBackground(kind: .zoom, selected: selected, showsGrips: true)
            if w > 40 {
                HStack(spacing: 3) {
                    Image(systemName: fixed ? "scope" : "cursorarrow.motionlines")
                    Text(String(format: "%.1f×", block.scale)).monospacedDigit()
                }
                .font(.system(size: 10, weight: .semibold)).foregroundStyle(.white).lineLimit(1)
                .padding(.horizontal, 8)
            }
        }
        .overlay(alignment: .topTrailing) {
            // Subtle marker for zooms made from clicks; otherwise auto and manual look the same.
            if block.isAuto && w > 22 {
                Image(systemName: "sparkle").font(.system(size: 7, weight: .bold)).foregroundStyle(.white.opacity(0.75))
                    .padding(.top, 3).padding(.trailing, 7).allowsHitTesting(false)
            }
        }
        .frame(width: w, height: TrackBlockStyle.blockHeight)
        .contentShape(Rectangle())
        .offset(x: x, y: TrackBlockStyle.blockInset)
        .onTapGesture { select(block.id) }
        .gesture(
            DragGesture(minimumDistance: TrackBlockStyle.dragThreshold, coordinateSpace: .named(space))
                .onChanged { v in
                    if dragBase == nil {
                        dragBase = model.project.timeline.zoomBlocks
                        dragZone = TrackBlockStyle.hitZone(atX: v.startLocation.x - x, blockWidth: w)
                        select(block.id)
                        model.beginGesture(dragZone == .body ? "Move Zoom" : "Resize Zoom")
                    }
                    let base = dragBase ?? []
                    switch dragZone ?? .body {
                    case .body: model.moveZoomBlock(block.id, by: v.translation.width / g.pixelsPerSecond, from: base)
                    case .startEdge: model.resizeZoomBlock(block.id, edge: .start, to: g.time(forX: v.location.x), from: base)
                    case .endEdge: model.resizeZoomBlock(block.id, edge: .end, to: g.time(forX: v.location.x), from: base)
                    }
                }
                .onEnded { _ in
                    dragBase = nil
                    dragZone = nil
                    model.endGesture()
                }
        )
        .help(block.isAuto ? "Auto zoom from clicks" : "Zoom")
    }
}

/// The Zoom track's label with its two actions: remove all auto zooms, and regenerate them from clicks.
struct ZoomTrackLabel: View {
    let model: EditorModel

    var body: some View {
        let autoCount = model.autoZoomCount
        let hasClicks = model.track.clicks.contains { $0.kind == .down }
        HStack(spacing: 2) {
            TrackLabel(.zoom)
            Button { model.removeAllAutoZooms() } label: { Image(systemName: "trash") }
                .disabled(autoCount == 0)
                .help(autoCount == 0 ? "Remove all auto zooms (there are none)"
                                     : "Remove all auto zooms (\(autoCount)). Your own zooms stay.")
                .accessibilityLabel("Remove all auto zooms")
            Button { model.regenerateAutoZooms() } label: { Image(systemName: "wand.and.stars") }
                .disabled(!hasClicks)
                .help(hasClicks ? "Regenerate auto zooms from clicks. Your own zooms stay."
                                : "Regenerate auto zooms from clicks (this recording has no clicks)")
                .accessibilityLabel("Regenerate auto zooms from clicks")
        }
        .buttonStyle(EditorIconButtonStyle())
        .padding(.trailing, 4)
    }
}
