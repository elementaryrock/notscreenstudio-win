import { useEffect, useRef, useState } from "react";
import {
  cancelCountdown,
  closeOverlay,
  discardRecording,
  getSettings,
  getShellSnapshot,
  listCaptureSources,
  onElapsed,
  onShellState,
  openOverlay,
  pauseRecording,
  resumeRecording,
  saveSettings,
  selectAreaRect,
  startRecording,
  stopRecording,
} from "../ipc";
import { formatDurationShort } from "../editor/editorLogic";
import { strings } from "../strings";
import type { CaptureSourceList, ShellSettings, ShellSnapshot } from "../types";

export function HudOverlayRouter({ hash }: { hash: string }): React.JSX.Element {
  useEffect(() => {
    document.body.classList.add("hud-mode");
    return () => document.body.classList.remove("hud-mode");
  }, []);

  switch (hash) {
    case "#hud-setup":
      return <SetupBarHud />;
    case "#hud-recording":
      return <RecordingBarHud />;
    case "#hud-countdown":
      return <CountdownHud />;
    case "#hud-area":
      return <AreaSelectionHud />;
    case "#hud-camera-bubble":
      return <CameraBubbleHud />;
    default:
      return <div className="hud-capsule">HUD</div>;
  }
}

function SetupBarHud(): React.JSX.Element {
  const [settings, setSettings] = useState<ShellSettings | null>(null);
  const [sources, setSources] = useState<CaptureSourceList>({ displays: [], windows: [] });
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void Promise.all([getSettings(), listCaptureSources()]).then(([s, src]) => {
      setSettings(s);
      setSources(src);
    });
  }, []);

  const toggleMic = async (): Promise<void> => {
    if (!settings) return;
    const saved = await saveSettings({ ...settings, micEnabled: !settings.micEnabled });
    setSettings(saved);
  };

  const toggleCam = async (): Promise<void> => {
    if (!settings) return;
    const saved = await saveSettings({ ...settings, cameraEnabled: !settings.cameraEnabled });
    setSettings(saved);
  };

  const handleRecord = async (): Promise<void> => {
    if (!settings) return;
    const src =
      settings.source ?? (sources.displays[0] ? { display: { id: sources.displays[0].id } } : null);
    if (!src) return;
    setBusy(true);
    try {
      await startRecording({
        source: src,
        micDeviceId: settings.micEnabled ? settings.micDeviceId ?? "default" : null,
        cameraDeviceId: settings.cameraEnabled ? settings.cameraDeviceId ?? "default" : null,
        countdownSeconds: settings.countdownSeconds,
      });
    } finally {
      setBusy(false);
    }
  };

  const modeLabel =
    settings?.source && "window" in settings.source
      ? strings.sourceWindow
      : settings?.source && "area" in settings.source
        ? strings.sourceArea
        : strings.sourceDisplay;

  return (
    <div className="hud-shell">
      <div className="hud-capsule" data-tauri-drag-region>
        <span className="hud-chip">{modeLabel}</span>
        <button
          type="button"
          className="hud-btn"
          onClick={() => void openOverlay("areaSelection")}
        >
          {strings.sourceArea}
        </button>
        <button
          type="button"
          className={`hud-btn ${settings?.micEnabled ? "on" : ""}`}
          onClick={() => void toggleMic()}
        >
          Mic {settings?.micEnabled ? "On" : "Off"}
        </button>
        <button
          type="button"
          className={`hud-btn ${settings?.cameraEnabled ? "on" : ""}`}
          onClick={() => void toggleCam()}
        >
          Cam {settings?.cameraEnabled ? "On" : "Off"}
        </button>
        <button
          type="button"
          className="btn-record compact"
          disabled={busy}
          onClick={() => void handleRecord()}
        >
          <span className="rec-glyph" />
          {strings.record}
        </button>
        <button
          type="button"
          className="icon-btn"
          aria-label={strings.close}
          onClick={() => void closeOverlay("setupBar")}
        >
          ✕
        </button>
      </div>
    </div>
  );
}

