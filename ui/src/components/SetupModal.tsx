import { useEffect, useState } from "react";
import {
  listCaptureSources,
  listDevices,
  openOverlay,
  saveSettings,
  startRecording,
} from "../ipc";
import { strings } from "../strings";
import type {
  CaptureSource,
  CaptureSourceList,
  DevicesList,
  ShellSettings,
} from "../types";

interface SetupModalProps {
  settings: ShellSettings;
  onSettingsChange: (s: ShellSettings) => void;
  onClose: () => void;
}

type SourceTab = "display" | "window" | "area";

function isSourceLive(source: CaptureSource | null, list: CaptureSourceList): boolean {
  if (!source) return false;
  if ("display" in source) {
    return list.displays.some((d) => d.id === source.display.id);
  }
  if ("window" in source) {
    return list.windows.some((w) => w.id === source.window.id);
  }
  return list.displays.some((d) => d.id === source.area.displayID);
}

export function SetupModal({
  settings,
  onSettingsChange,
  onClose,
}: SetupModalProps): React.JSX.Element {
  const [sources, setSources] = useState<CaptureSourceList>({ displays: [], windows: [] });
  const [devices, setDevices] = useState<DevicesList>({ microphones: [], cameras: [] });
  const [tab, setTab] = useState<SourceTab>(() => {
    if (!settings.source) return "display";
    if ("window" in settings.source) return "window";
    if ("area" in settings.source) return "area";
    return "display";
  });
  const [selectedSource, setSelectedSource] = useState<CaptureSource | null>(settings.source);
  const [areaDisplayId, setAreaDisplayId] = useState<number | null>(() => {
    if (settings.source && "area" in settings.source) return settings.source.area.displayID;
    if (settings.source && "display" in settings.source) return settings.source.display.id;
    return null;
  });
  const [micEnabled, setMicEnabled] = useState(settings.micEnabled);
  const [micId, setMicId] = useState<string>(settings.micDeviceId ?? "");
  const [camEnabled, setCamEnabled] = useState(settings.cameraEnabled);
  const [camId, setCamId] = useState<string>(settings.cameraDeviceId ?? "");
  const [systemAudio, setSystemAudio] = useState(settings.captureSystemAudio);
  const [countdown, setCountdown] = useState(settings.countdownSeconds);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Sync when `settings.source` updates externally (e.g. after dragging an Area).
  useEffect(() => {
    if (!settings.source) return;
    setSelectedSource(settings.source);
    if ("area" in settings.source) {
      setTab("area");
      setAreaDisplayId(settings.source.area.displayID);
    } else if ("window" in settings.source) {
      setTab("window");
    } else {
      setTab("display");
      setAreaDisplayId(settings.source.display.id);
    }
  }, [settings.source]);

  useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const [srcList, devList] = await Promise.all([listCaptureSources(), listDevices()]);
        if (!active) return;
        setSources(srcList);
        setDevices(devList);

        const primaryDisplay = srcList.displays.find((d) => d.isMain) ?? srcList.displays[0];
        if (!isSourceLive(selectedSource, srcList) && primaryDisplay) {
          setSelectedSource({ display: { id: primaryDisplay.id } });
          setTab("display");
        }
        setAreaDisplayId((prev) => {
          if (prev !== null && srcList.displays.some((d) => d.id === prev)) return prev;
          return primaryDisplay ? primaryDisplay.id : null;
        });

        // Validate persisted microphone & camera IDs against live hardware.
        if (devList.microphones.length === 0) {
          setMicEnabled(false);
          setMicId("");
        } else if (!devList.microphones.some((m) => m.id === micId)) {
          const def = devList.microphones.find((m) => m.isDefault) ?? devList.microphones[0]!;
          setMicId(def.id);
        }

        if (devList.cameras.length === 0) {
          setCamEnabled(false);
          setCamId("");
        } else if (!devList.cameras.some((c) => c.id === camId)) {
          const def = devList.cameras.find((c) => c.isDefault) ?? devList.cameras[0]!;
          setCamId(def.id);
        }
      } catch (e) {
        if (active) setError(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      active = false;
    };
  }, []);

  const handleTabChange = (nextTab: SourceTab): void => {
    setTab(nextTab);
    if (nextTab === "display") {
      if (!selectedSource || !("display" in selectedSource)) {
        const d = sources.displays[0];
        if (d) setSelectedSource({ display: { id: d.id } });
      }
    } else if (nextTab === "window") {
      if (!selectedSource || !("window" in selectedSource)) {
        const w = sources.windows[0];
        if (w) setSelectedSource({ window: { id: w.id } });
      }
    }
  };

  const persistCurrent = async (sourceOverride?: CaptureSource | null): Promise<ShellSettings> => {
    const next: ShellSettings = {
      ...settings,
      source: sourceOverride !== undefined ? sourceOverride : selectedSource,
      micEnabled: micEnabled && devices.microphones.length > 0,
      micDeviceId: micId || null,
      captureSystemAudio: systemAudio,
      cameraEnabled: camEnabled && devices.cameras.length > 0,
      cameraDeviceId: camId || null,
      countdownSeconds: countdown,
    };
    const saved = await saveSettings(next);
    onSettingsChange(saved);
    return saved;
  };

  const handleRecord = async (): Promise<void> => {
    const fallback = sources.displays[0] ? { display: { id: sources.displays[0].id } } : null;
    const src = isSourceLive(selectedSource, sources) ? selectedSource : fallback;
    if (!src) {
      setError("No display or window is available to record.");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await persistCurrent(src);
      await startRecording({
        source: src,
        micDeviceId: micEnabled && devices.microphones.length > 0 ? micId || "default" : null,
        cameraDeviceId: camEnabled && devices.cameras.length > 0 ? camId || "default" : null,
        captureSystemAudio: systemAudio,
        countdownSeconds: countdown,
      });
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleSelectArea = async (): Promise<void> => {
    setError(null);
    try {
      const targetDisplay =
        areaDisplayId !== null
          ? { display: { id: areaDisplayId } }
          : sources.displays[0]
            ? { display: { id: sources.displays[0].id } }
            : selectedSource;
      await persistCurrent(targetDisplay);
      await openOverlay("areaSelection");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const handlePopOut = async (): Promise<void> => {
    setError(null);
    try {
      await persistCurrent();
      await openOverlay("setupBar");
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const noMics = devices.microphones.length === 0;
  const noCams = devices.cameras.length === 0;

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal-card modal-wide" onClick={(e) => e.stopPropagation()}>
        <header className="modal-header">
          <h2>{strings.setupTitle}</h2>
          <button className="icon-btn" type="button" onClick={onClose}>
            ✕
          </button>
        </header>

        <div className="segmented-tabs">
          {(["display", "window", "area"] as const).map((t) => (
            <button
              key={t}
              type="button"
              className={tab === t ? "active" : ""}
              onClick={() => handleTabChange(t)}
            >
              {t === "display"
                ? strings.sourceDisplay
                : t === "window"
                  ? strings.sourceWindow
                  : strings.sourceArea}
            </button>
          ))}
        </div>

        <div className="source-list">
          {tab === "display" &&
            sources.displays.map((d) => {
              const active =
                selectedSource !== null &&
                "display" in selectedSource &&
                selectedSource.display.id === d.id;
              return (
                <button
                  key={d.id}
                  type="button"
                  className={`source-item ${active ? "selected" : ""}`}
                  onClick={() => {
                    setSelectedSource({ display: { id: d.id } });
                    setAreaDisplayId(d.id);
                  }}
                >
                  <span className="source-title">{d.name}</span>
                  <span className="source-sub">
                    {d.pixelSize.width}×{d.pixelSize.height}
                  </span>
                </button>
              );
            })}

          {tab === "window" &&
            (sources.windows.length === 0 ? (
              <p className="muted">No capturable windows detected.</p>
            ) : (
              sources.windows.map((w) => {
                const active =
                  selectedSource !== null &&
                  "window" in selectedSource &&
                  selectedSource.window.id === w.id;
                return (
                  <button
                    key={w.hwnd}
                    type="button"
                    className={`source-item ${active ? "selected" : ""}`}
                    onClick={() => setSelectedSource({ window: { id: w.id } })}
                  >
                    <span className="source-title">{w.title}</span>
                    <span className="source-sub">{w.appName}</span>
                  </button>
                );
              })
            ))}

          {tab === "area" && (
            <div className="area-box">
              {sources.displays.length > 1 && (
                <label className="device-row" style={{ width: "100%" }}>
                  <span>Target Display</span>
                  <select
                    value={areaDisplayId ?? ""}
                    onChange={(e) => setAreaDisplayId(Number(e.target.value))}
                  >
                    {sources.displays.map((d) => (
                      <option key={d.id} value={d.id}>
                        {d.name}
                      </option>
                    ))}
                  </select>
                </label>
              )}
              <p className="muted">
                {selectedSource && "area" in selectedSource
                  ? `Selected area: ${Math.round(selectedSource.area.rect.size.width)} × ${Math.round(selectedSource.area.rect.size.height)} pt`
                  : "Drag a rectangle on the chosen display to capture a cropped region."}
              </p>
              <button className="btn-secondary" type="button" onClick={() => void handleSelectArea()}>
                {strings.chooseArea}
              </button>
            </div>
          )}
        </div>

        <div className="setup-devices">
          <label className="device-row">
            <input
              type="checkbox"
              disabled={noMics}
              checked={micEnabled && !noMics}
              onChange={(e) => setMicEnabled(e.target.checked)}
            />
            <span>{strings.microphone}</span>
            <select
              disabled={!micEnabled || noMics}
              value={micId}
              onChange={(e) => setMicId(e.target.value)}
            >
              {noMics && <option value="">No microphone</option>}
              {devices.microphones.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name}
                  {m.isDefault ? " (Default)" : ""}
                </option>
              ))}
            </select>
          </label>

          <label className="device-row">
            <input
              type="checkbox"
              disabled={noCams}
              checked={camEnabled && !noCams}
              onChange={(e) => setCamEnabled(e.target.checked)}
            />
            <span>{strings.camera}</span>
            <select
              disabled={!camEnabled || noCams}
              value={camId}
              onChange={(e) => setCamId(e.target.value)}
            >
              {noCams && <option value="">No camera</option>}
              {devices.cameras.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
            </select>
          </label>

          <label className="device-row">
            <input
              type="checkbox"
              checked={systemAudio}
              onChange={(e) => setSystemAudio(e.target.checked)}
            />
            <span>{strings.systemAudio}</span>
            <span className="muted-sm">WASAPI loopback • 48 kHz stereo</span>
          </label>

          <div className="device-row">
            <span className="row-label">{strings.countdown}</span>
            <div className="pill-group">
              {[0, 3, 5].map((sec) => (
                <button
                  key={sec}
                  type="button"
                  className={countdown === sec ? "active" : ""}
                  onClick={() => setCountdown(sec)}
                >
                  {sec === 0 ? strings.countdownOff : `${sec} s`}
                </button>
              ))}
            </div>
          </div>
        </div>

        {error && <div className="inline-error">{error}</div>}

        <footer className="modal-footer">
          <button className="btn-secondary" type="button" onClick={() => void handlePopOut()}>
            {strings.popOutBar}
          </button>
          <div className="footer-right">
            <button className="btn-secondary" type="button" onClick={onClose}>
              {strings.cancel}
            </button>
            <button
              className="btn-record"
              type="button"
              disabled={busy}
              onClick={() => void handleRecord()}
            >
              <span className="rec-glyph" />
              {busy ? "Starting..." : strings.record}
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}
