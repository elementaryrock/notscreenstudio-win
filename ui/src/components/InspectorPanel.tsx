import { useEffect, useState } from "react";
import {
  listBackgroundPresets,
  listSystemWallpapers,
  planAutoZooms,
  renderBackgroundPreview,
  saveSettings,
} from "../ipc";
import {
  addZoomBlock,
  deleteZoomBlock,
  removeAutoZooms,
  setZoomEase,
  setZoomFocus,
  setZoomScale,
} from "../editor/editorLogic";
import { strings } from "../strings";
import {
  hexToRgba,
  rgbaToHex,
  type Background,
  type BackgroundPresetsDto,
  type CameraShape,
  type Project,
  type ShellSettings,
  type WallpaperEntry,
} from "../types";

interface InspectorPanelProps {
  project: Project;
  settings: ShellSettings;
  selectedZoomId: string | null;
  currentTime: number;
  duration: number;
  onSettingsChange: (s: ShellSettings) => void;
  onCommitProject: (next: Project) => void;
  onBeginDrag: () => void;
  onDragProject: (next: Project) => void;
  onEndDrag: () => void;
  onSelectZoom: (id: string | null) => void;
}

type BgTab = "wallpapers" | "gradients" | "colors" | "image";

export function InspectorPanel({
  project,
  settings,
  selectedZoomId,
  currentTime,
  duration,
  onSettingsChange,
  onCommitProject,
  onBeginDrag,
  onDragProject,
  onEndDrag,
  onSelectZoom,
}: InspectorPanelProps): React.JSX.Element {
  const [presets, setPresets] = useState<BackgroundPresetsDto | null>(null);
  const [wallpapers, setWallpapers] = useState<WallpaperEntry[]>([]);
  const [swatches, setSwatches] = useState<Record<string, string>>({});
  const [bgTab, setBgTab] = useState<BgTab>("gradients");
  const [imagePathInput, setImagePathInput] = useState("");
  const [zoomNotice, setZoomNotice] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void (async () => {
      const [p, wp] = await Promise.all([listBackgroundPresets(), listSystemWallpapers()]);
      if (!active) return;
      setPresets(p);
      setWallpapers(wp);

      // Render CPU swatches for gradient and mesh presets in parallel.
      const entries = [...p.gradients, ...p.meshes];
      const rendered = await Promise.all(
        entries.map(async (entry) => {
          const pkt = await renderBackgroundPreview(entry.background, 64, 40);
          if (!pkt) return [entry.id, ""] as const;
          const canvas = document.createElement("canvas");
          canvas.width = pkt.width;
          canvas.height = pkt.height;
          const ctx = canvas.getContext("2d");
          if (!ctx) return [entry.id, ""] as const;
          ctx.putImageData(new ImageData(pkt.pixels, pkt.width, pkt.height), 0, 0);
          return [entry.id, canvas.toDataURL("image/png")] as const;
        }),
      );
      if (!active) return;
      const map: Record<string, string> = {};
      for (const [id, url] of rendered) {
        if (url) map[id] = url;
      }
      setSwatches(map);
    })();
    return () => {
      active = false;
    };
  }, []);

  const isSectionOpen = (key: string, defaultOpen = false): boolean => {
    const stored = settings.inspectorSections[key];
    return stored !== undefined ? stored : defaultOpen;
  };

  const toggleSection = (key: string, defaultOpen = false): void => {
    const nextOpen = !isSectionOpen(key, defaultOpen);
    const nextSettings: ShellSettings = {
      ...settings,
      inspectorSections: { ...settings.inspectorSections, [key]: nextOpen },
    };
    onSettingsChange(nextSettings);
    void saveSettings(nextSettings);
  };

  const updateStyle = (patch: Partial<Project["style"]>, drag = false): void => {
    const next: Project = { ...project, style: { ...project.style, ...patch } };
    if (drag) onDragProject(next);
    else onCommitProject(next);
  };

  const applyImageBackground = (rawPath: string): void => {
    const trimmed = rawPath.trim();
    if (!trimmed) return;
    updateStyle({ background: { image: { path: trimmed } } });
    const recent = [
      trimmed,
      ...settings.recentBackgroundImages.filter((p) => p !== trimmed),
    ].slice(0, 8);
    const nextSettings: ShellSettings = { ...settings, recentBackgroundImages: recent };
    onSettingsChange(nextSettings);
    void saveSettings(nextSettings);
  };

  const selectedZoom =
    project.timeline.zoomBlocks.find((b) => b.id === selectedZoomId) ?? null;

  const handleAddZoom = (): void => {
    setZoomNotice(null);
    const res = addZoomBlock(currentTime, project.timeline.zoomBlocks, duration);
    if (!res) {
      setZoomNotice("No free room at playhead (move playhead outside existing zoom blocks).");
      return;
    }
    onCommitProject({
      ...project,
      timeline: { ...project.timeline, zoomBlocks: res.blocks },
    });
    onSelectZoom(res.id);
  };

  const handleRegenAutoZoom = async (): Promise<void> => {
    setZoomNotice(null);
    try {
      const nextBlocks = await planAutoZooms(project.timeline.zoomBlocks);
      onCommitProject({
        ...project,
        timeline: { ...project.timeline, zoomBlocks: nextBlocks, autoZoomEnabled: true },
      });
    } catch (e) {
      setZoomNotice(e instanceof Error ? e.message : String(e));
    }
  };

  const handleClearAutoZoom = (): void => {
    setZoomNotice(null);
    const nextBlocks = removeAutoZooms(project.timeline.zoomBlocks);
    onCommitProject({
      ...project,
      timeline: { ...project.timeline, zoomBlocks: nextBlocks },
    });
    if (selectedZoom?.isAuto) onSelectZoom(null);
  };

  const style = project.style;
  const activeSolidHex =
    "solid" in style.background ? rgbaToHex(style.background.solid._0) : "#1a1c24";
  const activeGradient = "gradient" in style.background ? style.background.gradient : null;

  return (
    <aside className="inspector-panel">
      {/* 1. Background */}
      <InspectorSection
        title={strings.sectionBackground}
        open={isSectionOpen("background", true)}
        onToggle={() => toggleSection("background", true)}
      >
        <div className="segmented-tabs compact">
          {(["wallpapers", "gradients", "colors", "image"] as BgTab[]).map((t) => (
            <button
              key={t}
              type="button"
              className={bgTab === t ? "active" : ""}
              onClick={() => setBgTab(t)}
            >
              {t === "wallpapers"
                ? strings.bgWallpapers
                : t === "gradients"
                  ? strings.bgGradients
                  : t === "colors"
                    ? strings.bgColors
                    : strings.bgImage}
            </button>
          ))}
        </div>

        {bgTab === "wallpapers" && (
          <div className="wallpaper-list">
            {wallpapers.length === 0 ? (
              <p className="muted-sm">No Windows wallpapers found in C:\Windows\Web.</p>
            ) : (
              wallpapers.map((wp) => {
                const active =
                  "systemWallpaper" in style.background &&
                  style.background.systemWallpaper.path === wp.path;
                return (
                  <button
                    key={wp.path}
                    type="button"
                    className={`wallpaper-chip ${active ? "selected" : ""}`}
                    onClick={() => updateStyle({ background: { systemWallpaper: { path: wp.path } } })}
                    title={wp.path}
                  >
                    {wp.name}
                  </button>
                );
              })
            )}
          </div>
        )}

        {bgTab === "gradients" && presets && (
          <>
            <div className="swatch-grid">
              {[...presets.gradients, ...presets.meshes].map((entry) => {
                const selected = backgroundsEqual(style.background, entry.background);
                return (
                  <button
                    key={entry.id}
                    type="button"
                    className={`swatch-btn ${selected ? "selected" : ""}`}
                    title={entry.name}
                    onClick={() => updateStyle({ background: entry.background })}
                  >
                    {swatches[entry.id] ? (
                      <img src={swatches[entry.id]} alt={entry.name} />
                    ) : (
                      <span className="swatch-fallback">{entry.name}</span>
                    )}
                  </button>
                );
              })}
            </div>
            {activeGradient && (
              <SliderRow
                label={strings.bgAngle}
                value={activeGradient.angleDegrees}
                min={0}
                max={360}
                step={1}
                display={`${Math.round(activeGradient.angleDegrees)}°`}
                onBegin={onBeginDrag}
                onChange={(v) =>
                  updateStyle(
                    {
                      background: {
                        gradient: { stops: activeGradient.stops, angleDegrees: v },
                      },
                    },
                    true,
                  )
                }
                onEnd={onEndDrag}
              />
            )}
          </>
        )}

        {bgTab === "colors" && presets && (
          <>
            <div className="swatch-grid solids">
              {presets.solids.map((entry) => {
                const hex =
                  "solid" in entry.background ? rgbaToHex(entry.background.solid._0) : "#222";
                const selected = backgroundsEqual(style.background, entry.background);
                return (
                  <button
                    key={entry.id}
                    type="button"
                    className={`swatch-btn solid ${selected ? "selected" : ""}`}
                    style={{ backgroundColor: hex }}
                    title={entry.name}
                    onClick={() => updateStyle({ background: entry.background })}
                  />
                );
              })}
            </div>
            <label className="inspector-row">
              <span>{strings.bgCustomColor}</span>
              <input
                type="color"
                value={activeSolidHex}
                onChange={(e) =>
                  updateStyle({ background: { solid: { _0: hexToRgba(e.target.value, 1) } } })
                }
              />
            </label>
          </>
        )}

        {bgTab === "image" && (
          <div className="image-picker-box">
            <div className="inline-input-row">
              <input
                type="text"
                placeholder={strings.bgImagePathPlaceholder}
                value={imagePathInput}
                onChange={(e) => setImagePathInput(e.target.value)}
              />
              <button
                type="button"
                className="btn-secondary compact"
                onClick={() => applyImageBackground(imagePathInput)}
              >
                {strings.bgApplyImage}
              </button>
            </div>
            {settings.recentBackgroundImages.length > 0 && (
              <div className="recent-images">
                <div className="muted-sm">{strings.bgRecentImages}</div>
                {settings.recentBackgroundImages.map((p) => (
                  <button
                    key={p}
                    type="button"
                    className="wallpaper-chip"
                    onClick={() => applyImageBackground(p)}
                    title={p}
                  >
                    {p.split(/[\\/]/).pop() ?? p}
                  </button>
                ))}
              </div>
            )}
          </div>
        )}

        <SliderRow
          label={strings.bgBlur}
          value={style.backgroundBlur}
          min={0}
          max={40}
          step={1}
          display={`${Math.round(style.backgroundBlur)} px`}
          onBegin={onBeginDrag}
          onChange={(v) => updateStyle({ backgroundBlur: v }, true)}
          onEnd={onEndDrag}
        />
      </InspectorSection>

      {/* 2. Frame */}
      <InspectorSection
        title={strings.sectionFrame}
        open={isSectionOpen("frame", true)}
        onToggle={() => toggleSection("frame", true)}
      >
        <SliderRow
          label={strings.padding}
          value={style.padding}
          min={0}
          max={160}
          step={1}
          display={`${Math.round(style.padding)} px`}
          onBegin={onBeginDrag}
          onChange={(v) => updateStyle({ padding: v }, true)}
          onEnd={onEndDrag}
        />
        <SliderRow
          label={strings.cornerRadius}
          value={style.cornerRadius}
          min={0}
          max={48}
          step={1}
          display={`${Math.round(style.cornerRadius)} px`}
          onBegin={onBeginDrag}
          onChange={(v) => updateStyle({ cornerRadius: v }, true)}
          onEnd={onEndDrag}
        />
        <SliderRow
          label={strings.shadowOpacity}
          value={style.shadow.color.a}
          min={0}
          max={1}
          step={0.02}
          display={`${Math.round(style.shadow.color.a * 100)}%`}
          onBegin={onBeginDrag}
          onChange={(v) =>
            updateStyle(
              { shadow: { ...style.shadow, color: { ...style.shadow.color, a: v } } },
              true,
            )
          }
          onEnd={onEndDrag}
        />
        <SliderRow
          label={strings.shadowBlur}
          value={style.shadow.radius}
          min={0}
          max={80}
          step={1}
          display={`${Math.round(style.shadow.radius)} px`}
          onBegin={onBeginDrag}
          onChange={(v) => updateStyle({ shadow: { ...style.shadow, radius: v } }, true)}
          onEnd={onEndDrag}
        />
        <SliderRow
          label={strings.shadowOffsetY}
          value={style.shadow.offsetY}
          min={-20}
          max={40}
          step={1}
          display={`${Math.round(style.shadow.offsetY)} px`}
          onBegin={onBeginDrag}
          onChange={(v) => updateStyle({ shadow: { ...style.shadow, offsetY: v } }, true)}
          onEnd={onEndDrag}
        />
      </InspectorSection>

      {/* 3. Cursor */}
      <InspectorSection
        title={strings.sectionCursor}
        open={isSectionOpen("cursor", false)}
        onToggle={() => toggleSection("cursor", false)}
      >
        <ToggleRow
          label={strings.showCursor}
          checked={style.cursor.visible}
          onChange={(v) => updateStyle({ cursor: { ...style.cursor, visible: v } })}
        />
        <SliderRow
          label={strings.cursorSize}
          value={style.cursor.sizeMultiplier}
          min={0.5}
          max={3.0}
          step={0.05}
          display={`${style.cursor.sizeMultiplier.toFixed(2)}×`}
          onBegin={onBeginDrag}
          onChange={(v) => updateStyle({ cursor: { ...style.cursor, sizeMultiplier: v } }, true)}
          onEnd={onEndDrag}
        />
        <ToggleRow
          label={strings.cursorSmoothing}
          checked={style.cursor.smoothing}
          onChange={(v) => updateStyle({ cursor: { ...style.cursor, smoothing: v } })}
        />
        <SliderRow
          label={strings.motionBlur}
          value={style.cursor.motionBlur}
          min={0}
          max={1}
          step={0.05}
          display={`${Math.round(style.cursor.motionBlur * 100)}%`}
          onBegin={onBeginDrag}
          onChange={(v) => updateStyle({ cursor: { ...style.cursor, motionBlur: v } }, true)}
          onEnd={onEndDrag}
        />
        <ToggleRow
          label={strings.clickRipples}
          checked={style.cursor.clickRipples}
          onChange={(v) => updateStyle({ cursor: { ...style.cursor, clickRipples: v } })}
        />
        <ToggleRow
          label={strings.hideWhenIdle}
          checked={style.cursor.hideWhenIdle}
          onChange={(v) => updateStyle({ cursor: { ...style.cursor, hideWhenIdle: v } })}
        />
        {style.cursor.hideWhenIdle && (
          <SliderRow
            label={strings.idleDelay}
            value={style.cursor.idleDelay}
            min={0.5}
            max={10}
            step={0.5}
            display={`${style.cursor.idleDelay.toFixed(1)} s`}
            onBegin={onBeginDrag}
            onChange={(v) => updateStyle({ cursor: { ...style.cursor, idleDelay: v } }, true)}
            onEnd={onEndDrag}
          />
        )}
      </InspectorSection>

      {/* 4. Zoom */}
      <InspectorSection
        title={strings.sectionZoom}
        open={isSectionOpen("zoom", true)}
        onToggle={() => toggleSection("zoom", true)}
      >
        <div className="zoom-actions">
          <button type="button" className="btn-secondary compact" onClick={handleAddZoom}>
            + {strings.addZoom}
          </button>
          <button
            type="button"
            className="btn-secondary compact"
            onClick={() => void handleRegenAutoZoom()}
          >
            ✦ {strings.regenAutoZoom}
          </button>
          <button type="button" className="btn-secondary compact" onClick={handleClearAutoZoom}>
            {strings.clearAutoZoom}
          </button>
        </div>

        {zoomNotice && <div className="muted-sm">{zoomNotice}</div>}

        {selectedZoom ? (
          <div className="zoom-editor-card">
            <div className="zoom-card-head">
              <span className="mono">
                {selectedZoom.start.toFixed(2)}s – {selectedZoom.end.toFixed(2)}s
              </span>
              <span className="status-chip">{selectedZoom.isAuto ? "✦ Auto" : "Manual"}</span>
            </div>

            <SliderRow
              label={strings.zoomLevel}
              value={selectedZoom.scale}
              min={1.1}
              max={4.0}
              step={0.05}
              display={`${selectedZoom.scale.toFixed(2)}×`}
              onBegin={onBeginDrag}
              onChange={(v) =>
                onDragProject({
                  ...project,
                  timeline: {
                    ...project.timeline,
                    zoomBlocks: setZoomScale(selectedZoom.id, v, project.timeline.zoomBlocks),
                  },
                })
              }
              onEnd={onEndDrag}
            />

            <div className="inspector-row">
              <span>{strings.zoomFocus}</span>
              <div className="pill-group compact">
                <button
                  type="button"
                  className={"auto" in selectedZoom.focus ? "active" : ""}
                  onClick={() =>
                    onCommitProject({
                      ...project,
                      timeline: {
                        ...project.timeline,
                        zoomBlocks: setZoomFocus(
                          selectedZoom.id,
                          { auto: {} },
                          project.timeline.zoomBlocks,
                        ),
                      },
                    })
                  }
                >
                  {strings.focusFollow}
                </button>
                <button
                  type="button"
                  className={"manual" in selectedZoom.focus ? "active" : ""}
                  onClick={() =>
                    onCommitProject({
                      ...project,
                      timeline: {
                        ...project.timeline,
                        zoomBlocks: setZoomFocus(
                          selectedZoom.id,
                          { manual: { _0: { x: 0.5, y: 0.5 } } },
                          project.timeline.zoomBlocks,
                        ),
                      },
                    })
                  }
                >
                  {strings.focusFixed}
                </button>
              </div>
            </div>

            {"manual" in selectedZoom.focus && (
              <>
                <div className="muted-sm">{strings.clickPreviewToPick}</div>
                <SliderRow
                  label="Focus X"
                  value={selectedZoom.focus.manual._0.x}
                  min={0}
                  max={1}
                  step={0.01}
                  display={`${Math.round(selectedZoom.focus.manual._0.x * 100)}%`}
                  onBegin={onBeginDrag}
                  onChange={(v) => {
                    const pt = "manual" in selectedZoom.focus ? selectedZoom.focus.manual._0 : { x: 0.5, y: 0.5 };
                    onDragProject({
                      ...project,
                      timeline: {
                        ...project.timeline,
                        zoomBlocks: setZoomFocus(
                          selectedZoom.id,
                          { manual: { _0: { x: v, y: pt.y } } },
                          project.timeline.zoomBlocks,
                        ),
                      },
                    });
                  }}
                  onEnd={onEndDrag}
                />
                <SliderRow
                  label="Focus Y"
                  value={selectedZoom.focus.manual._0.y}
                  min={0}
                  max={1}
                  step={0.01}
                  display={`${Math.round(selectedZoom.focus.manual._0.y * 100)}%`}
                  onBegin={onBeginDrag}
                  onChange={(v) => {
                    const pt = "manual" in selectedZoom.focus ? selectedZoom.focus.manual._0 : { x: 0.5, y: 0.5 };
                    onDragProject({
                      ...project,
                      timeline: {
                        ...project.timeline,
                        zoomBlocks: setZoomFocus(
                          selectedZoom.id,
                          { manual: { _0: { x: pt.x, y: v } } },
                          project.timeline.zoomBlocks,
                        ),
                      },
                    });
                  }}
                  onEnd={onEndDrag}
                />
              </>
            )}

            <SliderRow
              label={strings.easeIn}
              value={selectedZoom.easeIn}
              min={0.1}
              max={2.0}
              step={0.05}
              display={`${selectedZoom.easeIn.toFixed(2)} s`}
              onBegin={onBeginDrag}
              onChange={(v) =>
                onDragProject({
                  ...project,
                  timeline: {
                    ...project.timeline,
                    zoomBlocks: setZoomEase(selectedZoom.id, v, null, project.timeline.zoomBlocks),
                  },
                })
              }
              onEnd={onEndDrag}
            />
            <SliderRow
              label={strings.easeOut}
              value={selectedZoom.easeOut}
              min={0.1}
              max={2.0}
              step={0.05}
              display={`${selectedZoom.easeOut.toFixed(2)} s`}
              onBegin={onBeginDrag}
              onChange={(v) =>
                onDragProject({
                  ...project,
                  timeline: {
                    ...project.timeline,
                    zoomBlocks: setZoomEase(selectedZoom.id, null, v, project.timeline.zoomBlocks),
                  },
                })
              }
              onEnd={onEndDrag}
            />

            <button
              type="button"
              className="btn-danger compact full-width"
              onClick={() => {
                onCommitProject({
                  ...project,
                  timeline: {
                    ...project.timeline,
                    zoomBlocks: deleteZoomBlock(selectedZoom.id, project.timeline.zoomBlocks),
                  },
                });
                onSelectZoom(null);
              }}
            >
              {strings.deleteZoom}
            </button>
          </div>
        ) : (
          <p className="muted-sm">{strings.noZoomSelected}</p>
        )}
      </InspectorSection>

      {/* 5. Camera */}
      <InspectorSection
        title={strings.sectionCamera}
        open={isSectionOpen("camera", false)}
        onToggle={() => toggleSection("camera", false)}
      >
        <ToggleRow
          label={strings.showCamera}
          checked={style.camera.visible}
          onChange={(v) => updateStyle({ camera: { ...style.camera, visible: v } })}
        />

        <div className="inspector-row">
          <span>{strings.cameraShape}</span>
          <div className="pill-group compact">
            {(["circle", "square", "tall", "wide"] as CameraShape[]).map((sh) => (
              <button
                key={sh}
                type="button"
                className={style.camera.shape === sh ? "active" : ""}
                onClick={() => updateStyle({ camera: { ...style.camera, shape: sh } })}
              >
                {sh}
              </button>
            ))}
          </div>
        </div>

        <SliderRow
          label={strings.cameraRoundness}
          value={style.camera.roundness}
          min={0}
          max={1}
          step={0.05}
          display={`${Math.round(style.camera.roundness * 100)}%`}
          onBegin={onBeginDrag}
          onChange={(v) => updateStyle({ camera: { ...style.camera, roundness: v } }, true)}
          onEnd={onEndDrag}
        />
        <SliderRow
          label={strings.cameraSize}
          value={style.camera.size}
          min={0.08}
          max={0.45}
          step={0.01}
          display={`${Math.round(style.camera.size * 100)}%`}
          onBegin={onBeginDrag}
          onChange={(v) => updateStyle({ camera: { ...style.camera, size: v } }, true)}
          onEnd={onEndDrag}
        />
        <SliderRow
          label={strings.cameraBorder}
          value={style.camera.border?.width ?? 0}
          min={0}
          max={12}
          step={0.5}
          display={`${(style.camera.border?.width ?? 0).toFixed(1)} px`}
          onBegin={onBeginDrag}
          onChange={(v) =>
            updateStyle(
              {
                camera: {
                  ...style.camera,
                  border:
                    v > 0
                      ? {
                          width: v,
                          color: style.camera.border?.color ?? { r: 1, g: 1, b: 1, a: 0.9 },
                        }
                      : null,
                },
              },
              true,
            )
          }
          onEnd={onEndDrag}
        />
        {style.camera.border && style.camera.border.width > 0 && (
          <label className="inspector-row">
            <span>Border color</span>
            <input
              type="color"
              value={rgbaToHex(style.camera.border.color)}
              onChange={(e) =>
                updateStyle({
                  camera: {
                    ...style.camera,
                    border: {
                      width: style.camera.border?.width ?? 2,
                      color: hexToRgba(e.target.value, style.camera.border?.color.a ?? 1),
                    },
                  },
                })
              }
            />
          </label>
        )}
        <ToggleRow
          label={strings.cameraMirror}
          checked={style.camera.mirrored}
          onChange={(v) => updateStyle({ camera: { ...style.camera, mirrored: v } })}
        />
        <ToggleRow
          label={strings.cameraShrinkOnZoom}
          checked={style.camera.shrinkOnZoom}
          onChange={(v) => updateStyle({ camera: { ...style.camera, shrinkOnZoom: v } })}
        />
        <SliderRow
          label={strings.cameraPositionX}
          value={style.camera.position.x}
          min={0.05}
          max={0.95}
          step={0.01}
          display={`${Math.round(style.camera.position.x * 100)}%`}
          onBegin={onBeginDrag}
          onChange={(v) =>
            updateStyle(
              { camera: { ...style.camera, position: { ...style.camera.position, x: v } } },
              true,
            )
          }
          onEnd={onEndDrag}
        />
        <SliderRow
          label={strings.cameraPositionY}
          value={style.camera.position.y}
          min={0.05}
          max={0.95}
          step={0.01}
          display={`${Math.round(style.camera.position.y * 100)}%`}
          onBegin={onBeginDrag}
          onChange={(v) =>
            updateStyle(
              { camera: { ...style.camera, position: { ...style.camera.position, y: v } } },
              true,
            )
          }
          onEnd={onEndDrag}
        />
      </InspectorSection>

      {/* 6. Audio */}
      <InspectorSection
        title={strings.sectionAudio}
        open={isSectionOpen("audio", false)}
        onToggle={() => toggleSection("audio", false)}
      >
        <div className="inspector-row">
          <span>{strings.audioMicTrack}</span>
          <span className="muted-sm">{project.assets.micFile ?? "None"}</span>
        </div>
        <p className="muted-sm">{strings.audioTrimmedNotice}</p>
      </InspectorSection>
    </aside>
  );
}

