import { useCallback, useEffect, useRef, useState } from "react";
import { getPreviewLayout, renderPreviewFrame } from "../ipc";
import { formatTimecode } from "../editor/editorLogic";
import type { NormalizedPoint, PreviewLayoutDto, Project, ZoomBlock } from "../types";

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
  const [layout, setLayout] = useState<PreviewLayoutDto | null>(null);

  const mountedRef = useRef(true);
  const layoutRef = useRef<PreviewLayoutDto | null>(null);
  const inFlightRef = useRef(false);
  const pendingRef = useRef<{ time: number; rev: number; maxW: number; maxH: number } | null>(null);
  const lastStatsAtRef = useRef(0);
  const draggingCamRef = useRef(false);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

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
    if (inFlightRef.current || !mountedRef.current) return;
    const req = pendingRef.current;
    if (!req) return;
    pendingRef.current = null;
    inFlightRef.current = true;

    const t0 = performance.now();
    try {
      const pkt = await renderPreviewFrame(req.time, req.maxW, req.maxH);
      if (!mountedRef.current) return;
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
      try {
        const nextLayout = await getPreviewLayout(req.time, req.maxW, req.maxH);
        if (mountedRef.current) {
          layoutRef.current = nextLayout;
          setLayout(nextLayout);
        }
      } catch {
        // Non-fatal; fallback uses raw canvas coordinates.
      }
      if (!mountedRef.current) return;
      const now = performance.now();
      if (now - lastStatsAtRef.current > 220) {
        lastStatsAtRef.current = now;
        setFrameMs(Math.round(elapsed));
      }
      setRenderError(null);
    } catch (e) {
      if (mountedRef.current) {
        setRenderError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      inFlightRef.current = false;
      if (mountedRef.current && pendingRef.current) {
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
    selectedZoom && "manual" in selectedZoom.focus ? selectedZoom.focus.manual : null;

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
    const canvasPt = clientToNormalized(e.clientX, e.clientY);
    if (!canvasPt) return;
    const l = layoutRef.current;
    if (
      l &&
      l.width > 0 &&
      l.height > 0 &&
      l.contentWidth > 0 &&
      l.contentHeight > 0 &&
      l.cameraScale > 0
    ) {
      const outX = canvasPt.x * l.width;
      const outY = canvasPt.y * l.height;
      const ux = (outX - l.contentX) / l.contentWidth;
      const uy = (outY - l.contentY) / l.contentHeight;
      const srcX = Math.min(1, Math.max(0, l.cameraCenterX + (ux - 0.5) / l.cameraScale));
      const srcY = Math.min(1, Math.max(0, l.cameraCenterY + (uy - 0.5) / l.cameraScale));
      onPickManualFocus({ x: srcX, y: srcY });
      return;
    }
    onPickManualFocus(canvasPt);
  };

  const reticleStyle = (() => {
    if (!manualFocus) return null;
    if (
      !layout ||
      layout.width <= 0 ||
      layout.height <= 0 ||
      layout.contentWidth <= 0 ||
      layout.contentHeight <= 0
    ) {
      return {
        left: `${manualFocus.x * 100}%`,
        top: `${manualFocus.y * 100}%`,
      };
    }
    const ux = (manualFocus.x - layout.cameraCenterX) * layout.cameraScale + 0.5;
    const uy = (manualFocus.y - layout.cameraCenterY) * layout.cameraScale + 0.5;
    const outX = layout.contentX + ux * layout.contentWidth;
    const outY = layout.contentY + uy * layout.contentHeight;
    return {
      left: `${Math.min(100, Math.max(0, (outX / layout.width) * 100))}%`,
      top: `${Math.min(100, Math.max(0, (outY / layout.height) * 100))}%`,
    };
  })();

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

          {reticleStyle && (
            <div
              className="focus-reticle"
              style={reticleStyle}
              title="Manual zoom focus"
            />
          )}

          {project.style.camera.visible && project.assets.cameraFile !== null && (
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
