import { useCallback, useEffect, useRef, useState } from "react";
import {
  cancelCountdown,
  coreInfo,
  dismissError,
  getSettings,
  getShellSnapshot,
  onAreaSelected,
  onElapsed,
  onHotkey,
  onRecordingSaved,
  onSettingsChanged,
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

  const shellRef = useRef<ShellSnapshot | null>(null);
  shellRef.current = shell;

  useEffect(() => {
    let active = true;
    void Promise.all([coreInfo(), getShellSnapshot(), getSettings()])
      .then(([info, snap, set]) => {
        if (!active) return;
        setCore(info);
        setShell(snap);
        setSettings(set);
      })
      .catch(() => {
        // Initial load errors surface via subsequent calls or shell snapshot.
      });
    return () => {
      active = false;
    };
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
    const curr = shellRef.current;
    if (!curr || curr.phase.kind !== "recording") return;
    try {
      if (curr.paused) await resumeRecording();
      else await pauseRecording();
    } catch {
      // Error surfaced via shell-state event.
    }
  }, []);

  // Subscribe once to backend shell, tray, hotkey, elapsed, settings, and saved-project events.
  // Guard every async `listen` registration against React StrictMode unmount races.
  useEffect(() => {
    let disposed = false;
    const cleanups: Array<() => void> = [];
    const register = (promise: Promise<() => void>): void => {
      void promise
        .then((fn) => {
          if (disposed) fn();
          else cleanups.push(fn);
        })
        .catch(() => {
          // Ignore listener registration failure outside Tauri context.
        });
    };

    register(onShellState((s) => setShell(s)));
    register(
      onElapsed((secs) =>
        setShell((prev) => (prev ? { ...prev, elapsedSeconds: secs } : prev)),
      ),
    );
    register(
      onSettingsChanged((next) => {
        setSettings(next);
      }),
    );
    register(
      onRecordingSaved((ref) => {
        setShowSetup(false);
        setActiveProject(ref);
      }),
    );
    register(
      onAreaSelected((src) => {
        if (src) {
          setSettings((prev) => (prev ? { ...prev, source: src } : prev));
          setShowSetup(true);
        }
      }),
    );
    register(
      onTrayAction((action) => {
        if (action === "record") setShowSetup(true);
        else if (action === "settings") setShowSettings(true);
        else if (action === "stop") void handleStopRecording();
        else if (action === "pause") void handlePauseToggle();
        else if (action === "open") setActiveProject(null);
      }),
    );
    register(
      onHotkey((action) => {
        if (action === "toggle-record") {
          const phase = shellRef.current?.phase.kind;
          if (phase === "recording") {
            void handleStopRecording();
          } else if (phase === "preparing") {
            void cancelCountdown().catch(() => { });
          } else if (phase !== "saving") {
            setShowSetup(true);
          }
        } else if (action === "toggle-pause") {
          void handlePauseToggle();
        }
      }),
    );

    return () => {
      disposed = true;
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
          onDismissError={() => void dismissError().catch(() => { })}
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
