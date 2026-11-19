import { useCallback, useEffect, useState } from "react";
import {
  coreInfo,
  dismissError,
  getSettings,
  getShellSnapshot,
  onAreaSelected,
  onElapsed,
  onHotkey,
  onRecordingSaved,
  onShellState,
  onTrayAction,
  pauseRecording,
  resumeRecording,
  stopRecording,
} from "./ipc";
import type { CoreInfo, ProjectReference, ShellSettings, ShellSnapshot } from "./types";
import { EditorView } from "./components/EditorView";
import { HomeView } from "./components/HomeView";
import { HudOverlayRouter } from "./components/HudOverlays";
import { SettingsModal } from "./components/SettingsModal";
import { SetupModal } from "./components/SetupModal";

export function App(): React.JSX.Element {
  const hash = window.location.hash;
  if (hash.startsWith("#hud-")) {
    return <HudOverlayRouter hash={hash} />;
  }
  return <MainApp />;
}

function MainApp(): React.JSX.Element {
  const [core, setCore] = useState<CoreInfo | null>(null);
  const [shell, setShell] = useState<ShellSnapshot | null>(null);
  const [settings, setSettings] = useState<ShellSettings | null>(null);
  const [activeProject, setActiveProject] = useState<ProjectReference | null>(null);
  const [showSetup, setShowSetup] = useState(false);
  const [showSettings, setShowSettings] = useState(false);

  useEffect(() => {
    void Promise.all([coreInfo(), getShellSnapshot(), getSettings()]).then(
      ([info, snap, set]) => {
        setCore(info);
        setShell(snap);
        setSettings(set);
      },
    );
  }, []);

  const handleStopRecording = useCallback(async () => {
    try {
      const ref = await stopRecording();
      setActiveProject(ref);
    } catch {
      // Error surfaced via shell-state event.
    }
  }, []);

  const handlePauseToggle = useCallback(async () => {
    if (!shell || shell.phase.kind !== "recording") return;
    if (shell.paused) await resumeRecording();
    else await pauseRecording();
  }, [shell]);

  // Subscribe to backend shell, tray, hotkey, elapsed, and saved-project events.
  useEffect(() => {
    const cleanups: Array<() => void> = [];
    void onShellState((s) => setShell(s)).then((fn) => cleanups.push(fn));
    void onElapsed((secs) =>
      setShell((prev) => (prev ? { ...prev, elapsedSeconds: secs } : prev)),
    ).then((fn) => cleanups.push(fn));
    void onRecordingSaved((ref) => {
      setShowSetup(false);
      setActiveProject(ref);
    }).then((fn) => cleanups.push(fn));
    void onAreaSelected((src) => {
      if (src) {
        setSettings((prev) => (prev ? { ...prev, source: src } : prev));
        setShowSetup(true);
      }
    }).then((fn) => cleanups.push(fn));
    void onTrayAction((action) => {
      if (action === "record") setShowSetup(true);
      else if (action === "settings") setShowSettings(true);
      else if (action === "stop") void handleStopRecording();
      else if (action === "pause") void handlePauseToggle();
      else if (action === "open") setActiveProject(null);
    }).then((fn) => cleanups.push(fn));
    void onHotkey((action) => {
      if (action === "toggle-record") {
        setShell((curr) => {
          if (curr?.phase.kind === "recording") {
            void handleStopRecording();
          } else {
            setShowSetup(true);
          }
          return curr;
        });
      } else if (action === "toggle-pause") {
        void handlePauseToggle();
      }
    }).then((fn) => cleanups.push(fn));

    return () => {
      for (const fn of cleanups) fn();
    };
  }, [handleStopRecording, handlePauseToggle]);

  return (
    <>
      {activeProject && settings ? (
        <EditorView
          reference={activeProject}
          settings={settings}
          onSettingsChange={setSettings}
          onBack={() => setActiveProject(null)}
        />
      ) : (
        <HomeView
          core={core}
          shell={shell}
          onOpenProject={(ref) => setActiveProject(ref)}
          onOpenSetup={() => setShowSetup(true)}
          onOpenSettings={() => setShowSettings(true)}
          onStopRecording={() => void handleStopRecording()}
          onPauseToggle={() => void handlePauseToggle()}
          onDismissError={() => void dismissError()}
        />
      )}

      {showSetup && settings && (
        <SetupModal
          settings={settings}
          onSettingsChange={setSettings}
          onClose={() => setShowSetup(false)}
        />
      )}

      {showSettings && settings && (
        <SettingsModal
          settings={settings}
          shell={shell}
          onSettingsChange={setSettings}
          onClose={() => setShowSettings(false)}
        />
      )}
    </>
  );
}
