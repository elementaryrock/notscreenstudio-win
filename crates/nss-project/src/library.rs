//! The home screen's data source: summaries, thumbnails, rename, delete.
//!
//! Port of upstream `NSSProject/FileProjectLibrary.swift`.
//!
//! * Listing only reads each bundle's small `project.json`; it never touches media, so it stays
//!   fast with many recordings.
//! * Thumbnails (`thumbnail.jpg` in the bundle) are made on demand and cached on disk. Upstream
//!   uses `AVAssetImageGenerator`; on Windows the frame comes from the bundled FFmpeg (ADR 0002),
//!   behind the [`Thumbnailer`] trait so tests never spawn a process.
//! * Delete goes through the [`Trash`] trait, whose Windows implementation is the Recycle Bin.
//! * Rename changes the project name only; the bundle folder keeps its name so open references
//!   stay valid (and so a copied bundle's id does not move).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nss_core::{MediaTime, Project, ProjectReference, ProjectSummary};
use parking_lot::Mutex;
use uuid::Uuid;

use crate::errors::ProjectError;
use crate::migrator::ProjectMigrator;
use crate::store::FileProjectStore;

/// Longest edge of a generated thumbnail, in pixels.
pub const THUMBNAIL_MAX_EDGE: u32 = 640;

/// The bundle-relative thumbnail name when the project does not name one.
pub const THUMBNAIL_FILE_NAME: &str = "thumbnail.jpg";

/// Turns a video frame into a JPEG. Injectable so tests do not need FFmpeg.
pub trait Thumbnailer: Send + Sync + std::fmt::Debug {
    /// Writes a JPEG of the frame at `at` in `video` to `to`. Returns false when the video is
    /// missing or unreadable (upstream's `generateThumbnail` returns `Bool` too).
    fn generate(&self, video: &Path, at: MediaTime, to: &Path) -> Result<bool, ProjectError>;
}

/// A thumbnailer that never produces anything: used before FFmpeg is located, and by tests.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopThumbnailer;

impl Thumbnailer for NoopThumbnailer {
    fn generate(&self, _video: &Path, _at: MediaTime, _to: &Path) -> Result<bool, ProjectError> {
        Ok(false)
    }
}

/// Pulls one frame out of a video with FFmpeg and writes it as a JPEG.
#[derive(Debug, Clone)]
pub struct FfmpegThumbnailer {
    exe: PathBuf,
    max_edge: u32,
    /// `-q:v` for the mjpeg encoder: 2 (best) to 31 (worst). Upstream asks for JPEG quality 0.8,
    /// which is about 4 here.
    quality: u8,
}

impl FfmpegThumbnailer {
    #[must_use]
    pub fn new(exe: impl Into<PathBuf>) -> Self {
        Self {
            exe: exe.into(),
            max_edge: THUMBNAIL_MAX_EDGE,
            quality: 4,
        }
    }

    #[must_use]
    pub fn with_max_edge(mut self, max_edge: u32) -> Self {
        self.max_edge = max_edge.max(16);
        self
    }

    #[must_use]
    pub fn exe(&self) -> &Path {
        &self.exe
    }
}

