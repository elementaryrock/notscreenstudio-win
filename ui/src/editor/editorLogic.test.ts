import assert from "node:assert/strict";
import test from "node:test";
import {
  addZoomBlock,
  beginDragEdit,
  clampToTrim,
  commitEdit,
  createUndoState,
  deleteZoomBlock,
  endDragEdit,
  formatCfDate,
  formatDurationShort,
  formatTimecode,
  moveZoomBlock,
  normalizeZoomBlocks,
  PlayheadEasing,
  redoEdit,
  removeAutoZooms,
  resizeZoomBlock,
  setTrimEnd,
  setTrimStart,
  setZoomEase,
  setZoomFocus,
  setZoomScale,
  undoEdit,
  updateDragEdit,
  ZOOM_MAX_SCALE,
  ZOOM_MIN_LENGTH,
  ZOOM_MIN_SCALE,
} from "./editorLogic.ts";
import { hexToRgba, outputPixelSize, rgbaToHex } from "../types.ts";

test("addZoomBlock respects 0.3s floor and never overlaps", () => {
  const first = addZoomBlock(1.0, [], 10.0, { length: 2.0, id: "A" });
  assert.ok(first);
  assert.equal(first.blocks.length, 1);
  assert.equal(first.blocks[0]!.start, 1.0);
  assert.equal(first.blocks[0]!.end, 3.0);

  // Inside existing block -> null.
  assert.equal(addZoomBlock(1.5, first.blocks, 10.0), null);

  // Tight gap after 3.0 before a second block at 3.2 -> < 0.3 s -> rejected.
  const second = addZoomBlock(3.2, first.blocks, 10.0, { length: 1.0, id: "B" });
  assert.ok(second);
  assert.equal(addZoomBlock(3.05, second.blocks, 10.0), null);

  // Gap of 0.5 s at start fits a shrunk block.
  const third = addZoomBlock(0.2, second.blocks, 10.0, { length: 2.0, id: "C" });
  assert.ok(third);
  const c = third.blocks.find((b) => b.id === "C")!;
  assert.equal(c.start, 0.2);
  assert.equal(c.end, 1.0);
});

test("moving and resizing stop at neighbors and preserve minLength", () => {
  const a = addZoomBlock(1.0, [], 10.0, { length: 2.0, id: "A" })!;
  const b = addZoomBlock(4.0, a.blocks, 10.0, { length: 2.0, id: "B" })!;

  // Move A right past B -> clamped flush against B.start (4.0) -> [2.0, 4.0].
  const moved = moveZoomBlock("A", 5.0, b.blocks, 10.0);
  const aMoved = moved.find((x) => x.id === "A")!;
  assert.equal(aMoved.start, 2.0);
  assert.equal(aMoved.end, 4.0);

  // Resize A end too far left -> clamped at start + 0.3.
  const resized = resizeZoomBlock("A", "end", 0.0, moved, 10.0);
  const aResized = resized.find((x) => x.id === "A")!;
  assert.ok(Math.abs(aResized.end - (aResized.start + ZOOM_MIN_LENGTH)) < 1e-9);
});

test("editing an auto block converts it to manual and clamps scale/focus/ease", () => {
  const autoBlock = {
    id: "AUTO1",
    start: 1.0,
    end: 3.0,
    scale: 2.0,
    focus: { auto: {} as Record<string, never> },
    easeIn: 0.35,
    easeOut: 0.35,
    isAuto: true,
  };
  const scaled = setZoomScale("AUTO1", 99.0, [autoBlock]);
  assert.equal(scaled[0]!.scale, ZOOM_MAX_SCALE);
  assert.equal(scaled[0]!.isAuto, false);

  const lowScaled = setZoomScale("AUTO1", 0.2, scaled);
  assert.equal(lowScaled[0]!.scale, ZOOM_MIN_SCALE);

  const focused = setZoomFocus("AUTO1", { manual: { x: 1.5, y: -0.2 } }, lowScaled);
  assert.deepEqual(focused[0]!.focus, { manual: { x: 1, y: 0 } });

  const eased = setZoomEase("AUTO1", 5.0, 0.01, focused);
  assert.equal(eased[0]!.easeIn, 2.0);
  assert.equal(eased[0]!.easeOut, 0.1);

  assert.equal(removeAutoZooms([autoBlock]).length, 0);
  assert.equal(removeAutoZooms(eased).length, 1);
  assert.equal(deleteZoomBlock("AUTO1", eased).length, 0);
});

