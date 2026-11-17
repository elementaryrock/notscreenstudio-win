import Foundation
import Testing
import NSSCore
import NSSMotion
@testable import NSSUI

@Suite struct ZoomBlockEditingTests {
    let dur: MediaTime = 20

    @Test func addShrinksToFreeGapAndSorts() {
        let a = ZoomBlock(start: 5, end: 7)
        let r = ZoomBlockEditing.adding(at: 4, to: [a], duration: dur)
        #expect(r != nil)
        let b = r!.blocks.first { $0.id == r!.id }!
        #expect(b.start == 4 && b.end == 5 + 0 && b.end - b.start >= ZoomBlockEditing.minLength)
        #expect(r!.blocks.map(\.start) == [4, 5])
    }

    @Test func addInsideExistingBlockIsRejected() {
        #expect(ZoomBlockEditing.adding(at: 6, to: [ZoomBlock(start: 5, end: 7)], duration: dur) == nil)
    }

    @Test func addNearEndKeepsBlockInsideDuration() {
        let r = ZoomBlockEditing.adding(at: 19.9, to: [], duration: dur)!
        let b = r.blocks[0]
        #expect(b.end <= dur && b.end - b.start >= ZoomBlockEditing.minLength - 1e-9)
    }

    @Test func movingStopsAtNeighboursAndEnds() {
        let a = ZoomBlock(start: 2, end: 4), b = ZoomBlock(start: 6, end: 8)
        let blocks = [a, b]
        let right = ZoomBlockEditing.moving(a.id, by: 10, in: blocks, duration: dur)
        #expect(right.first { $0.id == a.id }!.start == 4)   // stops against b (length 2 -> 4...6)
        let left = ZoomBlockEditing.moving(a.id, by: -10, in: blocks, duration: dur)
        #expect(left.first { $0.id == a.id }!.start == 0)
        let end = ZoomBlockEditing.moving(b.id, by: 100, in: blocks, duration: dur)
        #expect(end.first { $0.id == b.id }!.end == dur)
    }

    @Test func resizeEnforcesMinLengthAndNoOverlap() {
        let a = ZoomBlock(start: 2, end: 4), b = ZoomBlock(start: 6, end: 8)
        let shrunk = ZoomBlockEditing.resizing(a.id, edge: .end, to: 2.01, in: [a, b], duration: dur)
        #expect(abs(shrunk.first { $0.id == a.id }!.end - (2 + ZoomBlockEditing.minLength)) < 1e-9)
        let grown = ZoomBlockEditing.resizing(a.id, edge: .end, to: 50, in: [a, b], duration: dur)
        #expect(grown.first { $0.id == a.id }!.end == 6)
        let startPast = ZoomBlockEditing.resizing(b.id, edge: .start, to: 0, in: [a, b], duration: dur)
        #expect(startPast.first { $0.id == b.id }!.start == 4)
        let startOver = ZoomBlockEditing.resizing(b.id, edge: .start, to: 99, in: [a, b], duration: dur)
        #expect(abs(startOver.first { $0.id == b.id }!.start - (8 - ZoomBlockEditing.minLength)) < 1e-9)
    }

    @Test func deleteScaleAndFocus() {
        let a = ZoomBlock(start: 1, end: 3)
        #expect(ZoomBlockEditing.deleting(a.id, from: [a]).isEmpty)
        #expect(ZoomBlockEditing.settingScale(a.id, to: 99, in: [a])[0].scale == ZoomBlockEditing.scaleRange.upperBound)
        #expect(ZoomBlockEditing.settingScale(a.id, to: 0, in: [a])[0].scale == ZoomBlockEditing.scaleRange.lowerBound)
        let f = ZoomBlockEditing.settingFocus(a.id, to: .manual(NormalizedPoint(x: 2, y: -1)), in: [a])[0].focus
        #expect(f == .manual(NormalizedPoint(x: 1, y: 0)))
    }

    @Test func normalizedDropsTinyAndClips() {
        let blocks = [ZoomBlock(start: 1, end: 3), ZoomBlock(start: 2.9, end: 3.1), ZoomBlock(start: 9, end: 30)]
        let n = ZoomBlockEditing.normalized(blocks, duration: 10)
        #expect(n.count == 2)
        #expect(n[1].end == 10)
    }
}

@Suite struct TrimAndGeometryTests {
    @Test func trimStartClamps() {
        let t = TrimEditing.settingStart(-5, in: TrimRange(), duration: 10)
        #expect(t.start == 0)
        let t2 = TrimEditing.settingStart(99, in: TrimRange(start: 0, end: 8), duration: 10)
        #expect(t2.start == 8 - TrimEditing.minLength)
    }