impl Thumbnailer for FfmpegThumbnailer {
    fn generate(&self, video: &Path, at: MediaTime, to: &Path) -> Result<bool, ProjectError> {
        if !video.is_file() {
            return Ok(false);
        }
        let seek = |time: MediaTime| -> Result<bool, ProjectError> {
            // Preserve the `.nss-tmp-<uuid>` marker so crash recovery still recognises this temp
            // file while giving FFmpeg a `.jpg` suffix.
            let mut temp_os = crate::atomic_file::temp_path_for(to).into_os_string();
            temp_os.push(".jpg");
            let temp = PathBuf::from(temp_os);
            let scale = format!(
                "scale={e}:{e}:force_original_aspect_ratio=decrease",
                e = self.max_edge
            );
            let mut cmd = std::process::Command::new(&self.exe);
            cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
                .arg("-ss")
                .arg(format!("{:.3}", time.max(0.0)))
                .arg("-i")
                .arg(video)
                .args(["-frames:v", "1", "-update", "1", "-vf"])
                .arg(scale)
                .args(["-q:v", &self.quality.to_string()])
                .arg("-f")
                .arg("mjpeg")
                .arg(&temp)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                cmd.creation_flags(CREATE_NO_WINDOW);
            }
            let mut child = cmd.spawn().map_err(|e| ProjectError::io(&self.exe, e))?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            let status = loop {
                match child.try_wait() {
                    Ok(Some(st)) => break st,
                    Ok(None) => {
                        if std::time::Instant::now() >= deadline {
                            let _ = child.kill();
                            let _ = child.wait();
                            let _ = fs::remove_file(&temp);
                            return Ok(false);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(25));
                    }
                    Err(e) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = fs::remove_file(&temp);
                        return Err(ProjectError::io(&self.exe, e));
                    }
                }
            };
            if !status.success() || !temp.is_file() {
                let _ = fs::remove_file(&temp);
                return Ok(false);
            }
            let bytes = fs::read(&temp).map_err(|e| ProjectError::io(&temp, e))?;
            let _ = fs::remove_file(&temp);
            if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
                return Ok(false);
            }
            crate::atomic_file::write_atomic(&bytes, to)?;
            Ok(true)
        };

        // The requested time can be past the real end of a short file; fall back to the first
        // frame, exactly as upstream does.
        if seek(at)? {
            return Ok(true);
        }
        if at > 0.0 {
            return seek(0.0);
        }
        Ok(false)
    }
}

/// Moves a bundle out of the library. Injectable so tests never touch the real Recycle Bin.
pub trait Trash: Send + Sync + std::fmt::Debug {
    fn move_to_trash(&self, path: &Path) -> Result<(), ProjectError>;
}

/// Construction options for [`FileProjectLibrary`].
pub struct LibraryOptions {
    pub library: PathBuf,
    pub migrator: ProjectMigrator,
    pub thumbnailer: Arc<dyn Thumbnailer>,
    pub trash: Arc<dyn Trash>,
}

impl std::fmt::Debug for LibraryOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LibraryOptions")
            .field("library", &self.library)
            .field("migrator", &self.migrator)
            .field("thumbnailer", &self.thumbnailer)
            .field("trash", &self.trash)
            .finish()
    }
}

impl Default for LibraryOptions {
    fn default() -> Self {
        Self {
            library: crate::store::default_library_dir(),
            migrator: ProjectMigrator::standard(),
            thumbnailer: Arc::new(NoopThumbnailer),
            trash: Arc::new(crate::windows_trash::RecycleBinTrash),
        }
    }
}

/// `ProjectLibrary` over the `.nssproj` bundles in a library folder.
pub struct FileProjectLibrary {
    library: PathBuf,
    migrator: ProjectMigrator,
    thumbnailer: Arc<dyn Thumbnailer>,
    trash: Arc<dyn Trash>,
    /// One lock per bundle, so a second `thumbnail_url` for the same bundle waits for the first
    /// instead of spawning a second FFmpeg (upstream keeps an `inFlight` dictionary of tasks).
    in_flight: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
}

impl std::fmt::Debug for FileProjectLibrary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileProjectLibrary")
            .field("library", &self.library)
            .finish()
    }
}

impl FileProjectLibrary {
    /// A library at `library` that generates no thumbnails and deletes to the Recycle Bin.
    #[must_use]
    pub fn at(library: impl Into<PathBuf>) -> Self {
        Self::new(LibraryOptions {
            library: library.into(),
            ..LibraryOptions::default()
        })
    }

    #[must_use]
    pub fn new(options: LibraryOptions) -> Self {
        Self {
            library: options.library,
            migrator: options.migrator,
            thumbnailer: options.thumbnailer,
            trash: options.trash,
            in_flight: Mutex::new(HashMap::new()),
        }
    }

    #[must_use]
    pub fn library(&self) -> &Path {
        &self.library
    }

    // MARK: Listing

    /// Every readable bundle, newest first. Unreadable bundles and stray files are skipped.
    pub fn summaries(&self) -> Vec<ProjectSummary> {
        let Ok(entries) = fs::read_dir(&self.library) else {
            return Vec::new();
        };
        let mut bundles: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && has_project_extension(p))
            .collect();
        // Sort deterministically before deduplicating project IDs so copied bundles keep the same
        // ID across launches regardless of directory entry enumeration order.
        bundles.sort_by_key(|p| crate::debounce::path_key(p));