test("normalizeZoomBlocks clips to new duration and drops tiny leftovers", () => {
  const a = addZoomBlock(0.5, [], 10.0, { length: 2.0, id: "A" })!;
  const b = addZoomBlock(4.9, a.blocks, 10.0, { length: 2.0, id: "B" })!;
  const norm = normalizeZoomBlocks(b.blocks, 5.0);
  // B has only 0.1 s left (< 0.3 s) so it is dropped; A stays.
  assert.deepEqual(
    norm.map((x) => x.id),
    ["A"],
  );
});

test("trim handles enforce 0.5s minimum and full-end returns null", () => {
  let trim = { start: 0, end: null as number | null };
  trim = setTrimStart(9.8, trim, 10.0);
  assert.equal(trim.start, 9.5);
  trim = setTrimStart(2.0, trim, 10.0);
  trim = setTrimEnd(2.1, trim, 10.0);
  assert.equal(trim.end, 2.5);
  assert.equal(clampToTrim(5.0, trim, 10.0), 2.5);
  trim = setTrimEnd(10.0, trim, 10.0);
  assert.equal(trim.end, null);
});

test("PlayheadEasing glides smoothly on jump and snaps immediately on drag", () => {
  const p = new PlayheadEasing(0, 0.2);
  p.glide(1.0);
  assert.equal(p.isAnimating, true);
  const mid = p.advance(0.1);
  assert.ok(mid > 0.5 && mid < 1.0, `cubic ease-out should be past midpoint at t=0.5: ${mid}`);
  p.advance(0.2);
  assert.equal(p.value, 1.0);
  assert.equal(p.isAnimating, false);

  p.snap(4.2);
  assert.equal(p.value, 4.2);
  assert.equal(p.isAnimating, false);
});

test("UndoHistory coalesces slider drags into one undo step", () => {
  let h = createUndoState({ padding: 0.08 });
  h = beginDragEdit(h);
  h = updateDragEdit(h, { padding: 0.12 });
  h = updateDragEdit(h, { padding: 0.16 });
  h = updateDragEdit(h, { padding: 0.2 });
  h = endDragEdit(h);
  assert.equal(h.past.length, 1);
  assert.equal(h.present.padding, 0.2);

  h = undoEdit(h);
  assert.equal(h.present.padding, 0.08);
  h = redoEdit(h);
  assert.equal(h.present.padding, 0.2);

  h = commitEdit(h, { padding: 0.25 });
  assert.equal(h.past.length, 2);
  assert.equal(h.future.length, 0);
  assert.equal(formatTimecode(65.4), "01:05.4");
  assert.equal(formatDurationShort(125), "2:05");
  assert.ok(formatCfDate(700_000_000).length > 0);
});

test("outputPixelSize and RGBA color helpers match upstream export presets", () => {
  assert.deepEqual(outputPixelSize("p1080", "r16x9"), { width: 1920, height: 1080 });
  assert.deepEqual(outputPixelSize("p1080", "r1x1"), { width: 1080, height: 1080 });
  assert.deepEqual(outputPixelSize("p1080", "r9x16"), { width: 1080, height: 1920 });
  assert.deepEqual(outputPixelSize("p2160", "r16x9"), { width: 3840, height: 2160 });
  assert.deepEqual(outputPixelSize("p2160", "r1x1"), { width: 2160, height: 2160 });
  assert.deepEqual(outputPixelSize("p2160", "r9x16"), { width: 2160, height: 3840 });

  const rgba = hexToRgba("#0a84ff", 0.5);
  assert.equal(rgbaToHex(rgba), "#0a84ff");
  assert.equal(rgba.a, 0.5);
});
