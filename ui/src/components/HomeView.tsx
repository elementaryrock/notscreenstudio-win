import { useEffect, useState } from "react";
import {
  createDemoProject,
  deleteProject,
  getProjectThumbnail,
  listProjects,
  renameProject,
  revealPath,
} from "../ipc";
import { formatCfDate, formatDurationShort } from "../editor/editorLogic";
import { strings } from "../strings";
import type { CoreInfo, ProjectReference, ProjectSummary, ShellSnapshot } from "../types";

interface HomeViewProps {
  core: CoreInfo | null;
  shell: ShellSnapshot | null;
  onOpenProject: (ref: ProjectReference) => void;
  onOpenSetup: () => void;
  onOpenSettings: () => void;
  onStopRecording: () => void;
  onPauseToggle: () => void;
  onDismissError: () => void;
}

export function HomeView({
  core,
  shell,
  onOpenProject,
  onOpenSetup,
  onOpenSettings,
  onStopRecording,
  onPauseToggle,
  onDismissError,
}: HomeViewProps): React.JSX.Element {
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [menuId, setMenuId] = useState<string | null>(null);

  const refresh = async (): Promise<void> => {
    setLoading(true);
    try {
      const list = await listProjects();
      setProjects(list);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void refresh();
  }, [shell?.phase.kind]);

  const handleDemo = async (): Promise<void> => {
    try {
      const ref = await createDemoProject();
      await refresh();
      onOpenProject(ref);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const commitRename = async (summary: ProjectSummary): Promise<void> => {
    const trimmed = renameDraft.trim();
    setRenamingId(null);
    if (!trimmed || trimmed === summary.name) return;
    try {
      await renameProject(summary, trimmed);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const handleDelete = async (summary: ProjectSummary): Promise<void> => {
    setMenuId(null);
    if (!window.confirm(`${strings.confirmDelete}\n\n"${summary.name}"`)) return;
    try {
      await deleteProject(summary);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const isRecording = shell?.phase.kind === "recording";

  return (
    <div className="home-shell" onClick={() => setMenuId(null)}>
      <header className="home-header">
        <div className="home-brand">
          <span className="brand-dot" />
          <span className="brand-name">{strings.appName}</span>
          {core && (
            <span className="brand-meta">
              v{core.version} {core.gpuAvailable ? "• wgpu" : "• CPU"}
            </span>
          )}
        </div>
        <div className="home-actions">
          {core && (
            <button
              className="btn-secondary"
              type="button"
              onClick={() => void revealPath(core.libraryDir)}
              title={core.libraryDir}
            >
              {strings.openLibraryFolder}
            </button>
          )}
          <button className="btn-secondary" type="button" onClick={onOpenSettings}>
            {strings.settingsAndPermissions}
          </button>
          <button className="btn-secondary" type="button" onClick={() => void handleDemo()}>
            {strings.sampleProject}
          </button>
          <button
            className="btn-record"
            type="button"
            disabled={shell?.phase.kind === "preparing" || shell?.phase.kind === "saving"}
            onClick={isRecording ? onStopRecording : onOpenSetup}
          >
            <span className={`rec-glyph ${isRecording ? "stop" : ""}`} />
            {isRecording ? strings.stop : strings.newRecording}
          </button>
        </div>
      </header>

      {isRecording && shell && (
        <div className="live-banner">
          <span className={`pulse-dot ${shell.paused ? "paused" : ""}`} />
          <strong>
            {shell.paused ? strings.paused : strings.recording} •{" "}
            {formatDurationShort(shell.elapsedSeconds)}
          </strong>
          <div className="live-banner-actions">
            <button className="btn-secondary" type="button" onClick={onPauseToggle}>
              {shell.paused ? strings.resume : strings.pause}
            </button>
            <button className="btn-danger" type="button" onClick={onStopRecording}>
              {strings.stop}
            </button>
          </div>
        </div>
      )}

      {shell?.phase.kind === "failed" && (
        <div className="error-banner">
          <span>{shell.phase.message}</span>
          <button className="btn-secondary" type="button" onClick={onDismissError}>
            {strings.close}
          </button>
        </div>
      )}

      {error && (
        <div className="error-banner">
          <span>{error}</span>
          <button className="btn-secondary" type="button" onClick={() => setError(null)}>
            {strings.close}
          </button>
        </div>
      )}

      <main className="home-content">
        <div className="section-title-row">
          <h1>{strings.recordings}</h1>
          <span className="muted-count">
            {projects.length} {projects.length === 1 ? "project" : "projects"}
          </span>
        </div>

        {loading && projects.length === 0 ? (
          <div className="empty-state">
            <p className="muted">Loading library...</p>
          </div>
        ) : projects.length === 0 ? (
          <div className="empty-state">
            <div className="empty-icon">●</div>
            <h2>{strings.emptyLibraryTitle}</h2>
            <p>{strings.emptyLibraryBody}</p>
            <div className="empty-actions">
              <button className="btn-record" type="button" onClick={onOpenSetup}>
                <span className="rec-glyph" />
                {strings.newRecording}
              </button>
              <button className="btn-secondary" type="button" onClick={() => void handleDemo()}>
                {strings.sampleProject}
              </button>
            </div>
          </div>
        ) : (
          <div className="project-grid">
            {projects.map((p) => (
              <article
                key={p.id}
                className="project-card"
                onClick={() => {
                  if (renamingId !== p.id) onOpenProject(p.bundle);
                }}
              >
                <ProjectThumb summary={p} />
                <div className="project-card-body">
                  {renamingId === p.id ? (
                    <input
                      className="inline-rename"
                      autoFocus
                      value={renameDraft}
                      onClick={(e) => e.stopPropagation()}
                      onChange={(e) => setRenameDraft(e.target.value)}
                      onBlur={() => void commitRename(p)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") void commitRename(p);
                        if (e.key === "Escape") setRenamingId(null);
                      }}
                    />
                  ) : (
                    <div className="project-card-title" title={p.name}>
                      {p.name}
                    </div>
                  )}
                  <div className="project-card-meta">
                    <span>{formatCfDate(p.createdAt)}</span>
                    <button
                      type="button"
                      className="icon-btn-sm"
                      aria-label="Project options"
                      onClick={(e) => {
                        e.stopPropagation();
                        setMenuId(menuId === p.id ? null : p.id);
                      }}
                    >
                      ⋯
                    </button>
                  </div>
                </div>

                {menuId === p.id && (
                  <div className="card-menu" onClick={(e) => e.stopPropagation()}>
                    <button
                      type="button"
                      onClick={() => {
                        setMenuId(null);
                        setRenameDraft(p.name);
                        setRenamingId(p.id);
                      }}
                    >
                      {strings.rename}
                    </button>
                    <button
                      type="button"
                      onClick={() => {
                        setMenuId(null);
                        void revealPath(p.bundle.url);
                      }}
                    >
                      {strings.showInExplorer}
                    </button>
                    <button
                      type="button"
                      className="danger"
                      onClick={() => void handleDelete(p)}
                    >
                      {strings.deleteToRecycleBin}
                    </button>
                  </div>
                )}
              </article>
            ))}
          </div>
        )}
      </main>
    </div>
  );
}

function ProjectThumb({ summary }: { summary: ProjectSummary }): React.JSX.Element {
  const [url, setUrl] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    let objectUrl: string | null = null;
    if (!summary.thumbnailURL) return;
    void (async () => {
      try {
        const bytes = await getProjectThumbnail(summary.thumbnailURL!);
        if (!active || bytes.byteLength === 0) return;
        objectUrl = URL.createObjectURL(new Blob([bytes], { type: "image/jpeg" }));
        setUrl(objectUrl);
      } catch {
        // Fallback placeholder shown below.
      }
    })();
    return () => {
      active = false;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [summary.thumbnailURL]);

  return (
    <div className="project-thumb">
      {url ? (
        <img src={url} alt={summary.name} />
      ) : (
        <div className="project-thumb-placeholder">
          <span>NSS</span>
        </div>
      )}
      <span className="duration-pill">{formatDurationShort(summary.duration)}</span>
    </div>
  );
}
