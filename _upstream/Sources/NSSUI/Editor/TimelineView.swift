import SwiftUI
import NSSCore

/// Timeline: ruler plus four labelled tracks (Video, Zoom, Camera, Audio) and one playhead across them.
/// Every block on every track is drawn with `TrackBlockBackground` and uses `TrackBlockStyle` hit zones.
struct EditorTimelineView: View {
    let model: EditorModel
    let thumbnails: ThumbnailStore
    let seek: (MediaTime) -> Void

    @State private var selectedStatic: TimelineTrackKind?
    /// Resets to false when a drag ends OR is cancelled (system interruption), unlike `onEnded`.
    @GestureState private var scrubbing = false
    private let space = "timelineTrack"
    private let rulerH: CGFloat = 22
    private let videoH: CGFloat = 52
    private let trackH = TrackBlockStyle.trackHeight
    private let gap: CGFloat = 4
    private let labelW: CGFloat = 120

    private var totalHeight: CGFloat { rulerH + videoH + 3 * trackH + 4 * gap }

    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            VStack(spacing: gap) {
                Color.clear.frame(height: rulerH)
                TrackLabel(.video).frame(height: videoH)
                ZoomTrackLabel(model: model).frame(height: trackH)
                TrackLabel(.camera).frame(height: trackH)
                TrackLabel(.audio).frame(height: trackH)
            }
            .frame(width: labelW)
            GeometryReader { geo in
                let g = TimelineGeometry(duration: model.duration, width: geo.size.width)
                ZStack(alignment: .topLeading) {
                    VStack(spacing: gap) {
                        ruler(g).frame(height: rulerH)
                        videoTrack(g).frame(height: videoH)
                        ZoomTrackView(model: model, geometry: g, space: space, onSelect: { selectedStatic = nil })
                            .frame(height: trackH)
                        staticTrack(.camera, present: model.project.assets.cameraFile != nil, g: g, emptyText: "No camera in this recording")
                        staticTrack(.audio, present: model.project.assets.micFile != nil, g: g, emptyText: "No audio in this recording")
                    }
                    playhead(g)
                }
                .coordinateSpace(name: space)
                .contentShape(Rectangle())
            }
        }
        .frame(height: totalHeight)
        .focusable()
        .focusEffectDisabled()
        .onKeyPress(.delete) { deleteSelected() }
        .onKeyPress(.deleteForward) { deleteSelected() }
        .onChange(of: scrubbing) { _, now in if !now { model.endScrub() } }
        .onDisappear { model.endScrub() }
    }

    private func deleteSelected() -> KeyPress.Result {
        guard let id = model.selectedZoomID else { return .ignored }
        model.deleteZoomBlock(id)
        return .handled
    }

    // MARK: Ruler

    private func ruler(_ g: TimelineGeometry) -> some View {
        Canvas { ctx, size in
            let step = g.tickInterval()
            var t: MediaTime = 0
            while t <= g.duration {
                let x = g.x(for: t)
                var p = Path()
                p.move(to: CGPoint(x: x, y: size.height - 7))
                p.addLine(to: CGPoint(x: x, y: size.height))
                ctx.stroke(p, with: .color(EditorTheme.textTertiary), lineWidth: 1)
                ctx.draw(Text(TimelineGeometry.label(for: t)).font(.system(size: 9).monospacedDigit()).foregroundStyle(EditorTheme.textSecondary),
                         at: CGPoint(x: x + 3, y: 8), anchor: .leading)
                t += step
            }
        }
        .contentShape(Rectangle())
                .gesture(scrubGesture(g))
    }

    /// Click or drag on the ruler or strip: seeks at once and the drawn playhead follows the mouse exactly.
    private func scrubGesture(_ g: TimelineGeometry) -> some Gesture {
        DragGesture(minimumDistance: 0, coordinateSpace: .named(space))
            .updating($scrubbing) { _, state, _ in state = true }
            .onChanged {
                model.scrub(to: g.time(forX: $0.location.x))
                seek(g.time(forX: $0.location.x))
            }
            .onEnded { _ in model.endScrub() }
    }

    // MARK: Video track (thumbnails, trim, click markers)

    private func videoTrack(_ g: TimelineGeometry) -> some View {
        ZStack(alignment: .leading) {
            Color.black.opacity(0.35)
            ForEach(thumbnails.thumbs) { th in
                let w = g.width / 24
                Image(decorative: th.image, scale: 1)
                    .resizable().aspectRatio(contentMode: .fill)
                    .frame(width: w, height: videoH).clipped()
                    .offset(x: g.x(for: th.time) - w / 2)
            }
            // Dim trimmed-away regions.
            let sx = g.x(for: model.trimStart), ex = g.x(for: model.trimEnd)
            Rectangle().fill(.black.opacity(0.6)).frame(width: max(sx, 0)).frame(maxWidth: .infinity, alignment: .leading)
            Rectangle().fill(.black.opacity(0.6)).frame(width: max(g.width - ex, 0)).frame(maxWidth: .infinity, alignment: .trailing)
            Rectangle().strokeBorder(EditorTheme.trim, lineWidth: 2)
                .frame(width: max(ex - sx, 0)).offset(x: sx).allowsHitTesting(false)
            clickMarkers(g).frame(height: 10).frame(maxHeight: .infinity, alignment: .bottom)
            trimHandle(x: sx, isStart: true, g: g)
            trimHandle(x: ex, isStart: false, g: g)
        }
        .clipShape(RoundedRectangle(cornerRadius: TrackBlockStyle.cornerRadius))
        .overlay(RoundedRectangle(cornerRadius: TrackBlockStyle.cornerRadius).strokeBorder(TimelineTrackKind.video.tint.opacity(0.6)))
        .contentShape(Rectangle())
        .gesture(scrubGesture(g))
    }

    private func trimHandle(x: Double, isStart: Bool, g: TimelineGeometry) -> some View {
        RoundedRectangle(cornerRadius: 2).fill(EditorTheme.trim)
            .frame(width: 8, height: videoH)
            .offset(x: x - (isStart ? 8 : 0))
            .gesture(
                DragGesture(minimumDistance: 0, coordinateSpace: .named(space))
                    .onChanged { v in
                        model.beginGesture("Trim")
                        let t = g.time(forX: v.location.x)
                        if isStart { model.setTrimStart(t) } else { model.setTrimEnd(t) }
                    }
                    .onEnded { _ in model.endGesture() }
            )
            .help(isStart ? "Trim start" : "Trim end")
    }

    private func clickMarkers(_ g: TimelineGeometry) -> some View {
        Canvas { ctx, size in
            for c in model.track.clicks where c.kind == .down && c.t <= g.duration {
                let x = g.x(for: c.t)
                ctx.fill(Path(ellipseIn: CGRect(x: x - 2.5, y: size.height / 2 - 2.5, width: 5, height: 5)), with: .color(.orange))
            }
        }
        .allowsHitTesting(false)
    }

    // MARK: Camera and audio tracks (one full-length block each)

    private func staticTrack(_ kind: TimelineTrackKind, present: Bool, g: TimelineGeometry, emptyText: String) -> some View {
        ZStack(alignment: .leading) {
            TrackRail()
            if present {
                ZStack {
                    TrackBlockBackground(kind: kind, selected: selectedStatic == kind)
                    Text(kind == .camera ? "Camera" : "Microphone").font(.system(size: 10, weight: .semibold)).foregroundStyle(.white).lineLimit(1)
                }
                .frame(width: g.width, height: TrackBlockStyle.blockHeight)
                .padding(.vertical, TrackBlockStyle.blockInset)
                .contentShape(Rectangle())
                .onTapGesture { selectedStatic = kind; model.selectedZoomID = nil }
            } else {
                Text(emptyText).editorLabel(.caption).opacity(0.7).padding(.leading, 8).allowsHitTesting(false)
            }
        }
    }

    // MARK: Playhead

    private func playhead(_ g: TimelineGeometry) -> some View {
        PlayheadLine(model: model, geometry: g)
    }
}

/// The drawn playhead. The only reader of `displayPlayhead`, so its per-frame changes invalidate just this
/// small view and not the ruler, strip, markers or zoom track.
private struct PlayheadLine: View {
    let model: EditorModel
    let geometry: TimelineGeometry

    var body: some View {
        let x = geometry.x(for: model.displayPlayhead)
        ZStack(alignment: .top) {
            Rectangle().fill(EditorTheme.playhead).frame(width: 1.5)
            Image(systemName: "arrowtriangle.down.fill").font(.system(size: 9)).foregroundStyle(EditorTheme.playhead).offset(y: -1)
        }
        .frame(width: 12)
        .offset(x: x - 6)
        .allowsHitTesting(false)
    }
}
