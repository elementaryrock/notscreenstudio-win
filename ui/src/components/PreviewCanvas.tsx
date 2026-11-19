import { useCallback, useEffect, useRef, useState } from "react";
import { renderPreviewFrame } from "../ipc";
import { formatTimecode } from "../editor/editorLogic";
import type { NormalizedPoint, Project, ZoomBlock } from "../types";

interface PreviewCanvasProps {
  project: Project;
  projectRevision: number;
  currentTime: number;
  duration: number;
  playing: boolean;
  rendererName: string;
  selectedZoom: ZoomBlock | null;
  onTogglePlay: () => void;
  onStepFrame: (deltaFrames: number) => void;
  onPickManualFocus: (point: NormalizedPoint) => void;
  onBeginCameraDrag: () => void;
  onMoveCameraPosition: (point: NormalizedPoint) => void;
  onEndCameraDrag: () => void;
}

export function PreviewCanvas({
  project,
  projectRevision,
  currentTime,
  duration,
  playing,
  rendererName,
  selectedZoom,
  onTogglePlay,
  onStepFrame,
  onPickManualFocus,
  onBeginCameraDrag,
  onMoveCameraPosition,
  onEndCameraDrag,
}: PreviewCanvasProps): React.JSX.Element {
  const stageRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [stageSize, setStageSize] = useState<{ w: number; h: number }>({ w: 960, h: 540 });
  const [frameMs, setFrameMs] = useState<number | null>(null);
  const [renderError, setRenderError] = useState<string | null>(null);

  const inFlightRef = useRef(false);
  const pendingRef = useRef<{ time: number; rev: number; maxW: number; maxH: number } | null>(null);
  const lastStatsAtRef = useRef(0);
  const draggingCamRef = useRef(false);

  // Measure available preview stage so GPU preview scales crisply without over-rendering.
  useEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const update = () => {
      const rect = el.getBoundingClientRect();
      const maxW = Math.max(320, Math.min(1280, Math.floor(rect.width)));
      const maxH = Math.max(240, Math.min(720, Math.floor(rect.height)));
      setStageSize((prev) => (prev.w === maxW && prev.h === maxH ? prev : { w: maxW, h: maxH }));
    };
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const pumpRender = useCallback(async () => {
    if (inFlightRef.current) return;
    const req = pendingRef.current;
    if (!req) return;
    pendingRef.current = null;
    inFlightRef.current = true;

    const t0 = performance.now();
    try {
      const pkt = await renderPreviewFrame(req.time, req.maxW, req.maxH);
      const elapsed = performance.now() - t0;
      const canvas = canvasRef.current;
      if (pkt && canvas) {
        if (canvas.width !== pkt.width || canvas.height !== pkt.height) {
          canvas.width = pkt.width;
          canvas.height = pkt.height;
        }
        const ctx = canvas.getContext("2d");
        if (ctx) {
          const img = new ImageData(pkt.pixels, pkt.width, pkt.height);
          ctx.putImageData(img, 0, 0);
        }
      }
      const now = performance.now();
      if (now - lastStatsAtRef.current > 220) {
        lastStatsAtRef.current = now;
        setFrameMs(Math.round(elapsed));
      }
      setRenderError(null);
    } catch (e) {
      setRenderError(e instanceof Error ? e.message : String(e));
    } finally {
      inFlightRef.current = false;
      if (pendingRef.current) {
        void pumpRender();
      }
    }
  }, []);

  useEffect(() => {
    pendingRef.current = {
      time: currentTime,
      rev: projectRevision,
      maxW: stageSize.w,
      maxH: stageSize.h,
    };
    void pumpRender();
  }, [currentTime, projectRevision, stageSize.w, stageSize.h, pumpRender]);

  const manualFocus =
    selectedZoom && "manual" in selectedZoom.focus ? selectedZoom.focus.manual._0 : null;

  const clientToNormalized = (clientX: number, clientY: number): NormalizedPoint | null => {
    const canvas = canvasRef.current;
    if (!canvas) return null;
    const rect = canvas.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return null;
    const x = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
    const y = Math.min(1, Math.max(0, (clientY - rect.top) / rect.height));
    return { x, y };
  };

  const handleCanvasClick = (e: React.MouseEvent<HTMLCanvasElement>): void => {
    if (!manualFocus) return;
    const pt = clientToNormalized(e.clientX, e.clientY);
    if (pt) onPickManualFocus(pt);
  };

  const handleCameraPointerDown = (e: React.PointerEvent<HTMLDivElement>): void => {
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    draggingCamRef.current = true;
    onBeginCameraDrag();
  };

  const handleCameraPointerMove = (e: React.PointerEvent<HTMLDivElement>): void => {
    if (!draggingCamRef.current) return;
    const pt = clientToNormalized(e.clientX, e.clientY);
    if (pt) {
      onMoveCameraPosition({
        x: Math.min(0.95, Math.max(0.05, pt.x)),
        y: Math.min(0.95, Math.max(0.05, pt.y)),
      });
    }
  };

  const handleCameraPointerUp = (e: React.PointerEvent<HTMLDivElement>): void => {
    if (!draggingCamRef.current) return;
    draggingCamRef.current = false;
    try {
      e.currentTarget.releasePointerCapture(e.pointerId);
    } catch {
      // Ignore if capture already released.
    }
    onEndCameraDrag();
  };

  return (
    <section className="preview-pane">
      <div className="preview-stage" ref={stageRef}>
        <div className="preview-canvas-wrap">
          <canvas
            ref={canvasRef}
            className={`preview-canvas ${manualFocus ? "pick-focus" : ""}`}
            onClick={handleCanvasClick}
          />

          {manualFocus && (
            <div
              className="focus-reticle"
              style={{
                left: `${manualFocus.x * 100}%`,
                top: `${manualFocus.y * 100}%`,
              }}
              title="Manual zoom focus"
            />
          )}

          {project.style.camera.visible && (
            <div
              className="camera-drag-handle"
              style={{
                left: `${project.style.camera.position.x * 100}%`,
                top: `${project.style.camera.position.y * 100}%`,
              }}
              title="Drag to position camera bubble"
              onPointerDown={handleCameraPointerDown}
              onPointerMove={handleCameraPointerMove}
              onPointerUp={handleCameraPointerUp}
              onPointerCancel={handleCameraPointerUp}
            >
              CAM
            </div>
          )}

          {renderError && <div className="preview-error-pill">{renderError}</div>}
        </div>
      </div>

      <div className="transport-bar">
        <div className="transport-controls">
          <button
            type="button"
            className="icon-btn"
            title="Step back 1 frame (Left Arrow)"
            onClick={() => onStepFrame(-1)}
          >
            ◀|
          </button>
          <button
            type="button"
            className="btn-play"
            title="Play / Pause (Space)"
            onClick={onTogglePlay}
          >
            {playing ? "❚❚" : "▶"}
          </button>
          <button
            type="button"
            className="icon-btn"
            title="Step forward 1 frame (Right Arrow)"
            onClick={() => onStepFrame(1)}
          >
            |▶
          </button>
        </div>

        <div className="transport-timecode mono">
          <span>{formatTimecode(currentTime)}</span>
          <span className="muted-sep">/</span>
          <span className="muted">{formatTimecode(duration)}</span>
        </div>

        <div className="transport-stats">
          <span className="renderer-pill">
            {rendererName}
            {frameMs !== null ? ` • ${frameMs} ms` : ""}
          </span>
        </div>
      </div>
    </section>
  );
}