function RecordingBarHud(): React.JSX.Element {
  const [shell, setShell] = useState<ShellSnapshot | null>(null);
  const [elapsed, setElapsed] = useState(0);

  useEffect(() => {
    void getShellSnapshot().then((s) => {
      setShell(s);
      setElapsed(s.elapsedSeconds);
    });
    let un1: (() => void) | undefined;
    let un2: (() => void) | undefined;
    void onShellState((s) => {
      setShell(s);
      setElapsed(s.elapsedSeconds);
    }).then((f) => {
      un1 = f;
    });
    void onElapsed((secs) => setElapsed(secs)).then((f) => {
      un2 = f;
    });
    return () => {
      un1?.();
      un2?.();
    };
  }, []);

  const paused = shell?.paused ?? false;

  return (
    <div className="hud-shell">
      <div className="hud-capsule recording" data-tauri-drag-region>
        <span className={`pulse-dot ${paused ? "paused" : ""}`} />
        <span className="mono hud-timer">{formatDurationShort(elapsed)}</span>
        <button
          type="button"
          className="hud-btn"
          onClick={() => void (paused ? resumeRecording() : pauseRecording())}
        >
          {paused ? strings.resume : strings.pause}
        </button>
        <button
          type="button"
          className="btn-danger compact"
          onClick={() => void stopRecording()}
        >
          {strings.stop}
        </button>
        <button
          type="button"
          className="icon-btn"
          title={strings.discard}
          onClick={() => void discardRecording()}
        >
          🗑
        </button>
      </div>
    </div>
  );
}

function CountdownHud(): React.JSX.Element {
  const [remaining, setRemaining] = useState<number>(3);

  useEffect(() => {
    void getShellSnapshot().then((s) => {
      if (s.countdownRemaining !== null) setRemaining(s.countdownRemaining);
    });
    let un: (() => void) | undefined;
    void onShellState((s) => {
      if (s.countdownRemaining !== null) setRemaining(s.countdownRemaining);
    }).then((f) => {
      un = f;
    });
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") void cancelCountdown();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      un?.();
      window.removeEventListener("keydown", onKey);
    };
  }, []);

  return (
    <div className="hud-countdown" onClick={() => void cancelCountdown()}>
      <div className="countdown-ring">
        <span className="countdown-digit">{remaining}</span>
        <span className="countdown-hint">Esc or click to cancel</span>
      </div>
    </div>
  );
}

function AreaSelectionHud(): React.JSX.Element {
  const [displayId, setDisplayId] = useState<number>(0);
  const [startPt, setStartPt] = useState<{ x: number; y: number } | null>(null);
  const [currPt, setCurrPt] = useState<{ x: number; y: number } | null>(null);
  const draggingRef = useRef(false);

  useEffect(() => {
    void listCaptureSources().then((src) => {
      const main = src.displays.find((d) => d.isMain) ?? src.displays[0];
      if (main) setDisplayId(main.id);
    });
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") void closeOverlay("areaSelection");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const rect =
    startPt && currPt
      ? {
          x: Math.min(startPt.x, currPt.x),
          y: Math.min(startPt.y, currPt.y),
          w: Math.abs(currPt.x - startPt.x),
          h: Math.abs(currPt.y - startPt.y),
        }
      : null;

  return (
    <div
      className="hud-area-scrim"
      onPointerDown={(e) => {
        draggingRef.current = true;
        setStartPt({ x: e.clientX, y: e.clientY });
        setCurrPt({ x: e.clientX, y: e.clientY });
      }}
      onPointerMove={(e) => {
        if (!draggingRef.current) return;
        setCurrPt({ x: e.clientX, y: e.clientY });
      }}
      onPointerUp={() => {
        if (!draggingRef.current) return;
        draggingRef.current = false;
        if (rect && rect.w >= 32 && rect.h >= 32) {
          const dpr = window.devicePixelRatio || 1;
          void selectAreaRect(displayId, {
            origin: { x: Math.round(rect.x * dpr), y: Math.round(rect.y * dpr) },
            size: { width: Math.round(rect.w * dpr), height: Math.round(rect.h * dpr) },
          });
        } else {
          void closeOverlay("areaSelection");
        }
      }}
    >
      <div className="hud-area-banner">{strings.selectAreaPrompt}</div>
      {rect && (
        <div
          className="hud-area-rect"
          style={{
            left: rect.x,
            top: rect.y,
            width: rect.w,
            height: rect.h,
          }}
        >
          <span className="hud-area-dims mono">
            {Math.round(rect.w)} × {Math.round(rect.h)}
          </span>
        </div>
      )}
    </div>
  );
}

function CameraBubbleHud(): React.JSX.Element {
  const [settings, setSettings] = useState<ShellSettings | null>(null);

  useEffect(() => {
    void getSettings().then(setSettings);
  }, []);

  const shape = settings?.cameraShape ?? "circle";
  return (
    <div className="hud-shell">
      <div className={`hud-camera-bubble shape-${shape}`} data-tauri-drag-region>
        <span>Camera Active</span>
      </div>
    </div>
  );
}