function InspectorSection({
  title,
  open,
  onToggle,
  children,
}: {
  title: string;
  open: boolean;
  onToggle: () => void;
  children: React.ReactNode;
}): React.JSX.Element {
  return (
    <div className={`inspector-section ${open ? "open" : ""}`}>
      <button type="button" className="inspector-section-head" onClick={onToggle}>
        <span>{title}</span>
        <span className="chevron">{open ? "▾" : "▸"}</span>
      </button>
      {open && <div className="inspector-section-body">{children}</div>}
    </div>
  );
}

function SliderRow({
  label,
  value,
  min,
  max,
  step,
  display,
  onBegin,
  onChange,
  onEnd,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  display: string;
  onBegin: () => void;
  onChange: (v: number) => void;
  onEnd: () => void;
}): React.JSX.Element {
  return (
    <div className="slider-row">
      <div className="slider-head">
        <span>{label}</span>
        <span className="mono muted-sm">{display}</span>
      </div>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onPointerDown={onBegin}
        onChange={(e) => onChange(parseFloat(e.target.value))}
        onPointerUp={onEnd}
        onKeyUp={onEnd}
      />
    </div>
  );
}

function ToggleRow({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}): React.JSX.Element {
  return (
    <label className="inspector-row toggle-row">
      <span>{label}</span>
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} />
    </label>
  );
}

function backgroundsEqual(a: Background, b: Background): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}
