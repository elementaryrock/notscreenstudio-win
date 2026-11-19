import { useCallback, useEffect, useRef, useState } from "react";
import { openEditorProject, updateEditorProject } from "../ipc";
import {
  beginDragEdit,
  clampToTrim,
  commitEdit,
  createUndoState,
  deleteZoomBlock,
  endDragEdit,
  PlayheadEasing,
  redoEdit,
  resolvedTrimEnd,
  setZoomFocus,
  undoEdit,
  updateDragEdit,
  type UndoState,
} from "../editor/editorLogic";
import { strings } from "../strings";
import {
  aspectLabel,
  type AspectRatio,
  type CursorTrack,
  type NormalizedPoint,
  type Project,
  type ProjectReference,
  type ShellSettings,
  type Timeline,
} from "../types";
import { ExportModal } from "./ExportModal";
import { InspectorPanel } from "./InspectorPanel";
import { PreviewCanvas } from "./PreviewCanvas";
import { TimelinePanel } from "./TimelinePanel";

interface EditorViewProps {
  reference: ProjectReference;
  settings: ShellSettings;
  onSettingsChange: (s: ShellSettings) => void;
  onBack: () => void;
}

export function EditorView({
  reference,
  settings,
  onSettingsChange,
  onBack,
}: EditorViewProps): React.JSX.Element {
  const [undo, setUndo] = useState<UndoState<Project> | null>(null);
  const [track, setTrack] = useState<CursorTrack>({
    samples: [],
    clicks: [],
    sampleRate: 120,
    shapes: [],
  });
  const [duration, setDuration] = useState(0);
  const [rendererName, setRendererName] = useState("wgpu");
  const [error, setError] = useState<string | null>(null);
  const [revision, setRevision] = useState(0);
  const [selectedZoomId, setSelectedZoomId] = useState<string | null>(null);
  const [playing, setPlaying] = useState(false);
  const [playheadTime, setPlayheadTime] = useState(0);
  const [glideTick, setGlideTick] = useState(0);
  const [titleDraft, setTitleDraft] = useState("");
  const [showExport, setShowExport] = useState(false);

  const easingRef = useRef<PlayheadEasing>(new PlayheadEasing(0));
  const undoRef = useRef<UndoState<Project> | null>(null);
  const latestProjectRef = useRef<{ url: string; project: Project } | null>(null);
  const pushSeqRef = useRef(0);
  const appliedSeqRef = useRef(0);

  undoRef.current = undo;

  // Load bundle into PreviewEngine.
  useEffect(() => {
    let active = true;
    const targetUrl = reference.url;
    latestProjectRef.current = null;
    undoRef.current = null;
    pushSeqRef.current = 0;
    appliedSeqRef.current = 0;
    setUndo(null);
    setSelectedZoomId(null);
    setPlaying(false);
    setError(null);

    void (async () => {
      try {
        const dto = await openEditorProject(reference);
        if (!active) return;
        const initialUndo = createUndoState(dto.project);
        undoRef.current = initialUndo;
        setUndo(initialUndo);
        setTitleDraft(dto.project.name);
        latestProjectRef.current = { url: targetUrl, project: dto.project };
        setTrack(dto.track);
        setDuration(Math.max(dto.duration, 0.1));
        setRendererName(dto.rendererName);
        const startT = dto.project.timeline.trim.start;
        easingRef.current.snap(startT);
        setPlayheadTime(startT);
        setRevision((r) => r + 1);
        setError(null);
      } catch (e) {
        if (active) setError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      active = false;
      const latest = latestProjectRef.current;
      if (latest && latest.url === targetUrl) {
        void updateEditorProject(reference, latest.project, true).catch(() => { });
      }
    };
  }, [reference.url]);

  const project = undo?.present ?? null;

  useEffect(() => {
    if (project) {
      setTitleDraft(project.name);
    }
  }, [project?.name]);

  const pushToBackend = useCallback(
    (next: Project, immediate = false) => {
      latestProjectRef.current = { url: reference.url, project: next };
      const seq = ++pushSeqRef.current;
      const finish = () => {
        if (seq > appliedSeqRef.current) {
          appliedSeqRef.current = seq;
          setRevision((r) => r + 1);
        }
      };
      void updateEditorProject(reference, next, immediate).then(finish, finish);
    },
    [reference],
  );

  const commitProject = useCallback(
    (next: Project) => {
      setUndo((prev) => {
        if (!prev) return prev;
        const updated = commitEdit(prev, next);
        undoRef.current = updated;
        return updated;
      });
      pushToBackend(next, false);
    },
    [pushToBackend],
  );

  const beginDrag = useCallback(() => {
    setUndo((prev) => {
      if (!prev) return prev;
      const updated = beginDragEdit(prev);
      undoRef.current = updated;
      return updated;
    });
  }, []);

  const dragProject = useCallback(
    (next: Project) => {
      setUndo((prev) => {
        if (!prev) return prev;
        const updated = updateDragEdit(prev, next);
        undoRef.current = updated;
        return updated;
      });
      pushToBackend(next, false);
    },
    [pushToBackend],
  );

  const endDrag = useCallback(() => {
    setUndo((prev) => {
      if (!prev) return prev;
      const updated = endDragEdit(prev);
      undoRef.current = updated;
      return updated;
    });
  }, []);

  const handleUndo = useCallback(() => {
    const cur = undoRef.current;
    if (!cur || cur.past.length === 0) return;
    const next = undoEdit(cur);
    undoRef.current = next;
    setUndo(next);
    pushToBackend(next.present, false);
  }, [pushToBackend]);

  const handleRedo = useCallback(() => {
    const cur = undoRef.current;
    if (!cur || cur.future.length === 0) return;
    const next = redoEdit(cur);
    undoRef.current = next;
    setUndo(next);
    pushToBackend(next.present, false);
  }, [pushToBackend]);

  const handleTogglePlay = useCallback(() => {
    if (!project) return;
    setPlaying((wasPlaying) => {
      if (wasPlaying) return false;
      const trimEnd = resolvedTrimEnd(project.timeline.trim, duration);
      if (easingRef.current.value >= trimEnd - 0.02) {
        easingRef.current.snap(project.timeline.trim.start);
        setPlayheadTime(project.timeline.trim.start);
      }
      return true;
    });
  }, [project, duration]);

  const handleStepFrame = useCallback(
    (deltaFrames: number) => {
      if (!project) return;
      setPlaying(false);
      const fps = Math.max(project.assets.frameRate || 60, 30);
      const next = clampToTrim(
        easingRef.current.target + deltaFrames / fps,
        project.timeline.trim,
        duration,
      );
      easingRef.current.snap(next);
      setPlayheadTime(next);
    },
    [project, duration],
  );

  const handleJumpTime = useCallback(
    (t: number) => {
      if (!project) return;
      setPlaying(false);
      const clamped = Math.min(duration, Math.max(0, t));
      easingRef.current.glide(clamped);
      setPlayheadTime(easingRef.current.value);
      setGlideTick((g) => g + 1);
    },
    [project, duration],
  );

  const handleScrubTime = useCallback(
    (t: number) => {
      setPlaying(false);
      const clamped = Math.min(duration, Math.max(0, t));
      easingRef.current.snap(clamped);
      setPlayheadTime(clamped);
    },
    [duration],
  );

  const hasProject = project !== null;
  const trimEnd = project ? resolvedTrimEnd(project.timeline.trim, duration) : duration;

  // Playback & PlayheadEasing animation loop.
  useEffect(() => {
    if (!hasProject) return;
    let rafId = 0;
    let lastTs = performance.now();

    const tick = (now: number) => {
      const dt = Math.min(0.1, Math.max(0, (now - lastTs) / 1000));
      lastTs = now;

      if (playing) {
        const next = easingRef.current.value + dt;
        if (next >= trimEnd) {
          easingRef.current.snap(trimEnd);
          setPlayheadTime(trimEnd);
          setPlaying(false);
          return;
        }
        easingRef.current.snap(next);
        setPlayheadTime(next);
        rafId = requestAnimationFrame(tick);
      } else if (easingRef.current.isAnimating) {
        const next = easingRef.current.advance(dt);
        setPlayheadTime(next);
        if (easingRef.current.isAnimating) {
          rafId = requestAnimationFrame(tick);
        }
      }
    };

    if (playing || easingRef.current.isAnimating) {
      rafId = requestAnimationFrame(tick);
    }
    return () => cancelAnimationFrame(rafId);
  }, [playing, glideTick, hasProject, trimEnd]);

  // Global editor keyboard shortcuts.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement | null)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;

      if (e.code === "Space") {
        e.preventDefault();
        handleTogglePlay();
      } else if (e.key === "ArrowLeft") {
        e.preventDefault();
        handleStepFrame(-1);
      } else if (e.key === "ArrowRight") {
        e.preventDefault();
        handleStepFrame(1);
      } else if ((e.key === "Delete" || e.key === "Backspace") && selectedZoomId && project) {
        e.preventDefault();
        commitProject({
          ...project,
          timeline: {
            ...project.timeline,
            zoomBlocks: deleteZoomBlock(selectedZoomId, project.timeline.zoomBlocks),
          },
        });
        setSelectedZoomId(null);
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") {
        e.preventDefault();
        if (e.shiftKey) handleRedo();
        else handleUndo();
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "y") {
        e.preventDefault();
        handleRedo();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    handleTogglePlay,
    handleStepFrame,
    selectedZoomId,
    project,
    commitProject,
    handleUndo,
    handleRedo,
  ]);

  const handleBack = async (): Promise<void> => {
    setPlaying(false);
    const latest = latestProjectRef.current;
    if (latest && latest.url === reference.url) {
      try {
        await updateEditorProject(reference, latest.project, true);
      } catch {
        // Don't trap the user in the editor on a transient flush error.
      }
    }
    onBack();
  };

  if (error) {
    return (
      <div className="editor-loading">
        <div className="error-banner">
          <span>Failed to open project: {error}</span>
          <button type="button" className="btn-secondary" onClick={onBack}>
            {strings.backToRecordings}
          </button>
        </div>
      </div>
    );
  }

  if (!project || !undo) {
    return (
      <div className="editor-loading">
        <p className="muted">Opening project...</p>
      </div>
    );
  }

  const selectedZoom =
    project.timeline.zoomBlocks.find((b) => b.id === selectedZoomId) ?? null;

  const handleTimelineUpdate = (nextTl: Timeline): void => {
    dragProject({ ...project, timeline: nextTl });
  };

  const handlePickManualFocus = (point: NormalizedPoint): void => {
    if (!selectedZoom) return;
    commitProject({
      ...project,
      timeline: {
        ...project.timeline,
        zoomBlocks: setZoomFocus(
          selectedZoom.id,
          { manual: point },
          project.timeline.zoomBlocks,
        ),
      },
    });
  };

  const handleMoveCamera = (point: NormalizedPoint): void => {
    dragProject({
      ...project,
      style: {
        ...project.style,
        camera: { ...project.style.camera, position: point },
      },
    });
  };

  const commitTitle = (): void => {
    const trimmed = titleDraft.trim();
    if (!trimmed) {
      setTitleDraft(project.name);
      return;
    }
    if (trimmed !== project.name) {
      commitProject({ ...project, name: trimmed });
    }
  };

  return (
    <div className="editor-shell">
      <header className="editor-topbar">
        <div className="topbar-left">
          <button type="button" className="btn-secondary compact" onClick={() => void handleBack()}>
            ← {strings.backToRecordings}
          </button>
          <input
            className="editor-title-input"
            value={titleDraft}
            aria-label="Project title"
            onChange={(e) => setTitleDraft(e.target.value)}
            onBlur={commitTitle}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.currentTarget.blur();
              } else if (e.key === "Escape") {
                setTitleDraft(project.name);
                e.currentTarget.blur();
              }
            }}
          />
        </div>

        <div className="topbar-center">
          <div className="pill-group compact">
            {(["r16x9", "r1x1", "r9x16"] as AspectRatio[]).map((ar) => (
              <button
                key={ar}
                type="button"
                className={project.style.aspectRatio === ar ? "active" : ""}
                onClick={() =>
                  commitProject({ ...project, style: { ...project.style, aspectRatio: ar } })
                }
              >
                {aspectLabel(ar)}
              </button>
            ))}
          </div>
        </div>

        <div className="topbar-right">
          <button
            type="button"
            className="icon-btn"
            title={strings.undo}
            disabled={undo.past.length === 0}
            onClick={handleUndo}
          >
            ↶
          </button>
          <button
            type="button"
            className="icon-btn"
            title={strings.redo}
            disabled={undo.future.length === 0}
            onClick={handleRedo}
          >
            ↷
          </button>
          <button type="button" className="btn-primary" onClick={() => setShowExport(true)}>
            {strings.export}
          </button>
        </div>
      </header>

      <div className="editor-workspace">
        <div className="editor-main-column">
          <PreviewCanvas
            project={project}
            projectRevision={revision}
            currentTime={playheadTime}
            duration={duration}
            playing={playing}
            rendererName={rendererName}
            selectedZoom={selectedZoom}
            onTogglePlay={handleTogglePlay}
            onStepFrame={handleStepFrame}
            onPickManualFocus={handlePickManualFocus}
            onBeginCameraDrag={beginDrag}
            onMoveCameraPosition={handleMoveCamera}
            onEndCameraDrag={endDrag}
          />
          <TimelinePanel
            project={project}
            track={track}
            duration={duration}
            playheadTime={playheadTime}
            selectedZoomId={selectedZoomId}
            onJumpTime={handleJumpTime}
            onScrubTime={handleScrubTime}
            onSelectZoom={setSelectedZoomId}
            onBeginDrag={beginDrag}
            onUpdateTimeline={handleTimelineUpdate}
            onEndDrag={endDrag}
          />
        </div>

        <InspectorPanel
          project={project}
          settings={settings}
          selectedZoomId={selectedZoomId}
          currentTime={playheadTime}
          duration={duration}
          onSettingsChange={onSettingsChange}
          onCommitProject={commitProject}
          onBeginDrag={beginDrag}
          onDragProject={dragProject}
          onEndDrag={endDrag}
          onSelectZoom={setSelectedZoomId}
        />
      </div>

      {showExport && (
        <ExportModal
          reference={reference}
          project={project}
          defaultOutputDir={settings.defaultExportDir}
          onClose={() => setShowExport(false)}
        />
      )}
    </div>
  );
}
