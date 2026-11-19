import { useEffect, useMemo, useRef, useState } from "react";
import { getTimelineThumbnails } from "../ipc";
import {
  addZoomBlock,
  moveZoomBlock,
  resizeZoomBlock,
  resolvedTrimEnd,
  setTrimEnd,
  setTrimStart,
} from "../editor/editorLogic";
import { strings } from "../strings";
import type { CursorTrack, Project, Timeline, TimelineThumbTile, ZoomBlock } from "../types";

interface TimelinePanelProps {
  project: Project;
  track: CursorTrack;
  duration: number;
  playheadTime: number;
  selectedZoomId: string | null;
  onJumpTime: (t: number) => void;
  onScrubTime: (t: number) => void;
  onSelectZoom: (id: string | null) => void;
  onBeginDrag: () => void;
  onUpdateTimeline: (next: Timeline) => void;
  onEndDrag: () => void;
}

type DragAction =
  | { kind: "scrub" }
  | { kind: "trim-start" }
  | { kind: "trim-end" }
  | { kind: "zoom-move"; id: string; startX: number; initialBlocks: ZoomBlock[] }
  | { kind: "zoom-start"; id: string; initialBlocks: ZoomBlock[] }
  | { kind: "zoom-end"; id: string; initialBlocks: ZoomBlock[] };

