import Foundation
import Testing
import NSSCore
import NSSMotion
import NSSRender
import NSSExport
@testable import NSSUI

@MainActor
@Suite struct ZoomEditingTests {
    let clicks = [ClickEvent(t: 2, x: 0.3, y: 0.3, kind: .down), ClickEvent(t: 9, x: 0.8, y: 0.7, kind: .down)]

    func model(timeline: Timeline = Timeline(), clicks: [ClickEvent]? = nil, store: RecordingStore = RecordingStore()) -> EditorModel {
        let assets = RecordingAssets(sourcePixelSize: PixelSize(width: 1920, height: 1080), frameRate: 60, duration: 20)
        let deps = EditorDependencies(store: store, motionFactory: LinearMotionModel.factory, renderer: PassthroughRenderer(), exporter: ScriptedExporter())
        return EditorModel(
            project: Project(name: "Z", assets: assets, timeline: timeline),
            reference: ProjectReference(url: URL(fileURLWithPath: "/tmp/z.nssproj")),
            track: CursorTrack(clicks: clicks ?? self.clicks),
            dependencies: deps
        )
    }

    // MARK: Migration

    @Test func openingMaterializesAutoZoomsOnceAndSavesThem() {
        let store = RecordingStore()
        let m = model(store: store)
        let blocks = m.project.timeline.zoomBlocks
        #expect(blocks.count == 2 && blocks.allSatisfy(\.isAuto))
        #expect(!m.project.timeline.autoZoomEnabled)
        #expect(store.saves.last == m.project)
        #expect(!m.canUndo)
        #expect(m.project != Project(name: "Z", assets: m.project.assets))
        // Opening the migrated project again changes nothing.
        let again = model(timeline: m.project.timeline, store: store)
        #expect(again.project.timeline == m.project.timeline)
        #expect(store.saves.count == 1)
    }

    @Test func migrationKeepsManualBlocksAndTheyWinOverlaps() {
        let manual = ZoomBlock(start: 1, end: 3, scale: 3, focus: .manual(.center))
        let m = model(timeline: Timeline(zoomBlocks: [manual], autoZoomEnabled: true))
        let blocks = m.project.timeline.zoomBlocks
        #expect(blocks.contains(manual))
        for b in blocks where b.isAuto { #expect(b.end <= manual.start || b.start >= manual.end) }
    }

    @Test func autoZoomDisabledProjectGetsNoAutoBlocks() {
        let m = model(timeline: Timeline(autoZoomEnabled: false))
        #expect(m.project.timeline.zoomBlocks.isEmpty)
    }

    // MARK: Auto zoom actions

    @Test func removeAllAutoKeepsManualAndUndoes() {
        let m = model()
        let id = m.addZoomBlock(at: 14)!
        #expect(m.autoZoomCount == 2)
        m.selectedZoomID = m.project.timeline.zoomBlocks.first(where: \.isAuto)!.id
        m.removeAllAutoZooms()
        #expect(m.project.timeline.zoomBlocks.map(\.id) == [id])
        #expect(m.selectedZoomID == nil)
        m.undo()
        #expect(m.autoZoomCount == 2)
    }

    @Test func regenerateReplacesOnlyAuto() {
        let m = model()
        m.removeAllAutoZooms()
        let id = m.addZoomBlock(at: 8.5)!   // overlaps where the second click's auto zoom would go
        m.regenerateAutoZooms()
        let blocks = m.project.timeline.zoomBlocks
        #expect(blocks.contains { $0.id == id && !$0.isAuto })
        #expect(m.autoZoomCount >= 1)
        let manual = blocks.first { $0.id == id }!
        for b in blocks where b.isAuto { #expect(b.end <= manual.start || b.start >= manual.end) }
        for (a, b) in zip(blocks, blocks.dropFirst()) { #expect(a.end <= b.start + 1e-9) }
        m.undo()
        #expect(m.autoZoomCount == 0)
    }

    @Test func editingAnAutoZoomMakesItManual() {
        let m = model()
        let auto = m.project.timeline.zoomBlocks[0]
        m.setZoomScale(auto.id, 3)
        #expect(m.project.timeline.zoomBlocks.first { $0.id == auto.id }?.isAuto == false)
        // A no-op edit leaves the marker alone.
        let other = m.project.timeline.zoomBlocks[1]
        m.setZoomScale(other.id, other.scale)
        #expect(m.project.timeline.zoomBlocks.first { $0.id == other.id }?.isAuto == true)
        m.regenerateAutoZooms()
        #expect(m.project.timeline.zoomBlocks.contains { $0.id == auto.id && $0.scale == 3 })
    }

    @Test func easeIsClampedAndUndoable() {
        let m = model(clicks: [])
        let id = m.addZoomBlock(at: 2)!
        m.setZoomEase(id, easeIn: 0.4)
        m.setZoomEase(id, easeOut: 99)
        #expect(m.selectedZoom?.easeIn == 0.4)
        #expect(m.selectedZoom?.easeOut == ZoomBlockEditing.easeRange.upperBound)
        m.setZoomEase(id, easeIn: .nan)
        #expect(m.selectedZoom?.easeIn == ZoomBlock.defaultEase)
        m.undo(); m.undo()
        #expect(m.selectedZoom?.easeOut == ZoomBlock.defaultEase && m.selectedZoom?.easeIn == 0.4)
    }

    @Test func deleteClearsSelectionAndFocusPicking() {
        let m = model()
        let id = m.project.timeline.zoomBlocks[0].id
        m.selectedZoomID = id
        m.pickingFocusFor = id
        m.deleteZoomBlock(id)
        #expect(m.selectedZoomID == nil && m.pickingFocusFor == nil)
        #expect(m.autoZoomCount == 1)
    }

    @Test func zoomEditsRebuildThePreview() {
        let m = model()
        var seen: [EditorChange] = []
        m.changeHandler = { seen.append($0) }
        let id = m.project.timeline.zoomBlocks[0].id
        m.setZoomFocus(id, .manual(NormalizedPoint(x: 0.1, y: 0.2)))
        m.setZoomEase(id, easeIn: 1)
        m.deleteZoomBlock(id)
        #expect(seen == [.timeline, .timeline, .timeline])
    }

    // MARK: Pure editing rules

    @Test func moveAndResizeStayInBoundsWithoutOverlap() {
        let a = ZoomBlock(start: 2, end: 4), b = ZoomBlock(start: 6, end: 8)
        let moved = ZoomBlockEditing.moving(a.id, by: 10, in: [a, b], duration: 20)
        #expect(moved.first { $0.id == a.id }!.end == 6)
        let left = ZoomBlockEditing.moving(a.id, by: -10, in: [a, b], duration: 20)
        #expect(left.first { $0.id == a.id }!.start == 0)
        let r = ZoomBlockEditing.resizing(a.id, edge: .end, to: 2.05, in: [a, b], duration: 20)
        #expect(abs(r.first { $0.id == a.id }!.end - (2 + ZoomBlockEditing.minLength)) < 1e-9)
        let r2 = ZoomBlockEditing.resizing(b.id, edge: .end, to: 50, in: [a, b], duration: 20)
        #expect(r2.first { $0.id == b.id }!.end == 20)
    }
}