    @Test func trimEndClampsAndNilsAtFullLength() {
        let t = TrimEditing.settingEnd(0, in: TrimRange(start: 3), duration: 10)
        #expect(t.end == 3 + TrimEditing.minLength)
        #expect(TrimEditing.settingEnd(50, in: TrimRange(start: 3), duration: 10).end == nil)
        #expect(TrimEditing.settingEnd(7, in: TrimRange(start: 3), duration: 10).end == 7)
    }

    @Test func clampToTrim() {
        #expect(TrimEditing.clamp(1, to: TrimRange(start: 2, end: 6), duration: 10) == 2)
        #expect(TrimEditing.clamp(9, to: TrimRange(start: 2, end: 6), duration: 10) == 6)
        #expect(TrimEditing.clamp(9, to: TrimRange(start: 2), duration: 10) == 9)
    }

    @Test func timeXRoundTrip() {
        let g = TimelineGeometry(duration: 30, width: 900)
        #expect(g.pixelsPerSecond == 30)
        #expect(g.x(for: 10) == 300)
        #expect(abs(g.time(forX: g.x(for: 12.34)) - 12.34) < 1e-9)
        #expect(g.time(forX: -50) == 0)
        #expect(g.time(forX: 5000) == 30)
    }

    @Test func tickIntervalGrowsWhenZoomedOut() {
        let wide = TimelineGeometry(duration: 10, width: 1000).tickInterval()
        let narrow = TimelineGeometry(duration: 600, width: 1000).tickInterval()
        #expect(wide < narrow)
        #expect(TimelineGeometry.label(for: 75.3) == "1:15.3")
    }

    @Test func previewGeometryMapsClicksToSource() {
        let center = PreviewGeometry.sourcePoint(forOutput: .center, outputAspect: 16.0 / 9, sourceAspect: 16.0 / 9, padding: 0.06, camera: .identity)
        #expect(center == NormalizedPoint.center)
        // Outside the padded content.
        #expect(PreviewGeometry.sourcePoint(forOutput: NormalizedPoint(x: 0.001, y: 0.5), outputAspect: 16.0 / 9, sourceAspect: 16.0 / 9, padding: 0.06, camera: .identity) == nil)
        // Zoomed 2x on (0.25, 0.25): the centre of the view shows that point.
        let z = PreviewGeometry.sourcePoint(forOutput: .center, outputAspect: 1, sourceAspect: 16.0 / 9, padding: 0, camera: CameraState(scale: 2, center: NormalizedPoint(x: 0.25, y: 0.25)))!
        #expect(abs(z.x - 0.25) < 1e-9 && abs(z.y - 0.25) < 1e-9)
    }

    @Test func cameraBubbleHitAndClamp() {
        let bubble = CameraBubble(position: NormalizedPoint(x: 0.8, y: 0.8), size: 0.2)
        #expect(PreviewGeometry.hitsCameraBubble(NormalizedPoint(x: 0.8, y: 0.8), bubble: bubble, outputAspect: 16.0 / 9))
        #expect(!PreviewGeometry.hitsCameraBubble(NormalizedPoint(x: 0.3, y: 0.3), bubble: bubble, outputAspect: 16.0 / 9))
        let c = PreviewGeometry.clampedBubbleCenter(NormalizedPoint(x: 5, y: -5), size: 0.2, outputAspect: 16.0 / 9)
        #expect(c.x == 0.9 && c.y > 0 && c.y < 0.5)
    }
}

@MainActor
@Suite struct EditorModelTests {
    @Test func styleEditWritesProjectAndAutosaves() {
        let store = RecordingStore()
        let m = makeModel(store: store)
        m.setStyle(\.padding, 0.1)
        #expect(m.project.style.padding == 0.1)
        #expect(store.saves.last?.style.padding == 0.1)
    }

    @Test func undoRedoRoundTrip() {
        let m = makeModel()
        let original = m.project.style.cornerRadius
        m.setStyle(\.cornerRadius, 0.05)
        m.setStyle(\.cornerRadius, 0.07)
        #expect(m.canUndo)
        m.undo()
        #expect(m.project.style.cornerRadius == 0.05)
        m.undo()
        #expect(m.project.style.cornerRadius == original)
        #expect(!m.canUndo && m.canRedo)
        m.redo()
        m.redo()
        #expect(m.project.style.cornerRadius == 0.07)
    }