        let mut result: Vec<ProjectSummary> = Vec::new();
        let mut seen: HashSet<Uuid> = HashSet::new();
        for bundle in bundles {
            let Some(project) = self.read_project(&bundle) else {
                continue;
            };
            let thumbnail = Self::thumbnail_path(&bundle, &project);
            // A bundle copied in Explorer keeps the original's project id; give copies a stable
            // id of their own.
            let id = if seen.insert(project.id) {
                project.id
            } else {
                path_id(&bundle)
            };
            result.push(ProjectSummary {
                id,
                name: project.name.clone(),
                created_at: project.created_at,
                duration: project.assets.duration,
                bundle: ProjectReference::new(bundle),
                thumbnail_url: thumbnail.filter(|t| t.is_file()),
            });
        }
        result.sort_by(|a, b| {
            b.created_at
                .0
                .partial_cmp(&a.created_at.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        result
    }

    /// Deterministic UUID from a path (FNV-1a, two 64-bit lanes), stable across launches.
    #[must_use]
    pub fn id_for_path(path: &Path) -> Uuid {
        path_id(path)
    }

    // MARK: Thumbnails

    /// About 1 s in, or the middle of clips shorter than 2 s.
    #[must_use]
    pub fn thumbnail_time(duration: MediaTime) -> MediaTime {
        if duration > 0.0 {
            if duration >= 2.0 {
                1.0
            } else {
                duration / 2.0
            }
        } else {
            0.0
        }
    }

    /// The thumbnail file the project names, whether or not it exists yet.
    #[must_use]
    pub fn thumbnail_path(bundle: &Path, project: &Project) -> Option<PathBuf> {
        let name = project
            .assets
            .thumbnail_file
            .clone()
            .unwrap_or_else(|| THUMBNAIL_FILE_NAME.to_owned());
        if !nss_core::RecordingAssets::is_safe_file_name(&name) {
            return None;
        }
        Some(bundle.join(name))
    }

    /// Generates the thumbnail if it is missing. Blocks for the duration of the FFmpeg call, so
    /// call it off the UI thread; a second call for the same bundle waits instead of duplicating
    /// the work.
    pub fn thumbnail_url(&self, summary: &ProjectSummary) -> Option<PathBuf> {
        let bundle = &summary.bundle.url;
        let project = self.read_project(bundle)?;
        let target = Self::thumbnail_path(bundle, &project)?;
        if target.is_file() {
            return Some(target);
        }
        let key = crate::debounce::path_key(bundle);
        let lock = {
            let mut map = self.in_flight.lock();
            Arc::clone(
                map.entry(key.clone())
                    .or_insert_with(|| Arc::new(Mutex::new(()))),
            )
        };
        let result = {
            let _generating = lock.lock();
            // Another thread may have produced it while we waited for the lock.
            if target.is_file() {
                Some(target)
            } else {
                let video = bundle.join(&project.assets.screen_file);
                let at = Self::thumbnail_time(project.assets.duration);
                match self.thumbnailer.generate(&video, at, &target) {
                    Ok(true) => Some(target),
                    Ok(false) => None,
                    Err(_) => None,
                }
            }
        };
        // Prune the map entry once no other caller holds a reference to this per-bundle lock.
        {
            let mut map = self.in_flight.lock();
            if map.get(&key).is_some_and(|existing| {
                Arc::ptr_eq(existing, &lock) && Arc::strong_count(&lock) == 2
            }) {
                map.remove(&key);
            }
        }
        result
    }

    // MARK: Delete and rename

    pub fn delete(&self, summary: &ProjectSummary) -> Result<(), ProjectError> {
        self.trash
            .move_to_trash(&summary.bundle.url)
            .map_err(|e| match e {
                ProjectError::Delete(m) => ProjectError::Delete(m),
                ProjectError::Io { source, .. } => ProjectError::Delete(format!(
                    "Couldn't move {} to the Recycle Bin: {source}",
                    leaf(&summary.bundle.url)
                )),
                other => ProjectError::Delete(format!(
                    "Couldn't move {} to the Recycle Bin: {other}",
                    leaf(&summary.bundle.url)
                )),
            })
    }

    /// Changes the project name in `project.json` (and the backup), leaving the folder alone.
    pub fn rename(&self, summary: &ProjectSummary, name: &str) -> Result<(), ProjectError> {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(ProjectError::InvalidName(
                "A name can't be empty".to_owned(),
            ));
        }
        // Primary file only: never rewrite project.json from an older backup.
        let primary = summary.bundle.project_file();
        let data = crate::atomic_file::read_with_retry(&primary).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ProjectError::Corrupt(format!(
                    "Missing {} in {}",
                    nss_core::ProjectReference::PROJECT_FILE_NAME,
                    leaf(&summary.bundle.url)
                ))
            } else {
                ProjectError::io(&primary, e)
            }
        })?;
        let migrated = self.migrator.migrate(&data)?;
        let mut project: Project = serde_json::from_slice(&migrated).map_err(|e| {
            ProjectError::Corrupt(format!(
                "Corrupt {}: {e}",
                nss_core::ProjectReference::PROJECT_FILE_NAME
            ))
        })?;
        if project.name == trimmed {
            return Ok(());
        }
        project.name = trimmed.to_owned();
        FileProjectStore::write_project(&project, &summary.bundle)
    }

    // MARK: Reading

    /// The project, from `project.json` or, if that is unreadable, from the backup.
    fn read_project(&self, bundle: &Path) -> Option<Project> {
        for file in [
            nss_core::ProjectReference::PROJECT_FILE_NAME,
            "project.json.bak",
        ] {
            let Ok(data) = fs::read(bundle.join(file)) else {
                continue;
            };
            let Ok(migrated) = self.migrator.migrate(&data) else {
                continue;
            };
            let Ok(project) = serde_json::from_slice::<Project>(&migrated) else {
                continue;
            };
            if project.assets.unsafe_file_name().is_none() {
                return Some(project);
            }
        }
        None
    }
}

