import { useEffect, useRef, useState } from "react";
import {
  cancelCountdown,
  closeOverlay,
  discardRecording,
  getSettings,
  getShellSnapshot,
  listCaptureSources,
  onAreaSelected,
  onElapsed,
  onSettingsChanged,
  onShellState,
  openOverlay,
  pauseRecording,
  resumeRecording,
  saveSettings,
  selectAreaRect,
  startRecording,
  stopRecording,
} from "../ipc";
import { formatElapsedTimer } from "../editor/editorLogic";
import { strings } from "../strings";
import type { CaptureSource, CaptureSourceList, ShellSettings, ShellSnapshot } from "../types";

function isLiveSource(source: CaptureSource | null, list: CaptureSourceList): boolean {
  if (!source) return false;
  if ("display" in source) return list.displays.some((d) => d.id === source.display.id);
  if ("window" in source) return list.windows.some((w) => w.id === source.window.id);
  return list.displays.some((d) => d.id === source.area.displayID);
}

export function HudOverlayRouter({ hash }: { hash: string }): React.JSX.Element {
  useEffect(() => {
    document.documentElement.classList.add("hud-mode");
    document.body.classList.add("hud-mode");
    return () => {
      document.documentElement.classList.remove("hud-mode");
      document.body.classList.remove("hud-mode");
    };
  }, []);

  const route = hash.split("?")[0] ?? hash;
  switch (route) {
    case "#hud-setup":
      return <SetupBarHud />;
    case "#hud-recording":
      return <RecordingBarHud />;
    case "#hud-countdown":
      return <CountdownHud />;
    case "#hud-area":
      return <AreaSelectionHud hash={hash} />;
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
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    const cleanups: Array<() => void> = [];
    const register = (p: Promise<() => void>) => {
      void p
        .then((fn) => {
          if (disposed) fn();
          else cleanups.push(fn);
        })
        .catch(() => {});
    };

    void Promise.all([getSettings(), listCaptureSources()])
      .then(([s, src]) => {
        if (disposed) return;
        setSettings(s);
        setSources(src);
      })
      .catch(() => {});

    register(onSettingsChanged((next) => setSettings(next)));
    register(
      onAreaSelected((src) => {
        if (src) {
          setSettings((prev) => (prev ? { ...prev, source: src } : prev));
        }
      }),
    );

    return () => {
      disposed = true;
      for (const fn of cleanups) fn();
    };
  }, []);

  const toggleMic = async (): Promise<void> => {
    if (!settings) return;
    try {
      const saved = await saveSettings({ ...settings, micEnabled: !settings.micEnabled });
      setSettings(saved);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const toggleCam = async (): Promise<void> => {
    if (!settings) return;
    try {
      const saved = await saveSettings({ ...settings, cameraEnabled: !settings.cameraEnabled });
      setSettings(saved);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const handleRecord = async (): Promise<void> => {
    if (!settings) return;
    const freshSources = await listCaptureSources().catch(() => sources);
    setSources(freshSources);
    const fallback = freshSources.displays[0]
      ? { display: { id: freshSources.displays[0].id } }
      : null;
    const src = isLiveSource(settings.source, freshSources) ? settings.source : fallback;
    if (!src) {
      setError("No display available.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await startRecording({
        source: src,
        micDeviceId: settings.micEnabled ? (settings.micDeviceId ?? "default") : null,
        cameraDeviceId: settings.cameraEnabled ? (settings.cameraDeviceId ?? "default") : null,
        captureSystemAudio: settings.captureSystemAudio,
        countdownSeconds: settings.countdownSeconds,
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
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
          onClick={() => void openOverlay("areaSelection").catch(() => {})}
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
          onClick={() => void closeOverlay("setupBar").catch(() => {})}
        >
          ✕
        </button>
      </div>
      {error && <div className="hud-error-pill">{error}</div>}
    </div>
  );
}

function RecordingBarHud(): React.JSX.Element {
  const [shell, setShell] = useState<ShellSnapshot | null>(null);
  const [elapsed, setElapsed] = useState(0);

  useEffect(() => {
    let disposed = false;
    const cleanups: Array<() => void> = [];
    const register = (p: Promise<() => void>) => {
      void p
        .then((fn) => {
          if (disposed) fn();
          else cleanups.push(fn);
        })
        .catch(() => {});
    };

    void getShellSnapshot()
      .then((s) => {
        if (disposed) return;
        setShell(s);
        setElapsed(s.elapsedSeconds);
      })
      .catch(() => {});

    register(
      onShellState((s) => {
        setShell(s);
        setElapsed(s.elapsedSeconds);
      }),
    );
    register(onElapsed((secs) => setElapsed(secs)));

    return () => {
      disposed = true;
      for (const fn of cleanups) fn();
    };
  }, []);

  const paused = shell?.paused ?? false;

  return (
    <div className="hud-shell">
      <div className="hud-capsule recording" data-tauri-drag-region>
        <span className={`pulse-dot ${paused ? "paused" : ""}`} />
        <span className="mono hud-timer">{formatElapsedTimer(elapsed)}</span>
        <button
          type="button"
          className="hud-btn"
          onClick={() =>
            void (paused ? resumeRecording() : pauseRecording()).catch(() => {})
          }
        >
          {paused ? strings.resume : strings.pause}
        </button>
        <button
          type="button"
          className="btn-danger compact"
          onClick={() => void stopRecording().catch(() => {})}
        >
          {strings.stop}
        </button>
        <button
          type="button"
          className="icon-btn"
          title={strings.discard}
          onClick={() => void discardRecording().catch(() => {})}
        >
          🗑
        </button>
      </div>
    </div>
  );
}

function CountdownHud(): React.JSX.Element {
  const [remaining, setRemaining] = useState<number | null>(null);

  useEffect(() => {
    let disposed = false;
    let un: (() => void) | undefined;

    void getShellSnapshot()
      .then((s) => {
        if (!disposed && s.countdownRemaining !== null) {
          setRemaining(s.countdownRemaining);
        }
      })
      .catch(() => {});

    void onShellState((s) => {
      if (s.countdownRemaining !== null) setRemaining(s.countdownRemaining);
    })
      .then((f) => {
        if (disposed) f();
        else un = f;
      })
      .catch(() => {});

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") void cancelCountdown().catch(() => {});
    };
    window.addEventListener("keydown", onKey);
    return () => {
      disposed = true;
      un?.();
      window.removeEventListener("keydown", onKey);
    };
  }, []);

  return (
    <div className="hud-countdown" onClick={() => void cancelCountdown().catch(() => {})}>
      <div className="countdown-ring">
        <span className="countdown-digit">{remaining ?? ""}</span>
        <span className="countdown-hint">Esc or click to cancel</span>
      </div>
    </div>
  );
}

function AreaSelectionHud({ hash }: { hash: string }): React.JSX.Element {
  const [displayId, setDisplayId] = useState<number | null>(() => {
    const query = hash.split("?")[1];
    if (!query) return null;
    const raw = new URLSearchParams(query).get("displayId");
    const parsed = raw ? Number(raw) : NaN;
    return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
  });
  const [startPt, setStartPt] = useState<{ x: number; y: number } | null>(null);
  const [currPt, setCurrPt] = useState<{ x: number; y: number } | null>(null);
  const draggingRef = useRef(false);

  useEffect(() => {
    let active = true;
    void listCaptureSources()
      .then((src) => {
        if (!active) return;
        setDisplayId((prev) => {
          if (prev !== null && src.displays.some((d) => d.id === prev)) return prev;
          const main = src.displays.find((d) => d.isMain) ?? src.displays[0];
          return main ? main.id : null;
        });
      })
      .catch(() => {});

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") void closeOverlay("areaSelection").catch(() => {});
    };
    window.addEventListener("keydown", onKey);
    return () => {
      active = false;
      window.removeEventListener("keydown", onKey);
    };
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

  const finishDrag = (e: React.PointerEvent<HTMLDivElement>, cancelled: boolean): void => {
    if (!draggingRef.current) return;
    draggingRef.current = false;
    try {
      e.currentTarget.releasePointerCapture(e.pointerId);
    } catch {
      // Capture may already be released.
    }
    if (!cancelled && displayId !== null && rect && rect.w >= 32 && rect.h >= 32) {
      // `CaptureSource::Area.rect` is in logical display points (CSS pixels); the Rust capture
      // engine and input normalizer multiply by `monitor.scale()` once to obtain physical pixels.
      void selectAreaRect(displayId, {
        origin: { x: Math.round(rect.x), y: Math.round(rect.y) },
        size: { width: Math.round(rect.w), height: Math.round(rect.h) },
      }).catch(() => {});
    } else {
      void closeOverlay("areaSelection").catch(() => {});
    }
  };

  return (
    <div
      className="hud-area-scrim"
      onPointerDown={(e) => {
        if (displayId === null) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        draggingRef.current = true;
        setStartPt({ x: e.clientX, y: e.clientY });
        setCurrPt({ x: e.clientX, y: e.clientY });
      }}
      onPointerMove={(e) => {
        if (!draggingRef.current) return;
        setCurrPt({ x: e.clientX, y: e.clientY });
      }}
      onPointerUp={(e) => finishDrag(e, false)}
      onPointerCancel={(e) => finishDrag(e, true)}
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
    let active = true;
    void getSettings()
      .then((s) => {
        if (active) setSettings(s);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
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
