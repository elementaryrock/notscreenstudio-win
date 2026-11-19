/**
 * Pure editor logic ported verbatim from upstream:
 * - `ZoomBlockEditing.swift` (0.3 s floor, non-overlap, 1.1..4.0 scale, manual takeover on edit)
 * - `TrimEditing` (0.5 s minimum trim length)
 * - `PlayheadEasing.swift` (0.2 s cubic ease-out glide on jump, instant snap on drag/playback)
 * - Coalescing Undo/Redo stack (slider drag = 1 undo step)
 */
import type { TrimRange, ZoomBlock, ZoomFocus } from "../types.ts";

export const ZOOM_MIN_LENGTH = 0.3;
export const ZOOM_DEFAULT_LENGTH = 2.0;
export const ZOOM_MIN_SCALE = 1.1;
export const ZOOM_MAX_SCALE = 4.0;
export const ZOOM_MIN_EASE = 0.1;
export const ZOOM_MAX_EASE = 2.0;
export const ZOOM_DEFAULT_EASE = 0.75;
export const TRIM_MIN_LENGTH = 0.5;

function sortedBlocks(blocks: readonly ZoomBlock[]): ZoomBlock[] {
  return [...blocks].sort((a, b) => a.start - b.start);
}

function makeUuid(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID().toUpperCase();
  }
  const hex = () =>
    Math.floor(Math.random() * 0x10000)
      .toString(16)
      .padStart(4, "0")
      .toUpperCase();
  return `${hex()}${hex()}-${hex()}-4${hex().slice(1)}-8${hex().slice(1)}-${hex()}${hex()}${hex()}`;
}

export function freeGapAround(
  t: number,
  blocks: readonly ZoomBlock[],
  duration: number,
  ignoringId?: string,
): { lo: number; hi: number } | null {
  let lo = 0;
  let hi = duration;
  for (const b of blocks) {
    if (b.id === ignoringId) continue;
    if (t >= b.start && t < b.end) return null;
    if (b.end <= t) lo = Math.max(lo, b.end);
    if (b.start >= t) hi = Math.min(hi, b.start);
  }
  return { lo, hi };
}

export function addZoomBlock(
  time: number,
  blocks: readonly ZoomBlock[],
  duration: number,
  options?: { length?: number; scale?: number; focus?: ZoomFocus; id?: string },
): { blocks: ZoomBlock[]; id: string } | null {
  const length = options?.length ?? ZOOM_DEFAULT_LENGTH;
  const scale = clamp(options?.scale ?? 2.0, ZOOM_MIN_SCALE, ZOOM_MAX_SCALE);
  const focus: ZoomFocus = options?.focus ?? { auto: {} };
  const id = options?.id ?? makeUuid();

  const t = clamp(time, 0, Math.max(duration - ZOOM_MIN_LENGTH, 0));
  const gap = freeGapAround(t, blocks, duration);
  if (!gap) return null;

  const end = Math.min(t + length, gap.hi);
  let start = t;
  if (end - start < ZOOM_MIN_LENGTH - 1e-9) {
    start = Math.max(gap.lo, end - length);
    if (end - start < ZOOM_MIN_LENGTH - 1e-9) return null;
  }

  const created: ZoomBlock = {
    id,
    start,
    end,
    scale,
    focus,
    easeIn: ZOOM_DEFAULT_EASE,
    easeOut: ZOOM_DEFAULT_EASE,
    isAuto: false,
  };
  return { blocks: sortedBlocks([...blocks, created]), id };
}

function replaceBlock(
  id: string,
  blocks: readonly ZoomBlock[],
  mutate: (b: ZoomBlock) => ZoomBlock,
): ZoomBlock[] {
  return blocks.map((b) => {
    if (b.id !== id) return b;
    const next = mutate({ ...b });
    const changed =
      next.start !== b.start ||
      next.end !== b.end ||
      next.scale !== b.scale ||
      next.easeIn !== b.easeIn ||
      next.easeOut !== b.easeOut ||
      JSON.stringify(next.focus) !== JSON.stringify(b.focus);
    return changed ? { ...next, isAuto: false } : next;
  });
}

export function moveZoomBlock(
  id: string,
  delta: number,
  blocks: readonly ZoomBlock[],
  duration: number,
): ZoomBlock[] {
  const target = blocks.find((b) => b.id === id);
  if (!target) return [...blocks];
  const len = target.end - target.start;
  let lo = 0;
  let hi = duration;
  for (const o of blocks) {
    if (o.id === id) continue;
    if (o.end <= target.start + 1e-9) lo = Math.max(lo, o.end);
    if (o.start >= target.end - 1e-9) hi = Math.min(hi, o.start);
  }
  const newStart = clamp(target.start + delta, lo, Math.max(lo, hi - len));
  return replaceBlock(id, blocks, (b) => ({ ...b, start: newStart, end: newStart + len }));
}

