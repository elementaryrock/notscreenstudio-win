import { useEffect, useState } from "react";
import {
  cancelExport,
  checkFfmpeg,
  onExportProgress,
  revealPath,
  startExport,
} from "../ipc";
import { strings } from "../strings";
import {
  outputPixelSize,
  resolutionLabel,
  type ExportPreset,
  type ExportProgress,
  type ExportResolution,
  type FfmpegStatusDto,
  type Project,
  type ProjectReference,
  type VideoCodec,
} from "../types";

interface ExportModalProps {
  reference: ProjectReference;
  project: Project;
  defaultOutputDir: string | null;
  onClose: () => void;
}

export function ExportModal({
  reference,
  project,
  defaultOutputDir,
  onClose,
}: ExportModalProps): React.JSX.Element {
  const [resolution, setResolution] = useState<ExportResolution>("p1080");
  const [fps, setFps] = useState<number>(60);
  const [codec, setCodec] = useState<VideoCodec>("h264");
  const [outputPath, setOutputPath] = useState<string>(() => {
    const safeName = project.name.replace(/[<>:"/\\|?*]/g, "-") || "export";
    if (!defaultOutputDir) return "";
    const cleanDir = defaultOutputDir.replace(/[\\/]+$/, "");
    return `${cleanDir}\\${safeName}.mp4`;
  });
  const [ffmpeg, setFfmpeg] = useState<FfmpegStatusDto | null>(null);
  const [progress, setProgress] = useState<ExportProgress>({ idle: {} });
  const [finishedPath, setFinishedPath] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void checkFfmpeg()
      .then((ff) => {
        if (disposed) return;
        setFfmpeg(ff);
        if (!ff.h264 && ff.hevc) setCodec("hevc");
      })
      .catch(() => {});

    void onExportProgress((p) => {
      if (disposed) return;
      setProgress(p);
      if ("finished" in p) {
        setFinishedPath(p.finished);
      }
    })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => {});

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const size = outputPixelSize(resolution, project.style.aspectRatio);
  const isRunning = "running" in progress;

  const handleStart = async (): Promise<void> => {
    setFinishedPath(null);
    setProgress({ running: { fraction: 0, framesWritten: 0, totalFrames: 1 } });
    const preset: ExportPreset = { resolution, fps, codec };
    try {
      const target = await startExport(
        reference,
        project,
        preset,
        outputPath.trim() ? outputPath.trim() : undefined,
      );
      if (!outputPath.trim()) setOutputPath(target);
    } catch (e) {
      setProgress({
        failed: { message: e instanceof Error ? e.message : String(e) },
      });
    }
  };

  return (
    <div className="modal-backdrop" onClick={isRunning ? undefined : onClose}>
      <div className="modal-card" onClick={(e) => e.stopPropagation()}>
        <header className="modal-header">
          <h2>{strings.exportTitle}</h2>
          {!isRunning && (
            <button className="icon-btn" type="button" onClick={onClose}>
              ✕
            </button>
          )}
        </header>

        {ffmpeg && !ffmpeg.available && (
          <div className="inline-error">
            <strong>{strings.ffmpegMissingTitle}:</strong> {ffmpeg.message}
          </div>
        )}

        <div className="form-row">
          <label>{strings.resolution}</label>
          <div className="pill-group">
            {(["p1080", "p2160"] as ExportResolution[]).map((res) => {
              const s = outputPixelSize(res, project.style.aspectRatio);
              return (
                <button
                  key={res}
                  type="button"
                  disabled={isRunning}
                  className={resolution === res ? "active" : ""}
                  onClick={() => setResolution(res)}
                >
                  {resolutionLabel(res)} ({s.width}×{s.height})
                </button>
              );
            })}
          </div>
        </div>

        <div className="form-row">
          <label>{strings.frameRate}</label>
          <div className="pill-group">
            {[30, 60].map((rate) => (
              <button
                key={rate}
                type="button"
                disabled={isRunning}
                className={fps === rate ? "active" : ""}
                onClick={() => setFps(rate)}
              >
                {rate} fps
              </button>
            ))}
          </div>
        </div>

        <div className="form-row">
          <label>{strings.codec}</label>
          <div className="pill-group">
            {(["h264", "hevc"] as VideoCodec[]).map((c) => (
              <button
                key={c}
                type="button"
                disabled={
                  isRunning ||
                  (c === "h264" && ffmpeg !== null && !ffmpeg.h264) ||
                  (c === "hevc" && ffmpeg !== null && !ffmpeg.hevc)
                }
                className={codec === c ? "active" : ""}
                onClick={() => setCodec(c)}
              >
                {c === "h264" ? "H.264 (MP4)" : "HEVC (H.265)"}
              </button>
            ))}
          </div>
        </div>

        <div className="form-row">
          <label>{strings.destination}</label>
          <input
            type="text"
            disabled={isRunning}
            placeholder="(Default: library folder)"
            value={outputPath}
            onChange={(e) => setOutputPath(e.target.value)}
          />
        </div>

        <div className="export-summary">
          Output: {size.width}×{size.height} • {fps} fps • {codec.toUpperCase()}
          {project.assets.micFile ? " • AAC 48 kHz" : ""}
        </div>

        {"running" in progress && (
          <div className="progress-wrap">
            <div className="progress-bar">
              <div
                className="progress-fill"
                style={{ width: `${Math.round(progress.running.fraction * 100)}%` }}
              />
            </div>
            <div className="progress-meta">
              <span>{strings.exporting}</span>
              <span className="mono">
                {Math.round(progress.running.fraction * 100)}% ({progress.running.framesWritten} /{" "}
                {progress.running.totalFrames} frames)
              </span>
            </div>
          </div>
        )}

        {"finished" in progress && (
          <div className="success-banner">
            <span>{strings.exportComplete}</span>
            {finishedPath && (
              <button
                className="btn-secondary"
                type="button"
                onClick={() => void revealPath(finishedPath).catch(() => {})}
              >
                {strings.showInExplorer}
              </button>
            )}
          </div>
        )}

        {"cancelled" in progress && <div className="muted-sm">{strings.exportCancelled}</div>}

        {"failed" in progress && <div className="inline-error">{progress.failed.message}</div>}

        <footer className="modal-footer">
          <span />
          <div className="footer-right">
            {isRunning ? (
              <button
                className="btn-danger"
                type="button"
                onClick={() => void cancelExport().catch(() => {})}
              >
                {strings.cancel}
              </button>
            ) : (
              <>
                <button className="btn-secondary" type="button" onClick={onClose}>
                  {strings.close}
                </button>
                <button
                  className="btn-primary"
                  type="button"
                  disabled={
                    ffmpeg !== null &&
                    (!ffmpeg.available || (codec === "h264" ? !ffmpeg.h264 : !ffmpeg.hevc))
                  }
                  onClick={() => void handleStart()}
                >
                  {strings.startExport}
                </button>
              </>
            )}
          </div>
        </footer>
      </div>
    </div>
  );
}