    @Test func gestureCoalescesIntoOneUndoStep() {
        let m = makeModel()
        m.beginGesture("Padding")
        for v in stride(from: 0.01, through: 0.1, by: 0.01) { m.setStyle(\.padding, v) }
        m.endGesture()
        #expect(m.project.style.padding == 0.1)
        m.undo()
        #expect(m.project.style.padding == Style().padding)
        #expect(!m.canUndo)
    }

    @Test func rapidColorEditsShareOneUndoStepAndSlowOnesDont() {
        let m = makeModel()
        var now = Date(timeIntervalSince1970: 1000)
        m.clock = { now }
        for i in 1...5 {
            m.setStyle(\.background, .solid(RGBAColor(r: Double(i) / 10, g: 0, b: 0)), coalescingKey: "bg")
            now = now.addingTimeInterval(0.1)
        }
        #expect(m.project.style.background == .solid(RGBAColor(r: 0.5, g: 0, b: 0)))
        m.undo()
        #expect(m.project.style.background == Style().background)
        #expect(!m.canUndo)
        // Redo, then a later edit (outside the window) is its own step.
        m.redo()
        now = now.addingTimeInterval(5)
        m.setStyle(\.background, .solid(RGBAColor(r: 0.9, g: 0, b: 0)), coalescingKey: "bg")
        m.undo()
        #expect(m.project.style.background == .solid(RGBAColor(r: 0.5, g: 0, b: 0)))
    }

    @Test func aspectFactoryIsPreferredWhenProvided() {
        let m = makeModel()
        #expect(m.dependencies.motionFactoryForAspect == nil)
        var deps = m.dependencies
        let calls = CallCounter()
        deps.motionFactoryForAspect = { aspect in calls.bump(); return LinearMotionModel.factory }
        let m2 = EditorModel(project: m.project, reference: m.reference, track: m.track, dependencies: deps)
        _ = m2.motionFactoryForCurrentAspect
        #expect(calls.count == 1)
    }

    @Test func noOpEditsDontRegisterUndo() {
        let m = makeModel()
        m.setStyle(\.padding, Style().padding)
        #expect(!m.canUndo)
    }

    @Test func undoAlsoAutosaves() {
        let store = RecordingStore()
        let m = makeModel(store: store)
        m.setStyle(\.padding, 0.1)
        m.undo()
        #expect(store.saves.last?.style.padding == Style().padding)
    }

    @Test func changeKindsDriveThePreview() {
        let m = makeModel()
        var seen: [EditorChange] = []
        m.changeHandler = { seen.append($0) }
        m.setStyle(\.padding, 0.1)
        m.setStyle(\.cursor.size, 3)
        m.removeAllAutoZooms()
        #expect(seen == [.style, .timeline, .timeline])
    }

    @Test func trimMovesPlayheadInside() {
        let m = makeModel(duration: 20)
        m.playhead = 1
        m.setTrimStart(5)
        #expect(m.trimStart == 5 && m.playhead == 5)
        m.setTrimEnd(100)
        #expect(m.project.timeline.trim.end == nil)
        m.setTrimEnd(10)
        m.seek(to: 50)
        #expect(m.playhead == 10)
    }

    @Test func zoomBlockLifecycleWithUndo() {
        let m = makeModel()
        m.removeAllAutoZooms()   // the fixture click at 1 s made an auto zoom on open
        m.playhead = 3
        let id = m.addZoomBlock()
        #expect(id != nil && m.selectedZoomID == id)
        #expect(m.project.timeline.zoomBlocks.count == 1)
        m.setZoomScale(id!, 3)
        m.setZoomFocus(id!, .manual(NormalizedPoint(x: 0.2, y: 0.3)))
        #expect(m.selectedZoom?.scale == 3)
        #expect(m.addZoomBlock(at: 3.5) == nil)   // overlaps
        m.deleteZoomBlock(id!)
        #expect(m.project.timeline.zoomBlocks.isEmpty && m.selectedZoomID == nil)
        m.undo()
        #expect(m.project.timeline.zoomBlocks.count == 1)
    }

    @Test func dragMoveIsOneUndoStep() {
        let m = makeModel()
        m.removeAllAutoZooms()   // the fixture click at 1 s made an auto zoom on open
        let id = m.addZoomBlock(at: 2)!
        let base = m.project.timeline.zoomBlocks
        m.beginGesture("Move Zoom")
        for d in [0.5, 1.0, 1.5] { m.moveZoomBlock(id, by: d, from: base) }
        m.endGesture()
        #expect(m.selectedZoom?.start == 3.5)
        m.undo()
        #expect(m.selectedZoom?.start == 2)
    }