export function resizeZoomBlock(
  id: string,
  edge: "start" | "end",
  time: number,
  blocks: readonly ZoomBlock[],
  duration: number,
): ZoomBlock[] {
  const target = blocks.find((b) => b.id === id);
  if (!target) return [...blocks];
  let lo = 0;
  let hi = duration;
  for (const o of blocks) {
    if (o.id === id) continue;
    if (o.end <= target.start + 1e-9) lo = Math.max(lo, o.end);
    if (o.start >= target.end - 1e-9) hi = Math.min(hi, o.start);
  }
  if (edge === "start") {
    const v = clamp(time, lo, target.end - ZOOM_MIN_LENGTH);
    return replaceBlock(id, blocks, (b) => ({ ...b, start: v }));
  }
  const v = clamp(time, target.start + ZOOM_MIN_LENGTH, hi);
  return replaceBlock(id, blocks, (b) => ({ ...b, end: v }));
}

export function deleteZoomBlock(id: string, blocks: readonly ZoomBlock[]): ZoomBlock[] {
  return blocks.filter((b) => b.id !== id);
}

export function setZoomScale(id: string, scale: number, blocks: readonly ZoomBlock[]): ZoomBlock[] {
  const clamped = clamp(scale, ZOOM_MIN_SCALE, ZOOM_MAX_SCALE);
  return replaceBlock(id, blocks, (b) => ({ ...b, scale: clamped }));
}

export function setZoomFocus(
  id: string,
  focus: ZoomFocus,
  blocks: readonly ZoomBlock[],
): ZoomBlock[] {
  const clean: ZoomFocus =
    "manual" in focus
      ? {
          manual: {
            x: clamp(focus.manual.x, 0, 1),
            y: clamp(focus.manual.y, 0, 1),
          },
        }
      : { auto: {} };
  return replaceBlock(id, blocks, (b) => ({ ...b, focus: clean }));
}

export function setZoomEase(
  id: string,
  easeIn: number | null,
  easeOut: number | null,
  blocks: readonly ZoomBlock[],
): ZoomBlock[] {
  return replaceBlock(id, blocks, (b) => ({
    ...b,
    easeIn: easeIn !== null && Number.isFinite(easeIn) ? clamp(easeIn, ZOOM_MIN_EASE, ZOOM_MAX_EASE) : b.easeIn,
    easeOut: easeOut !== null && Number.isFinite(easeOut) ? clamp(easeOut, ZOOM_MIN_EASE, ZOOM_MAX_EASE) : b.easeOut,
  }));
}

export function removeAutoZooms(blocks: readonly ZoomBlock[]): ZoomBlock[] {
  return blocks.filter((b) => !b.isAuto);
}

export function normalizeZoomBlocks(blocks: readonly ZoomBlock[], duration: number): ZoomBlock[] {
  const out: ZoomBlock[] = [];
  let cursor = 0;
  for (const b of sortedBlocks(blocks)) {
    const start = Math.max(b.start, cursor);
    const end = Math.min(b.end, duration);
    if (end - start >= ZOOM_MIN_LENGTH - 1e-9) {
      out.push({ ...b, start, end });
      cursor = end;
    }
  }
  return out;
}

// Trim Editing
export function resolvedTrimEnd(trim: TrimRange, duration: number): number {
  if (trim.end === null || !Number.isFinite(trim.end)) return duration;
  return clamp(trim.end, trim.start, duration);
}

export function setTrimStart(t: number, trim: TrimRange, duration: number): TrimRange {
  const end = resolvedTrimEnd(trim, duration);
  const minLen = Math.min(TRIM_MIN_LENGTH, duration);
  return {
    ...trim,
    start: clamp(t, 0, Math.max(end - minLen, 0)),
  };
}

export function setTrimEnd(t: number, trim: TrimRange, duration: number): TrimRange {
  const minLen = Math.min(TRIM_MIN_LENGTH, duration);
  const v = clamp(t, trim.start + minLen, duration);
  return {
    ...trim,
    end: v >= duration - 1e-9 ? null : v,
  };
}

export function clampToTrim(t: number, trim: TrimRange, duration: number): number {
  return clamp(t, trim.start, resolvedTrimEnd(trim, duration));
}

// Playhead Easing (port of PlayheadEasing.swift)
export class PlayheadEasing {
  static readonly DEFAULT_DURATION = 0.2;