fn leaf(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn has_project_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(nss_core::ProjectReference::FILE_EXTENSION))
}

/// FNV-1a over the standardised path, two lanes, as upstream does. Windows paths are folded to
/// lower case first so `C:\a` and `c:\A` name the same bundle.
fn path_id(path: &Path) -> Uuid {
    let text = path.to_string_lossy();
    let normalized = if cfg!(windows) {
        text.replace('\\', "/").to_lowercase()
    } else {
        text.into_owned()
    };
    let mut a: u64 = 0xcbf2_9ce4_8422_2325;
    let mut b: u64 = 0x8422_2325_cbf2_9ce4;
    for byte in normalized.as_bytes() {
        a = (a ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
        b = (b.wrapping_add(u64::from(*byte))).wrapping_mul(0x0000_0100_0000_01b3) ^ (b >> 29);
    }
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&a.to_le_bytes());
    bytes[8..].copy_from_slice(&b.to_le_bytes());
    Uuid::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nss_core::{PixelSize, RecordingAssets, Timestamp};

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nss-lib-{tag}-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Creates a bundle with a `project.json`, and optionally a fake movie.
    fn make_bundle(
        library: &Path,
        name: &str,
        created: Timestamp,
        duration: MediaTime,
        movie: bool,
    ) -> PathBuf {
        let bundle = library.join(format!("{name}.nssproj"));
        fs::create_dir_all(&bundle).unwrap();
        let mut project = Project::new(
            name,
            RecordingAssets::new(PixelSize::new(64, 64), 60.0, duration),
        );
        project.created_at = created;
        FileProjectStore::write_project(&project, &ProjectReference::new(bundle.clone())).unwrap();
        if movie {
            fs::write(bundle.join("screen.mov"), b"not really a movie").unwrap();
        }
        bundle
    }

    #[derive(Debug, Clone, Copy)]
    struct FakeThumbnailer {
        /// Bytes to write; `None` means "the video is unreadable".
        writes: bool,
    }

    impl Thumbnailer for FakeThumbnailer {
        fn generate(&self, video: &Path, _at: MediaTime, to: &Path) -> Result<bool, ProjectError> {
            // A missing video is "unreadable", exactly like the real FFmpeg path.
            if !self.writes || !video.is_file() {
                return Ok(false);
            }
            crate::atomic_file::write_atomic(&[0xFF, 0xD8, 0xFF, 0xD9], to)?;
            Ok(true)
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct FailingTrash;

    impl Trash for FailingTrash {
        fn move_to_trash(&self, _path: &Path) -> Result<(), ProjectError> {
            Err(ProjectError::Delete("no bin".to_owned()))
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct RemovingTrash;

    impl Trash for RemovingTrash {
        fn move_to_trash(&self, path: &Path) -> Result<(), ProjectError> {
            fs::remove_dir_all(path).map_err(|e| ProjectError::io(path, e))
        }
    }

    fn library_with(
        dir: &Path,
        thumbnailer: FakeThumbnailer,
        trash: Arc<dyn Trash>,
    ) -> FileProjectLibrary {
        FileProjectLibrary::new(LibraryOptions {
            library: dir.to_path_buf(),
            thumbnailer: Arc::new(thumbnailer),
            trash,
            ..LibraryOptions::default()
        })
    }

    #[test]
    fn lists_newest_first_and_skips_junk() {
        let dir = scratch("list");
        make_bundle(&dir, "old", Timestamp(1000.0), 3.0, false);
        make_bundle(&dir, "new", Timestamp(3000.0), 3.0, false);
        make_bundle(&dir, "mid", Timestamp(2000.0), 3.0, false);
        fs::create_dir_all(dir.join("broken.nssproj")).unwrap();
        fs::write(dir.join("notes.txt"), b"x").unwrap();
        let names: Vec<String> = library_with(
            &dir,
            FakeThumbnailer { writes: false },
            Arc::new(RemovingTrash),
        )
        .summaries()
        .into_iter()
        .map(|s| s.name)
        .collect();
        assert_eq!(names, vec!["new", "mid", "old"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn copied_bundles_get_distinct_stable_ids() {
        let dir = scratch("copies");
        let a = make_bundle(&dir, "a", Timestamp(1.0), 3.0, false);
        copy_dir(&a, &dir.join("a copy.nssproj"));
        let lib = library_with(
            &dir,
            FakeThumbnailer { writes: false },
            Arc::new(RemovingTrash),
        );
        let first = lib.summaries();
        let second = lib.summaries();
        assert_eq!(first.len(), 2);
        let ids: HashSet<Uuid> = first.iter().map(|s| s.id).collect();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids, second.iter().map(|s| s.id).collect());
        fs::remove_dir_all(&dir).unwrap();
    }

    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap().flatten() {
            fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
        }
    }

    #[test]
    fn an_empty_or_missing_library_is_empty() {
        let dir = scratch("empty");
        let lib = library_with(
            &dir,
            FakeThumbnailer { writes: false },
            Arc::new(RemovingTrash),
        );
        assert!(lib.summaries().is_empty());
        assert!(FileProjectLibrary::at(dir.join("nope"))
            .summaries()
            .is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rename_changes_the_name_only() {
        let dir = scratch("rename");
        let bundle = make_bundle(&dir, "a", Timestamp::now(), 3.0, false);
        let lib = library_with(
            &dir,
            FakeThumbnailer { writes: false },
            Arc::new(RemovingTrash),
        );
        let summary = lib.summaries().into_iter().next().unwrap();
        lib.rename(&summary, "  Launch demo ").unwrap();
        let after = lib.summaries().into_iter().next().unwrap();
        assert_eq!(after.name, "Launch demo");
        assert_eq!(after.bundle.url, bundle);
        assert_eq!(after.id, summary.id);
        assert!(lib.rename(&summary, "   ").is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rename_refuses_an_unreadable_primary_and_leaves_it_untouched() {
        for content in [
            "{\"schemaVersion\": 99, \"name\": \"x\"}",
            "not json at all",
        ] {
            let dir = scratch("rename-bad");
            let bundle = make_bundle(&dir, "a", Timestamp::now(), 3.0, false);
            let primary = bundle.join("project.json");
            fs::copy(&primary, bundle.join("project.json.bak")).unwrap();
            fs::write(&primary, content).unwrap();
            let lib = library_with(
                &dir,
                FakeThumbnailer { writes: false },
                Arc::new(RemovingTrash),
            );
            let summary = lib
                .summaries()
                .into_iter()
                .next()
                .expect("listing may use the backup");
            assert!(lib.rename(&summary, "New").is_err());
            assert_eq!(fs::read(&primary).unwrap(), content.as_bytes());
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn delete_reports_success_and_failure() {
        let dir = scratch("delete");
        let bundle = make_bundle(&dir, "a", Timestamp::now(), 3.0, false);
        let ok = library_with(
            &dir,
            FakeThumbnailer { writes: false },
            Arc::new(RemovingTrash),
        );
        let summary = ok.summaries().into_iter().next().unwrap();
        ok.delete(&summary).unwrap();
        assert!(!bundle.exists());
        assert!(ok.summaries().is_empty());

        let bundle = make_bundle(&dir, "b", Timestamp::now(), 3.0, false);
        let failing = library_with(
            &dir,
            FakeThumbnailer { writes: false },
            Arc::new(FailingTrash),
        );
        let summary = failing.summaries().into_iter().next().unwrap();
        assert!(failing.delete(&summary).is_err());
        assert!(bundle.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_thumbnail_time_rule_matches_upstream() {
        assert_eq!(FileProjectLibrary::thumbnail_time(10.0), 1.0);
        assert_eq!(FileProjectLibrary::thumbnail_time(2.0), 1.0);
        assert_eq!(FileProjectLibrary::thumbnail_time(1.0), 0.5);
        assert_eq!(FileProjectLibrary::thumbnail_time(0.0), 0.0);
    }

    #[test]
    fn a_thumbnail_is_generated_then_cached() {
        let dir = scratch("thumb");
        let bundle = make_bundle(&dir, "a", Timestamp::now(), 3.0, true);
        let lib = library_with(
            &dir,
            FakeThumbnailer { writes: true },
            Arc::new(RemovingTrash),
        );
        let summary = lib.summaries().into_iter().next().unwrap();
        assert_eq!(summary.thumbnail_url, None);

        let url = lib.thumbnail_url(&summary).expect("generated");
        assert_eq!(url, bundle.join("thumbnail.jpg"));
        let bytes = fs::read(&url).unwrap();
        assert_eq!(&bytes[..2], &[0xFF, 0xD8]);

        // Cached: listing now reports it, and a second request does not rewrite the file.
        assert_eq!(
            lib.summaries().first().unwrap().thumbnail_url,
            Some(url.clone())
        );
        let modified = fs::metadata(&url).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(lib.thumbnail_url(&summary), Some(url.clone()));
        assert_eq!(fs::metadata(&url).unwrap().modified().unwrap(), modified);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_movie_gives_no_thumbnail() {
        let dir = scratch("nothumb");
        make_bundle(&dir, "a", Timestamp::now(), 3.0, false);
        let lib = library_with(
            &dir,
            FakeThumbnailer { writes: true },
            Arc::new(RemovingTrash),
        );
        let summary = lib.summaries().into_iter().next().unwrap();
        assert_eq!(lib.thumbnail_url(&summary), None);
        assert!(!make_bundle(&dir, "b", Timestamp::now(), 3.0, false)
            .join("thumbnail.jpg")
            .exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_unsafe_thumbnail_name_is_ignored() {
        let dir = scratch("unsafe-thumb");
        let bundle = make_bundle(&dir, "a", Timestamp::now(), 3.0, false);
        let mut project: Project =
            serde_json::from_slice(&fs::read(bundle.join("project.json")).unwrap()).unwrap();
        project.assets.thumbnail_file = Some("../evil.jpg".to_owned());
        assert_eq!(FileProjectLibrary::thumbnail_path(&bundle, &project), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_path_id_is_stable_and_case_folded_on_windows() {
        let a = FileProjectLibrary::id_for_path(Path::new("C:/lib/Demo.nssproj"));
        let b = FileProjectLibrary::id_for_path(Path::new("C:/lib/Demo.nssproj"));
        assert_eq!(a, b);
        let c = FileProjectLibrary::id_for_path(Path::new("C:/lib/Other.nssproj"));
        assert_ne!(a, c);
        if cfg!(windows) {
            assert_eq!(
                a,
                FileProjectLibrary::id_for_path(Path::new("c:/LIB/demo.nssproj"))
            );
        }
    }

    #[test]
    fn a_bundle_with_an_upper_case_extension_is_listed() {
        let dir = scratch("upper");
        let bundle = dir.join("Upper.NSSPROJ");
        fs::create_dir_all(&bundle).unwrap();
        let project = Project::new(
            "upper",
            RecordingAssets::new(PixelSize::new(8, 8), 60.0, 1.0),
        );
        FileProjectStore::write_project(&project, &ProjectReference::new(bundle)).unwrap();
        assert_eq!(
            library_with(
                &dir,
                FakeThumbnailer { writes: false },
                Arc::new(RemovingTrash)
            )
            .summaries()
            .len(),
            1
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
