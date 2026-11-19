import { useEffect, useRef, useState } from "react";
import {
  checkFfmpeg,
  getPermissions,
  requestBorderlessAccess,
  saveSettings,
} from "../ipc";
import { strings } from "../strings";
import {
  aspectLabel,
  type AspectRatio,
  type FfmpegStatusDto,
  type PermissionsDto,
  type ShellSettings,
  type ShellSnapshot,
} from "../types";

interface SettingsModalProps {
  settings: ShellSettings;
  shell: ShellSnapshot | null;
  onSettingsChange: (s: ShellSettings) => void;
  onClose: () => void;
}

export function SettingsModal({
  settings,
  shell,
  onSettingsChange,
  onClose,
}: SettingsModalProps): React.JSX.Element {
  const [perms, setPerms] = useState<PermissionsDto | null>(null);
  const [ffmpeg, setFfmpeg] = useState<FfmpegStatusDto | null>(null);
  const [draft, setDraft] = useState<ShellSettings>(settings);
  const [savedBanner, setSavedBanner] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const savedTimerRef = useRef<number | null>(null);

  useEffect(() => {
    let active = true;
    void Promise.all([getPermissions(), checkFfmpeg()])
      .then(([p, ff]) => {
        if (!active) return;
        setPerms(p);
        setFfmpeg(ff);
      })
      .catch((e) => {
        if (active) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      active = false;
      if (savedTimerRef.current !== null) {
        window.clearTimeout(savedTimerRef.current);
      }
    };
  }, []);

  const handleSave = async (): Promise<void> => {
    setError(null);
    try {
      const saved = await saveSettings(draft);
      onSettingsChange(saved);
      setSavedBanner(true);
      if (savedTimerRef.current !== null) {
        window.clearTimeout(savedTimerRef.current);
      }
      savedTimerRef.current = window.setTimeout(() => setSavedBanner(false), 1800);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const handleBorderless = async (): Promise<void> => {
    setError(null);
    try {
      const next = await requestBorderlessAccess();
      setPerms(next);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal-card modal-wide" onClick={(e) => e.stopPropagation()}>
        <header className="modal-header">
          <h2>{strings.settingsTitle}</h2>
          <button className="icon-btn" type="button" onClick={onClose}>
            ✕
          </button>
        </header>

        <section className="settings-group">
          <h3>{strings.permissionsHeading}</h3>
          <div className="perm-row">
            <span>Screen Capture (WGC)</span>
            <span className="status-chip ok">{perms?.screenRecording ?? "granted"}</span>
          </div>
          <div className="perm-row">
            <span>Microphone</span>
            <span className={`status-chip ${perms?.microphone === "denied" ? "err" : "ok"}`}>
              {perms?.microphone ?? "checking"}
            </span>
          </div>
          <div className="perm-row">
            <span>Camera</span>
            <span className={`status-chip ${perms?.camera === "denied" ? "err" : "ok"}`}>
              {perms?.camera ?? "checking"}
            </span>
          </div>
          <div className="perm-row">
            <div>
              <div>Borderless Capture (Yellow Border)</div>
              <div className="muted-sm">{strings.borderlessHint}</div>
            </div>
            <button
              className="btn-secondary"
              type="button"
              disabled={
                perms?.borderlessCapture === "granted" || perms?.borderlessCapture === "denied"
              }
              onClick={() => void handleBorderless()}
            >
              {perms?.borderlessCapture === "granted"
                ? "Granted"
                : perms?.borderlessCapture === "denied"
                  ? "Declined (restart to retry)"
                  : strings.requestBorderless}
            </button>
          </div>
          <div className="perm-row">
            <div>
              <div>FFmpeg Encoder</div>
              <div className="muted-sm">
                {ffmpeg?.available
                  ? `Found (${ffmpeg.path}) • H.264 ${ffmpeg.h264 ? "✓" : "✗"} • HEVC ${ffmpeg.hevc ? "✓" : "✗"}`
                  : ffmpeg?.message ?? `Tested against FFmpeg ${ffmpeg?.testedVersion ?? "8.1.1"}`}
              </div>
            </div>
            <span className={`status-chip ${ffmpeg?.available ? "ok" : "err"}`}>
              {ffmpeg?.available ? "Ready" : "Missing"}
            </span>
          </div>
        </section>

        <section className="settings-group">
          <h3>{strings.hotkeysHeading}</h3>
          <div className="form-row">
            <label>{strings.recordHotkeyLabel}</label>
            <input
              type="text"
              value={draft.recordHotkey}
              onChange={(e) => setDraft({ ...draft, recordHotkey: e.target.value })}
            />
          </div>
          {shell?.hotkeys.recordConflict && (
            <div className="inline-error">{shell.hotkeys.recordConflict}</div>
          )}
          <div className="form-row">
            <label>{strings.pauseHotkeyLabel}</label>
            <input
              type="text"
              value={draft.pauseHotkey}
              onChange={(e) => setDraft({ ...draft, pauseHotkey: e.target.value })}
            />
          </div>
          {shell?.hotkeys.pauseConflict && (
            <div className="inline-error">{shell.hotkeys.pauseConflict}</div>
          )}
        </section>

        <section className="settings-group">
          <h3>{strings.defaultsHeading}</h3>
          <div className="form-row">
            <label>Default Aspect Ratio</label>
            <div className="pill-group">
              {(["r16x9", "r1x1", "r9x16"] as AspectRatio[]).map((ar) => (
                <button
                  key={ar}
                  type="button"
                  className={draft.defaultAspect === ar ? "active" : ""}
                  onClick={() => setDraft({ ...draft, defaultAspect: ar })}
                >
                  {aspectLabel(ar)}
                </button>
              ))}
            </div>
          </div>
          <div className="form-row">
            <label>Default Countdown</label>
            <div className="pill-group">
              {[0, 3, 5].map((sec) => (
                <button
                  key={sec}
                  type="button"
                  className={draft.countdownSeconds === sec ? "active" : ""}
                  onClick={() => setDraft({ ...draft, countdownSeconds: sec })}
                >
                  {sec === 0 ? strings.countdownOff : `${sec} s`}
                </button>
              ))}
            </div>
          </div>
          <div className="form-row">
            <label>Default Export Folder</label>
            <input
              type="text"
              placeholder="(Same as library folder)"
              value={draft.defaultExportDir ?? ""}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  defaultExportDir: e.target.value.trim() ? e.target.value : null,
                })
              }
            />
          </div>
        </section>

        {error && <div className="inline-error">{error}</div>}

        <footer className="modal-footer">
          <span className="muted-sm">{savedBanner ? "Saved." : "Offline only • zero telemetry"}</span>
          <div className="footer-right">
            <button className="btn-secondary" type="button" onClick={onClose}>
              {strings.close}
            </button>
            <button className="btn-primary" type="button" onClick={() => void handleSave()}>
              {strings.save}
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}