  value: number;
  target: number;
  readonly duration: number;
  private start: number;
  private elapsed = 0;

  constructor(initialTime = 0, duration = PlayheadEasing.DEFAULT_DURATION) {
    this.value = initialTime;
    this.target = initialTime;
    this.start = initialTime;
    this.duration = Math.max(duration, 0.001);
  }

  get isAnimating(): boolean {
    return this.value !== this.target;
  }

  static easeOut(p: number): number {
    const q = 1 - clamp(p, 0, 1);
    return 1 - q * q * q;
  }

  snap(time: number): void {
    this.value = time;
    this.target = time;
    this.start = time;
    this.elapsed = 0;
  }

  glide(time: number): void {
    if (time === this.target) return;
    this.start = this.value;
    this.target = time;
    this.elapsed = 0;
  }

  advance(dt: number): number {
    if (this.value === this.target) return this.value;
    this.elapsed += Math.max(dt, 0);
    if (this.elapsed >= this.duration) {
      this.value = this.target;
    } else {
      this.value = this.start + (this.target - this.start) * PlayheadEasing.easeOut(this.elapsed / this.duration);
    }
    return this.value;
  }
}

// Coalescing Undo/Redo stack (slider drag = 1 undo step)
export interface UndoState<T> {
  past: T[];
  present: T;
  future: T[];
  dragSnapshot: T | null;
}

export function createUndoState<T>(initial: T): UndoState<T> {
  return { past: [], present: initial, future: [], dragSnapshot: null };
}

export function commitEdit<T>(state: UndoState<T>, next: T, maxDepth = 100): UndoState<T> {
  const past = [...state.past, state.present];
  if (past.length > maxDepth) past.shift();
  return { past, present: next, future: [], dragSnapshot: null };
}

export function beginDragEdit<T>(state: UndoState<T>): UndoState<T> {
  if (state.dragSnapshot !== null) return state;
  return { ...state, dragSnapshot: state.present };
}

export function updateDragEdit<T>(state: UndoState<T>, next: T): UndoState<T> {
  const snap = state.dragSnapshot ?? state.present;
  return { ...state, present: next, dragSnapshot: snap };
}

export function endDragEdit<T>(state: UndoState<T>, maxDepth = 100): UndoState<T> {
  if (state.dragSnapshot === null) return state;
  if (JSON.stringify(state.dragSnapshot) === JSON.stringify(state.present)) {
    return { ...state, dragSnapshot: null };
  }
  const past = [...state.past, state.dragSnapshot];
  if (past.length > maxDepth) past.shift();
  return { past, present: state.present, future: [], dragSnapshot: null };
}

export function undoEdit<T>(state: UndoState<T>): UndoState<T> {
  if (state.past.length === 0) return state;
  const prev = state.past[state.past.length - 1]!;
  return {
    past: state.past.slice(0, -1),
    present: prev,
    future: [state.present, ...state.future],
    dragSnapshot: null,
  };
}

export function redoEdit<T>(state: UndoState<T>): UndoState<T> {
  if (state.future.length === 0) return state;
  const next = state.future[0]!;
  return {
    past: [...state.past, state.present],
    present: next,
    future: state.future.slice(1),
    dragSnapshot: null,
  };
}

export function formatTimecode(seconds: number): string {
  const safe = Number.isFinite(seconds) ? Math.max(0, seconds) : 0;
  const mins = Math.floor(safe / 60);
  const secs = Math.floor(safe % 60);
  const tenths = Math.floor((safe - Math.floor(safe)) * 10);
  return `${String(mins).padStart(2, "0")}:${String(secs).padStart(2, "0")}.${tenths}`;
}

export function formatDurationShort(seconds: number): string {
  const safe = Number.isFinite(seconds) ? Math.max(0, Math.round(seconds)) : 0;
  const mins = Math.floor(safe / 60);
  const secs = safe % 60;
  return `${mins}:${String(secs).padStart(2, "0")}`;
}

/** Floored live recording timer matching Rust `format_elapsed` (`MM:SS` or `H:MM:SS`). */
export function formatElapsedTimer(seconds: number): string {
  const total = Number.isFinite(seconds) ? Math.max(0, Math.floor(seconds)) : 0;
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) {
    return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  }
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

/** Converts CoreFoundation reference-date seconds (2001-01-01Z) into a readable local date. */
export function formatCfDate(cfSeconds: number): string {
  const unixMs = (cfSeconds + 978307200) * 1000;
  const d = new Date(unixMs);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(Math.max(v, lo), hi);
}
