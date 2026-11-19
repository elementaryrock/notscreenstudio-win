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
  const [micEnabled, setMicEnabled] = useState(settings.micEnabled);
  const [micId, setMicId] = useState<string>(settings.micDeviceId ?? "");
  const [camEnabled, setCamEnabled] = useState(settings.cameraEnabled);
  const [camId, setCamId] = useState<string>(settings.cameraDeviceId ?? "");
  const [systemAudio, setSystemAudio] = useState(false);
  const [countdown, setCountdown] = useState(settings.countdownSeconds);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void (async () => {
      const [srcList, devList] = await Promise.all([listCaptureSources(), listDevices()]);
      setSources(srcList);
      setDevices(devList);
      if (!selectedSource && srcList.displays[0]) {
        setSelectedSource({ display: { id: srcList.displays[0].id } });
      }
      if (!micId && devList.microphones[0]) {
        const def = devList.microphones.find((m) => m.isDefault) ?? devList.microphones[0];
        setMicId(def.id);
      }
      if (!camId && devList.cameras[0]) {
        const def = devList.cameras.find((c) => c.isDefault) ?? devList.cameras[0];
        setCamId(def.id);
      }
    })();
  }, []);

  const persistCurrent = async (sourceOverride?: CaptureSource | null): Promise<ShellSettings> => {
    const next: ShellSettings = {
      ...settings,
      source: sourceOverride !== undefined ? sourceOverride : selectedSource,
      micEnabled,
      micDeviceId: micId || null,
      cameraEnabled: camEnabled,
      cameraDeviceId: camId || null,
      countdownSeconds: countdown,
    };
    const saved = await saveSettings(next);
    onSettingsChange(saved);
    return saved;
  };

  const handleRecord = async (): Promise<void> => {
    const src =
      selectedSource ??
      (sources.displays[0] ? { display: { id: sources.displays[0].id } } : null);
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
        micDeviceId: micEnabled ? micId || "default" : null,
        cameraDeviceId: camEnabled ? camId || "default" : null,
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
    await persistCurrent();
    await openOverlay("areaSelection");
  };

  const handlePopOut = async (): Promise<void> => {
    await persistCurrent();
    await openOverlay("setupBar");
    onClose();
  };

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
              onClick={() => setTab(t)}
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
                selectedSource !== null && "display" in selectedSource && selectedSource.display.id === d.id;
              return (
                <button
                  key={d.id}
                  type="button"
                  className={`source-item ${active ? "selected" : ""}`}
                  onClick={() => setSelectedSource({ display: { id: d.id } })}
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
                  selectedSource !== null && "window" in selectedSource && selectedSource.window.id === w.id;
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
              <p className="muted">
                {selectedSource && "area" in selectedSource
                  ? `Selected area: ${Math.round(selectedSource.area.rect.size.width)} × ${Math.round(selectedSource.area.rect.size.height)} px`
                  : "Drag a rectangle on any display to capture a cropped region."}
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
              checked={micEnabled}
              onChange={(e) => setMicEnabled(e.target.checked)}
            />
            <span>{strings.microphone}</span>
            <select
              disabled={!micEnabled}
              value={micId}
              onChange={(e) => setMicId(e.target.value)}
            >
              {devices.microphones.length === 0 && <option value="">No microphone</option>}
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
              checked={camEnabled}
              onChange={(e) => setCamEnabled(e.target.checked)}
            />
            <span>{strings.camera}</span>
            <select
              disabled={!camEnabled}
              value={camId}
              onChange={(e) => setCamId(e.target.value)}
            >
              {devices.cameras.length === 0 && <option value="">No camera</option>}
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
            <span className="muted-sm">48 kHz stereo</span>
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