    @Test func pickFocusSetsManualFocusAndClears() {
        let m = makeModel()
        m.removeAllAutoZooms()   // the fixture click at 1 s made an auto zoom on open
        let id = m.addZoomBlock(at: 1)!
        m.pickingFocusFor = id
        #expect(m.pickFocus(atOutput: .center, camera: .identity))
        #expect(m.selectedZoom?.focus == .manual(.center))
        #expect(m.pickingFocusFor == nil)
        #expect(!m.pickFocus(atOutput: .center, camera: .identity))
    }

    @Test func cameraBubbleMoveIsClamped() {
        let m = makeModel()
        m.moveCameraBubble(toOutput: NormalizedPoint(x: 3, y: 3))
        let p = m.project.style.camera.position
        #expect(p.x <= 1 && p.y <= 1 && p.x < 1)
    }
}

@Suite struct InspectorBindingTests {
    @Test func backgroundKindSwitching() {
        let solid = Background.solid(RGBAColor(r: 1, g: 0, b: 0))
        let grad = InspectorBindings.background(switchingTo: .gradient, from: solid)
        guard case .gradient(let stops, _) = grad else { Issue.record("not gradient"); return }
        #expect(stops.first?.color == RGBAColor(r: 1, g: 0, b: 0))
        #expect(InspectorBindings.background(switchingTo: .solid, from: solid) == solid)
        #expect(InspectorBindings.kind(of: InspectorBindings.background(switchingTo: .wallpaper, from: solid)) == .wallpaper)
    }

    @Test func shapeRoundTrip() {
        for k in InspectorBindings.ShapeKind.allCases {
            #expect(InspectorBindings.kind(of: InspectorBindings.shape(k)) == k)
        }
    }

    @Test func colorRoundTrip() {
        let c = RGBAColor(r: 0.2, g: 0.4, b: 0.6, a: 1)
        let back = InspectorBindings.rgba(InspectorBindings.color(c))
        #expect(abs(back.r - c.r) < 0.01 && abs(back.g - c.g) < 0.01 && abs(back.b - c.b) < 0.01)
    }

    @MainActor @Test func bindingsWriteStyleAndUndo() {
        let m = makeModel()
        let b = m.styleBinding(\.cursor.clickRipples)
        b.wrappedValue = false
        #expect(m.project.style.cursor.clickRipples == false)
        m.styleBinding(\.aspectRatio).wrappedValue = .r9x16
        #expect(m.project.style.aspectRatio == .r9x16)
        m.styleBinding(\.camera.mirror).wrappedValue = false
        #expect(m.project.style.camera.mirror == false)
        m.undo(); m.undo(); m.undo()
        #expect(m.project.style == Style())
    }
}

@MainActor
@Suite struct ExportModelTests {
    @Test func sizeFollowsAspectAndResolution() {
        let m = makeModel()
        let e = ExportModel(editor: m)
        #expect(e.outputSize == PixelSize(width: 1920, height: 1080))
        e.resolution = .p2160
        #expect(e.outputSize == PixelSize(width: 3840, height: 2160))
        m.setStyle(\.aspectRatio, .r9x16)
        #expect(e.outputSize == PixelSize(width: 2160, height: 3840))
        #expect(e.outputDescription.contains("9:16"))
    }

    @Test func defaultFileNameIsSanitized() {
        let u = ExportModel.defaultOutputURL(projectName: "a/b", in: URL(fileURLWithPath: "/tmp"))
        #expect(u.lastPathComponent == "a-b.mp4")
    }

    @Test func finishesAndFailsFromExporter() async throws {
        let ok = ExportModel(editor: makeModel())
        ok.start()
        for _ in 0..<1500 where ok.finishedURL == nil { try await Task.sleep(for: .milliseconds(20)) }
        #expect(ok.finishedURL == ok.outputURL)

        let bad = ExportModel(editor: makeModel(exporter: ScriptedExporter(fail: true)))
        bad.start()
        for _ in 0..<1500 where bad.failureMessage == nil { try await Task.sleep(for: .milliseconds(20)) }
        #expect(bad.failureMessage != nil)
    }

    @Test func cancelStops() {
        let e = ExportModel(editor: makeModel())
        e.start()
        e.cancel()
        #expect(e.progress == .cancelled && !e.isRunning)
    }
}