export function TimelinePanel({
  project,
  track,
  duration,
  playheadTime,
  selectedZoomId,
  onJumpTime,
  onScrubTime,
  onSelectZoom,
  onBeginDrag,
  onUpdateTimeline,
  onEndDrag,
}: TimelinePanelProps): React.JSX.Element {
  const lanesRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<DragAction | null>(null);
  const movedRef = useRef(false);
  const [thumbUrls, setThumbUrls] = useState<string[]>([]);

  const safeDuration = Math.max(duration, 0.1);
  const trimStart = project.timeline.trim.start;
  const trimEnd = resolvedTrimEnd(project.timeline.trim, safeDuration);

  // Fetch filmstrip thumbnails once per project.
  useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const tiles = await getTimelineThumbnails(14, 96, 54);
        if (!active) return;
        const urls = tiles.map(tileToDataUrl).filter((u): u is string => u !== null);
        setThumbUrls(urls);
      } catch {
        if (active) setThumbUrls([]);
      }
    })();
    return () => {
      active = false;
    };
  }, [project.id, project.assets.screenFile]);

  const ticks = useMemo(() => {
    const step =
      safeDuration <= 10 ? 1 : safeDuration <= 30 ? 2 : safeDuration <= 90 ? 5 : 10;
    const list: number[] = [];
    for (let t = 0; t <= safeDuration + 1e-6; t += step) {
      list.push(t);
    }
    return list;
  }, [safeDuration]);

  const clickDots = useMemo(
    () => track.clicks.filter((c) => c.kind === "down" && c.t >= 0 && c.t <= safeDuration),
    [track.clicks, safeDuration],
  );

  const clientXToTime = (clientX: number): number => {
    const el = lanesRef.current;
    if (!el) return 0;
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0) return 0;
    const frac = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
    return frac * safeDuration;
  };

  const pct = (t: number): string => `${(Math.min(safeDuration, Math.max(0, t)) / safeDuration) * 100}%`;

  const startDrag = (e: React.PointerEvent, action: DragAction): void => {
    e.stopPropagation();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    dragRef.current = action;
    movedRef.current = false;
    if (action.kind !== "scrub") {
      onBeginDrag();
    }
  };

  const handlePointerMove = (e: React.PointerEvent): void => {
    const act = dragRef.current;
    if (!act) return;
    movedRef.current = true;
    const t = clientXToTime(e.clientX);

    if (act.kind === "scrub") {
      onScrubTime(t);
      return;
    }
    if (act.kind === "trim-start") {
      onUpdateTimeline({
        ...project.timeline,
        trim: setTrimStart(t, project.timeline.trim, safeDuration),
      });
      return;
    }
    if (act.kind === "trim-end") {
      onUpdateTimeline({
        ...project.timeline,
        trim: setTrimEnd(t, project.timeline.trim, safeDuration),
      });
      return;
    }
    if (act.kind === "zoom-move") {
      const el = lanesRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const deltaT = ((e.clientX - act.startX) / Math.max(rect.width, 1)) * safeDuration;
      onUpdateTimeline({
        ...project.timeline,
        zoomBlocks: moveZoomBlock(act.id, deltaT, act.initialBlocks, safeDuration),
      });
      return;
    }
    if (act.kind === "zoom-start") {
      onUpdateTimeline({
        ...project.timeline,
        zoomBlocks: resizeZoomBlock(act.id, "start", t, act.initialBlocks, safeDuration),
      });
      return;
    }
    if (act.kind === "zoom-end") {
      onUpdateTimeline({
        ...project.timeline,
        zoomBlocks: resizeZoomBlock(act.id, "end", t, act.initialBlocks, safeDuration),
      });
    }
  };

  const handlePointerUp = (e: React.PointerEvent): void => {
    const act = dragRef.current;
    if (!act) return;
    dragRef.current = null;
    try {
      (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {
      // Capture may already have been released.
    }
    if (act.kind === "scrub" && !movedRef.current) {
      onJumpTime(clientXToTime(e.clientX));
    } else if (act.kind !== "scrub") {
      onEndDrag();
    }
  };

  const hasCamera = project.assets.cameraFile !== null;
  const hasAudio = project.assets.micFile !== null;

  return (
    <section
      className="timeline-panel"
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
      onPointerCancel={handlePointerUp}
    >
      <div className="timeline-grid">
        {/* Track labels column */}
        <div className="timeline-labels">
          <div className="timeline-ruler-label">Time</div>
          <div className="timeline-track-label">{strings.trackVideo}</div>
          <div className="timeline-track-label">{strings.trackZoom}</div>
          <div className="timeline-track-label">{strings.trackCamera}</div>
          <div className="timeline-track-label">{strings.trackAudio}</div>
        </div>

        {/* Interactive lanes */}
        <div
          className="timeline-lanes"
          ref={lanesRef}
          onClick={() => onSelectZoom(null)}
        >
          {/* Time Ruler */}
          <div
            className="timeline-ruler"
            onPointerDown={(e) => startDrag(e, { kind: "scrub" })}
          >
            {ticks.map((t) => (
              <div key={t} className="ruler-tick" style={{ left: pct(t) }}>
                <span>{formatRulerTime(t)}</span>
              </div>
            ))}
          </div>

          {/* Video Track with filmstrip + trim handles */}
          <div className="timeline-lane lane-video">
            <div className="filmstrip">
              {thumbUrls.length > 0 ? (
                thumbUrls.map((url, idx) => (
                  <img key={idx} src={url} alt="" draggable={false} />
                ))
              ) : (
                <div className="filmstrip-fallback" />
              )}
            </div>

            {/* Trimmed-out dim regions */}
            {trimStart > 0 && (
              <div className="trim-dim" style={{ left: 0, width: pct(trimStart) }} />
            )}
            {trimEnd < safeDuration && (
              <div
                className="trim-dim"
                style={{ left: pct(trimEnd), right: 0 }}
              />
            )}

            {/* Active trim outline + draggable handles */}
            <div
              className="trim-window"
              style={{
                left: pct(trimStart),
                width: `${((trimEnd - trimStart) / safeDuration) * 100}%`,
              }}
            >
              <div
                className="trim-handle left"
                title="Drag to adjust trim start"
                onPointerDown={(e) => startDrag(e, { kind: "trim-start" })}
              />
              <div
                className="trim-handle right"
                title="Drag to adjust trim end"
                onPointerDown={(e) => startDrag(e, { kind: "trim-end" })}
              />
            </div>
          </div>

          {/* Zoom Track + Click dots (double-click empty track to add a 2x zoom block) */}
          <div
            className="timeline-lane lane-zoom"
            onDoubleClick={(e) => {
              const t = clientXToTime(e.clientX);
              const res = addZoomBlock(t, project.timeline.zoomBlocks, safeDuration);
              if (!res) return;
              onBeginDrag();
              onUpdateTimeline({ ...project.timeline, zoomBlocks: res.blocks });
              onEndDrag();
              onSelectZoom(res.id);
            }}
          >
            {project.timeline.zoomBlocks.map((b) => {
              const selected = b.id === selectedZoomId;
              const leftPct = (b.start / safeDuration) * 100;
              const widthPct = Math.max(1.2, ((b.end - b.start) / safeDuration) * 100);
              return (
                <div
                  key={b.id}
                  className={`zoom-block ${b.isAuto ? "auto" : "manual"} ${selected ? "selected" : ""}`}
                  style={{ left: `${leftPct}%`, width: `${widthPct}%` }}
                  onClick={(e) => {
                    e.stopPropagation();
                    onSelectZoom(b.id);
                  }}
                  onPointerDown={(e) => {
                    onSelectZoom(b.id);
                    startDrag(e, {
                      kind: "zoom-move",
                      id: b.id,
                      startX: e.clientX,
                      initialBlocks: project.timeline.zoomBlocks,
                    });
                  }}
                >
                  <div
                    className="zoom-edge left"
                    onPointerDown={(e) => {
                      onSelectZoom(b.id);
                      startDrag(e, {
                        kind: "zoom-start",
                        id: b.id,
                        initialBlocks: project.timeline.zoomBlocks,
                      });
                    }}
                  />
                  <span className="zoom-block-label">
                    {b.isAuto ? "✦ " : ""}
                    {b.scale.toFixed(1)}×
                  </span>
                  <div
                    className="zoom-edge right"
                    onPointerDown={(e) => {
                      onSelectZoom(b.id);
                      startDrag(e, {
                        kind: "zoom-end",
                        id: b.id,
                        initialBlocks: project.timeline.zoomBlocks,
                      });
                    }}
                  />
                </div>
              );
            })}

            {/* Recorded mouse click dots */}
            {clickDots.map((c, idx) => (
              <span
                key={idx}
                className="click-dot"
                style={{ left: pct(c.t) }}
                title={`Click at ${c.t.toFixed(2)}s`}
              />
            ))}
          </div>

          {/* Camera Track */}
          <div className="timeline-lane lane-camera">
            {hasCamera ? (
              <div className="aux-clip camera-clip">Camera recording</div>
            ) : (
              <span className="lane-empty-hint">No camera track</span>
            )}
          </div>

          {/* Audio Track */}
          <div className="timeline-lane lane-audio">
            {hasAudio ? (
              <div className="aux-clip audio-clip">Microphone audio (48 kHz AAC)</div>
            ) : (
              <span className="lane-empty-hint">No audio track</span>
            )}
          </div>

          {/* Playhead line spanning ruler + all lanes */}
          <div className="playhead" style={{ left: pct(playheadTime) }}>
            <div
              className="playhead-head"
              onPointerDown={(e) => startDrag(e, { kind: "scrub" })}
            />
          </div>
        </div>
      </div>
    </section>
  );
}

function tileToDataUrl(tile: TimelineThumbTile): string | null {
  if (tile.width === 0 || tile.height === 0 || tile.rgba.length < tile.width * tile.height * 4) {
    return null;
  }
  const canvas = document.createElement("canvas");
  canvas.width = tile.width;
  canvas.height = tile.height;
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  const img = new ImageData(new Uint8ClampedArray(tile.rgba), tile.width, tile.height);
  ctx.putImageData(img, 0, 0);
  return canvas.toDataURL("image/png");
}

function formatRulerTime(sec: number): string {
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60);
  return m > 0 ? `${m}:${String(s).padStart(2, "0")}` : `${s}s`;
}
