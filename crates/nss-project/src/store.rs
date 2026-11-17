//! `.nssproj` bundle store.
//!
//! Port of upstream `NSSProject/FileProjectStore.swift`. A bundle is a folder holding
//! `project.json`, the media files and `cursor.json`.
//!
//! * Writes are atomic (`atomic_file`), and the last good `project.json` is kept as
//!   `project.json.bak`.
//! * [`schedule_autosave`](FileProjectStore::schedule_autosave) debounces: the write happens 1 s
//!   after the last change. Call [`flush_pending_autosaves`](FileProjectStore::flush_pending_autosaves)
//!   on quit so nothing is lost.
//! * [`load`](FileProjectStore::load) migrates older schemas and falls back to the backup when
//!   `project.json` is missing or corrupt.
//!
//! Synchronous by design (ADR 0009): every method here is called from the Tauri command layer,
//! which owns the async boundary. The autosave debouncer owns the only background thread.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nss_core::{CursorTrack, Project, ProjectReference, RecordingAssets, Timestamp};
use parking_lot::Mutex;

use crate::atomic_file;
use crate::debounce::{AutosaveDebouncer, DebounceClock, ErrorHandler, SystemClock, Writer};
use crate::errors::ProjectError;
use crate::migrator::ProjectMigrator;
use crate::recent_projects::{RecentProject, RecentProjects};

/// Longest bundle name we will create. Leaves room for the library path and the `.nssproj`
/// suffix under the 260-character `MAX_PATH` limit most Windows tools still assume.
const MAX_BUNDLE_NAME: usize = 100;

/// Windows device names are unusable as file names, with or without an extension.
const RESERVED_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Construction options, mirroring upstream's defaulted initializer.
pub struct StoreOptions {
    /// Where bundles live. Defaults to `%USERPROFILE%\Videos\notscreenstudio`.
    pub library: PathBuf,
    pub migrator: ProjectMigrator,
    /// Seconds of quiet before an autosave writes.
    pub autosave_delay: f64,
    pub clock: Arc<dyn DebounceClock>,
    pub on_autosave_error: Option<ErrorHandler>,
    /// Injectable "now" so tests get deterministic recents ordering.
    pub now: Arc<dyn Fn() -> Timestamp + Send + Sync>,
}

impl std::fmt::Debug for StoreOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreOptions")
            .field("library", &self.library)
            .field("migrator", &self.migrator)
            .field("autosave_delay", &self.autosave_delay)
            .finish_non_exhaustive()
    }
}

impl Default for StoreOptions {
    fn default() -> Self {
        Self {
            library: default_library_dir(),
            migrator: ProjectMigrator::standard(),
            autosave_delay: 1.0,
            clock: Arc::new(SystemClock::new()),
            on_autosave_error: None,
            now: Arc::new(Timestamp::now),
        }
    }
}

/// The library root. `%USERPROFILE%\Videos\notscreenstudio`, per PLAN.md §5 Stage 3.
///
/// Note: a redirected Videos folder (OneDrive) is not followed here; Stage 8 switches this to
/// `SHGetKnownFolderPath(FOLDERID_Videos)`.
#[must_use]
pub fn default_library_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    home.join("Videos").join("notscreenstudio")
}

/// A `ProjectStore` over a library folder of `.nssproj` bundles.
pub struct FileProjectStore {
    library: PathBuf,
    migrator: ProjectMigrator,
    debouncer: AutosaveDebouncer,
    now: Arc<dyn Fn() -> Timestamp + Send + Sync>,
    /// Serialises recents read-modify-write cycles, which upstream gets from being an actor.
    recents_lock: Mutex<()>,
}

impl std::fmt::Debug for FileProjectStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileProjectStore")
            .field("library", &self.library)
            .finish()
    }
}

impl FileProjectStore {
    pub const BACKUP_FILE_NAME: &'static str = "project.json.bak";
    pub const RECENTS_FILE_NAME: &'static str = ".recent-projects.json";

    /// A store at `library` with every default (1 s autosave, real clock, standard migrator).
    #[must_use]
    pub fn at(library: impl Into<PathBuf>) -> Self {
        Self::new(StoreOptions {
            library: library.into(),
            ..StoreOptions::default()
        })
    }

    #[must_use]
    pub fn new(options: StoreOptions) -> Self {
        let writer: Writer = Arc::new(|project: &Project, reference: &ProjectReference| {
            Self::write_project(project, reference)
        });
        let debouncer = AutosaveDebouncer::new(
            options.autosave_delay,
            Arc::clone(&options.clock),
            options.on_autosave_error,
            writer,
        );
        Self {
            library: options.library,
            migrator: options.migrator,
            debouncer,
            now: options.now,
            recents_lock: Mutex::new(()),
        }
    }

    #[must_use]
    pub fn library(&self) -> &Path {
        &self.library
    }

    #[must_use]
    pub fn recents_file(&self) -> PathBuf {
        self.library.join(Self::RECENTS_FILE_NAME)
    }

    // MARK: Create

    /// Creates a new, empty bundle and returns it. Never reuses an existing one.
    pub fn create_project(&self, name: &str) -> Result<ProjectReference, ProjectError> {
        fs::create_dir_all(&self.library).map_err(|e| ProjectError::io(&self.library, e))?;
        let base = sanitize_name(name);
        for n in 1..10_000 {
            let leaf = if n == 1 {
                base.clone()
            } else {
                format!("{base} {n}")
            };
            let bundle = self
                .library
                .join(format!("{leaf}.{}", ProjectReference::FILE_EXTENSION));
            match fs::create_dir(&bundle) {
                // Non-recursive create fails when it exists, so a bundle is never reused.
                Ok(()) => {
                    let reference = ProjectReference::new(bundle);
                    self.touch_recent(&reference, name);
                    return Ok(reference);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(ProjectError::io(&bundle, e)),
            }
        }
        Err(ProjectError::InvalidName(format!(
            "Cannot find a free name for \"{name}\""
        )))
    }

    // MARK: Load / save

    /// Loads a bundle, migrating an older schema and falling back to `project.json.bak`.
    pub fn load(&self, reference: &ProjectReference) -> Result<Project, ProjectError> {
        if !reference.url.is_dir() {
            return Err(ProjectError::NotFound(
                reference.url.file_name().map_or_else(
                    || reference.url.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                ),
            ));
        }
        let primary = reference.project_file();
        let project = match Self::read_project(&self.migrator, &primary) {
            Ok(project) => project,
            Err(primary_error) => {
                // A newer-schema file must not be silently replaced by an older backup.
                if matches!(primary_error, ProjectError::Migration(_)) {
                    return Err(primary_error);
                }
                match Self::read_project(&self.migrator, &reference.backup_file()) {
                    Ok(recovered) => recovered,
                    Err(_) => return Err(primary_error),
                }
            }
        };
        self.touch_recent(reference, &project.name);
        Ok(project)
    }

    fn read_project(migrator: &ProjectMigrator, file: &Path) -> Result<Project, ProjectError> {
        let leaf = file
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let data = fs::read(file).map_err(|_| ProjectError::Corrupt(format!("Missing {leaf}")))?;
        let migrated = migrator.migrate(&data)?;
        let project: Project = serde_json::from_slice(&migrated)
            .map_err(|e| ProjectError::Corrupt(format!("Corrupt {leaf}: {e}")))?;
        // Asset names are joined onto the bundle path: they must not point outside it.
        if let Some(bad) = project.assets.unsafe_file_name() {
            return Err(ProjectError::Corrupt(format!(
                "{leaf} names a file outside the project: {bad}"
            )));
        }
        Ok(project)
    }

    /// An explicit save. It waits for any in-flight autosave, drops the pending one, and becomes
    /// the last write for the bundle.
    pub fn save(
        &self,
        project: &Project,
        reference: &ProjectReference,
    ) -> Result<(), ProjectError> {
        let result = self
            .debouncer
            .exclusive(reference, || Self::write_project(project, reference));
        result?;
        self.touch_recent(reference, &project.name);
        Ok(())
    }

    /// Atomic write, keeping the previous valid `project.json` as a backup.
    ///
    /// Synchronous and static so the debouncer can call it from its worker thread and from
    /// `flush()` at quit.
    pub fn write_project(
        project: &Project,
        reference: &ProjectReference,
    ) -> Result<(), ProjectError> {
        fs::create_dir_all(&reference.url).map_err(|e| ProjectError::io(&reference.url, e))?;
        let primary = reference.project_file();
        let data = encode_project(project)?;
        // Keep the old file only if it still decodes: a corrupt one must not overwrite a good
        // backup.
        if let Ok(old) = fs::read(&primary) {
            if serde_json::from_slice::<Project>(&old).is_ok() {
                let _ = atomic_file::write_atomic(&old, &reference.backup_file());
            }
        }
        atomic_file::write_atomic(&data, &primary)
    }

    /// Removes temp files a crash left behind in every bundle of the library.
    pub fn remove_stale_temps(&self) -> usize {
        let Ok(entries) = fs::read_dir(&self.library) else {
            return 0;
        };
        let mut removed = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && has_project_extension(&path) {
                removed += atomic_file::remove_stale_temps(&path);
            }
        }
        removed += atomic_file::remove_stale_temps(&self.library);
        removed
    }

    // MARK: Cursor track

    /// A missing `cursor.json` yields an empty track (the video is still usable); a corrupt one
    /// is an error.
    pub fn load_cursor_track(
        &self,
        reference: &ProjectReference,
        assets: &RecordingAssets,
    ) -> Result<CursorTrack, ProjectError> {
        let file = reference.url.join(&assets.cursor_file);
        let Ok(data) = fs::read(&file) else {
            return Ok(CursorTrack::default());
        };
        serde_json::from_slice(&data)
            .map_err(|e| ProjectError::Corrupt(format!("Corrupt {}: {e}", assets.cursor_file)))
    }

    pub fn save_cursor_track(
        &self,
        track: &CursorTrack,
        reference: &ProjectReference,
        assets: &RecordingAssets,
    ) -> Result<(), ProjectError> {
        // Compact: this is the largest file in the bundle (120 samples/s).
        let data = serde_json::to_vec(track)
            .map_err(|e| ProjectError::Corrupt(format!("cannot encode cursor track: {e}")))?;
        atomic_file::write_atomic(&data, &reference.url.join(&assets.cursor_file))
    }

    // MARK: Autosave

    pub fn schedule_autosave(&self, project: Project, reference: &ProjectReference) {
        self.debouncer.schedule(project, reference);
    }

    /// Writes all pending autosaves now. Call on app quit.
    pub fn flush_pending_autosaves(&self) -> usize {
        self.debouncer.flush()
    }

    #[must_use]
    pub fn has_pending_autosave(&self, reference: &ProjectReference) -> bool {
        self.debouncer.has_pending(reference)
    }

    // MARK: Listing and recents

    /// All bundles in the library, newest first.
    pub fn list_projects(&self) -> Vec<ProjectReference> {
        let Ok(entries) = fs::read_dir(&self.library) else {
            return Vec::new();
        };
        let mut bundles: Vec<(std::time::SystemTime, ProjectReference)> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && has_project_extension(p))
            .map(|p| {
                let modified = fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH);
                (modified, ProjectReference::new(p))
            })
            .collect();
        bundles.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
        bundles.into_iter().map(|(_, r)| r).collect()
    }

    /// Recently opened projects, newest first, with bundles that no longer exist removed.
    pub fn recent_projects(&self) -> Vec<RecentProject> {
        let _guard = self.recents_lock.lock();
        let mut list = self.read_recents();
        if list.prune(|path| Path::new(path).exists()) {
            self.write_recents(&list);
        }
        list.items
    }

    pub fn remove_from_recents(&self, reference: &ProjectReference) {
        let _guard = self.recents_lock.lock();
        let mut list = self.read_recents();
        list.remove(&reference.url.to_string_lossy());
        self.write_recents(&list);
    }

    fn read_recents(&self) -> RecentProjects {
        fs::read(self.recents_file())
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }

    fn write_recents(&self, list: &RecentProjects) {
        let Ok(data) = encode_pretty(list) else {
            return;
        };
        let _ = fs::create_dir_all(&self.library);
        let _ = atomic_file::write_atomic(&data, &self.recents_file());
    }

    fn touch_recent(&self, reference: &ProjectReference, name: &str) {
        let _guard = self.recents_lock.lock();
        let mut list = self.read_recents();
        list.touch(&reference.url.to_string_lossy(), name, (self.now)());
        self.write_recents(&list);
    }
}

/// Makes `name` safe as one path component: no separators, no characters Windows forbids, no
/// leading or trailing dots (Windows silently strips trailing ones), never a device name, never
/// empty.
#[must_use]
pub fn sanitize_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        let bad = matches!(
            ch,
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\0'
        ) || (ch as u32) < 0x20;
        out.push(if bad { '-' } else { ch });
    }
    let mut s = out.trim().to_owned();
    while s.starts_with('.') {
        s.remove(0);
    }
    // Windows cannot create a name ending in a dot or a space.
    while s.ends_with('.') || s.ends_with(' ') {
        s.pop();
    }
    if s.chars().count() > MAX_BUNDLE_NAME {
        s = s.chars().take(MAX_BUNDLE_NAME).collect::<String>();
        while s.ends_with('.') || s.ends_with(' ') {
            s.pop();
        }
    }
    let stem = s.split('.').next().unwrap_or("").to_lowercase();
    if RESERVED_NAMES.contains(&stem.as_str()) {
        s = format!("_{s}");
    }
    if s.is_empty() {
        "Untitled".to_owned()
    } else {
        s
    }
}

/// True when `path` looks like a bundle (extension compared case-insensitively, as Windows does).
fn has_project_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(ProjectReference::FILE_EXTENSION))
}

/// Pretty, sorted-key JSON: what upstream's `JSONEncoder` does with `.prettyPrinted, .sortedKeys`.
pub(crate) fn encode_pretty<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, ProjectError> {
    let as_value = serde_json::to_value(value)
        .map_err(|e| ProjectError::Corrupt(format!("cannot encode JSON: {e}")))?;
    serde_json::to_vec_pretty(&as_value)
        .map_err(|e| ProjectError::Corrupt(format!("cannot encode JSON: {e}")))
}

pub(crate) fn encode_project(project: &Project) -> Result<Vec<u8>, ProjectError> {
    encode_pretty(project)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::ProjectMigrationError;
    use nss_core::{PixelSize, Style};
    use std::sync::Arc;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nss-store-{tag}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Bounded wait for a condition produced by the debouncer's worker thread.
    fn settle_until(check: impl Fn() -> bool) {
        for _ in 0..5_000 {
            if check() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn sample(name: &str, duration: f64) -> Project {
        Project::new(
            name,
            RecordingAssets::new(PixelSize::new(100, 100), 60.0, duration),
        )
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = scratch("roundtrip");
        let store = FileProjectStore::at(&dir);
        let reference = store.create_project("t").unwrap();
        let project = sample("t", 1.0);
        store.save(&project, &reference).unwrap();
        assert_eq!(store.load(&reference).unwrap(), project);

        let track = CursorTrack {
            samples: vec![nss_core::CursorSample::new(0.0, 0.1, 0.2)],
            ..CursorTrack::default()
        };
        store
            .save_cursor_track(&track, &reference, &project.assets)
            .unwrap();
        assert_eq!(
            store
                .load_cursor_track(&reference, &project.assets)
                .unwrap(),
            track
        );
        assert_eq!(store.list_projects().len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_same_name_gives_distinct_bundles_inside_the_library() {
        let dir = scratch("names");
        let store = FileProjectStore::at(&dir);
        let a = store.create_project("Demo").unwrap();
        let b = store.create_project("Demo").unwrap();
        assert_ne!(a.url, b.url);
        assert_eq!(b.url.file_name().unwrap(), "Demo 2.nssproj");
        let evil = store.create_project("../../escape/x").unwrap();
        assert_eq!(evil.url.parent().unwrap(), dir);
        assert_eq!(store.list_projects().len(), 3);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn names_are_safe_for_windows() {
        assert_eq!(
            sanitize_name("a/b\\c:d*e?f\"g<h>i|j"),
            "a-b-c-d-e-f-g-h-i-j"
        );
        assert_eq!(sanitize_name("  spaced  "), "spaced");
        assert_eq!(sanitize_name("...hidden"), "hidden");
        assert_eq!(sanitize_name("trailing dot."), "trailing dot");
        assert_eq!(sanitize_name("trailing space "), "trailing space");
        assert_eq!(sanitize_name(""), "Untitled");
        assert_eq!(sanitize_name("   "), "Untitled");
        assert_eq!(sanitize_name("CON"), "_CON");
        assert_eq!(sanitize_name("con.txt"), "_con.txt");
        assert_eq!(sanitize_name("LPT9"), "_LPT9");
        assert_eq!(sanitize_name("console"), "console");
        assert_eq!(
            sanitize_name(&"x".repeat(300)).chars().count(),
            MAX_BUNDLE_NAME
        );
        // Control characters become dashes rather than producing an invalid name.
        assert_eq!(sanitize_name("a\u{1}b"), "a-b");
    }

    #[test]
    fn a_cursor_track_is_written_compactly() {
        let dir = scratch("compact");
        let store = FileProjectStore::at(&dir);
        let reference = store.create_project("c").unwrap();
        let assets = sample("c", 1.0).assets;
        let track = CursorTrack {
            samples: (0..1000)
                .map(|i| nss_core::CursorSample::new(i as f64 / 120.0, i as f64 / 1000.0, 0.5))
                .collect(),
            clicks: vec![nss_core::ClickEvent::new(
                1.0,
                0.5,
                0.5,
                nss_core::MouseButton::Right,
                nss_core::ClickKind::Up,
            )],
            ..CursorTrack::default()
        };
        store
            .save_cursor_track(&track, &reference, &assets)
            .unwrap();
        assert_eq!(store.load_cursor_track(&reference, &assets).unwrap(), track);
        let text = fs::read_to_string(reference.url.join("cursor.json")).unwrap();
        assert!(!text.contains('\n'), "cursor.json must stay on one line");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_cursor_file_yields_an_empty_track() {
        let dir = scratch("nocursor");
        let store = FileProjectStore::at(&dir);
        let reference = store.create_project("m").unwrap();
        let assets = sample("m", 1.0).assets;
        assert_eq!(
            store.load_cursor_track(&reference, &assets).unwrap(),
            CursorTrack::default()
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_bundles_and_files_are_reported() {
        let dir = scratch("missing");
        let store = FileProjectStore::at(&dir);
        let reference = store.create_project("x").unwrap();
        assert!(matches!(
            store.load(&reference),
            Err(ProjectError::Corrupt(_))
        ));
        assert!(matches!(
            store.load(&ProjectReference::new(dir.join("nope.nssproj"))),
            Err(ProjectError::NotFound(_))
        ));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_corrupt_primary_falls_back_to_the_backup_then_fails() {
        let dir = scratch("backup");
        let store = FileProjectStore::at(&dir);
        let reference = store.create_project("x").unwrap();
        store.save(&sample("one", 1.0), &reference).unwrap();
        store.save(&sample("two", 1.0), &reference).unwrap(); // "one" becomes the backup
        fs::write(reference.project_file(), b"{ truncated").unwrap();
        assert_eq!(store.load(&reference).unwrap().name, "one");
        fs::remove_file(reference.backup_file()).unwrap();
        assert!(matches!(
            store.load(&reference),
            Err(ProjectError::Corrupt(_))
        ));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_newer_schema_is_not_replaced_by_the_backup() {
        let dir = scratch("newer");
        let store = FileProjectStore::at(&dir);
        let reference = store.create_project("n").unwrap();
        store.save(&sample("first", 1.0), &reference).unwrap();
        store.save(&sample("again", 1.0), &reference).unwrap();
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(reference.project_file()).unwrap()).unwrap();
        json["schemaVersion"] = serde_json::Value::from(99);
        fs::write(reference.project_file(), serde_json::to_vec(&json).unwrap()).unwrap();
        assert!(matches!(
            store.load(&reference),
            Err(ProjectError::Migration(
                ProjectMigrationError::NewerSchema { .. }
            ))
        ));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_bundle_naming_a_file_outside_it_is_rejected() {
        let dir = scratch("escape");
        let store = FileProjectStore::at(&dir);
        let reference = store.create_project("e").unwrap();
        let mut json: serde_json::Value = serde_json::to_value(sample("e", 1.0)).unwrap();
        json["assets"]["screenFile"] = serde_json::Value::from("../../evil.mov");
        fs::write(reference.project_file(), serde_json::to_vec(&json).unwrap()).unwrap();
        let err = store.load(&reference).unwrap_err();
        assert!(err.to_string().contains("outside the project"), "{err}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_written_json_is_pretty_and_uses_swift_key_names() {
        let dir = scratch("pretty");
        let store = FileProjectStore::at(&dir);
        let reference = store.create_project("p").unwrap();
        let mut project = sample("p", 2.5);
        project.style = Style::default();
        store.save(&project, &reference).unwrap();
        let text = fs::read_to_string(reference.project_file()).unwrap();
        assert!(text.contains("\n  "), "pretty printed: {text}");
        for key in [
            "\"schemaVersion\"",
            "\"createdAt\"",
            "\"modifiedAt\"",
            "\"sourcePixelSize\"",
            "\"zoomBlocks\"",
        ] {
            assert!(text.contains(key), "missing {key}");
        }
        // The .bak only appears once a second valid save exists.
        assert!(!reference.backup_file().exists());
        store.save(&sample("p2", 2.5), &reference).unwrap();
        assert!(reference.backup_file().exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn listing_is_newest_first_and_ignores_junk() {
        let dir = scratch("listing");
        let store = FileProjectStore::at(&dir);
        let first = store.create_project("old").unwrap();
        store.save(&sample("old", 1.0), &first).unwrap();
        let second = store.create_project("new").unwrap();
        store.save(&sample("new", 1.0), &second).unwrap();
        // Nudge the modification times apart so the order is deterministic on every filesystem.
        let now = std::time::SystemTime::now();
        fs::File::options()
            .write(true)
            .open(first.project_file())
            .unwrap()
            .set_modified(now - std::time::Duration::from_secs(60))
            .unwrap();
        fs::create_dir_all(dir.join("Broken.nssproj")).unwrap();
        fs::write(dir.join("notes.txt"), b"x").unwrap();
        // `list_projects` filters by extension only (upstream does too); deciding whether a
        // bundle is *readable* is `FileProjectLibrary::summaries`'s job.
        let listed: Vec<String> = store
            .list_projects()
            .iter()
            .map(|r| r.url.file_name().unwrap().to_string_lossy().into_owned())
            .filter(|n| n != "Broken.nssproj")
            .collect();
        assert_eq!(listed, vec!["new.nssproj", "old.nssproj"]);
        assert!(!store
            .list_projects()
            .iter()
            .any(|r| r.url.extension().is_some_and(|e| e == "txt")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recents_track_opens_and_prune_deleted_bundles() {
        let dir = scratch("recents");
        let store = FileProjectStore::at(&dir);
        let a = store.create_project("A").unwrap();
        let b = store.create_project("B").unwrap();
        store.save(&sample("A", 1.0), &a).unwrap();
        store.load(&a).unwrap();
        let names: Vec<String> = store
            .recent_projects()
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(names, vec!["A".to_owned(), "B".to_owned()]);
        fs::remove_dir_all(&b.url).unwrap();
        let names: Vec<String> = store
            .recent_projects()
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(names, vec!["A".to_owned()]);
        // The recents file does not show up as a project.
        assert_eq!(store.list_projects().len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stale_temps_are_cleaned_from_bundles() {
        let dir = scratch("temps");
        let store = FileProjectStore::at(&dir);
        let reference = store.create_project("t").unwrap();
        fs::write(
            reference
                .url
                .join(format!(".project.json{}.abc", atomic_file::TEMP_MARKER)),
            b"junk",
        )
        .unwrap();
        assert_eq!(store.remove_stale_temps(), 1);
        assert_eq!(fs::read_dir(&reference.url).unwrap().count(), 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_store_at_the_default_library_points_into_the_users_videos() {
        let dir = default_library_dir();
        assert!(dir.ends_with("notscreenstudio"));
        assert!(dir.to_string_lossy().contains("Videos"));
    }

    #[test]
    fn autosave_writes_after_the_delay_and_an_explicit_save_supersedes() {
        // Mirrors upstream `storeAutosaveWritesToDiskAfterDelayAndExplicitSaveSupersedes`.
        let dir = scratch("autosave");
        let clock = Arc::new(crate::debounce::ManualClock::new());
        let store = FileProjectStore::new(StoreOptions {
            library: dir.clone(),
            clock: Arc::clone(&clock) as Arc<dyn DebounceClock>,
            on_autosave_error: Some(Arc::new(|e, r| {
                panic!("autosave failed for {}: {e}", r.url.display());
            })),
            ..StoreOptions::default()
        });
        let reference = store.create_project("auto").unwrap();
        store.schedule_autosave(sample("auto-1", 1.0), &reference);
        store.schedule_autosave(sample("auto-2", 1.0), &reference);
        clock.wait_for_sleepers(1);
        clock.advance(1.0);
        // The worker removes the pending entry before it writes, so wait for the outcome (the file
        // on disk), not for "no longer pending".
        settle_until(|| reference.project_file().is_file());
        assert!(!store.has_pending_autosave(&reference));
        assert_eq!(store.load(&reference).unwrap().name, "auto-2");

        store.schedule_autosave(sample("stale", 1.0), &reference);
        store.save(&sample("explicit", 1.0), &reference).unwrap();
        assert!(!store.has_pending_autosave(&reference));
        assert_eq!(store.flush_pending_autosaves(), 0);
        assert_eq!(store.load(&reference).unwrap().name, "explicit");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_v1_wallpaper_project_loads_through_the_store() {
        let dir = scratch("v1");
        let store = FileProjectStore::at(&dir);
        let reference = store.create_project("legacy").unwrap();
        let mut project = sample("legacy", 1.0);
        project.style.background = nss_core::Background::Wallpaper {
            name: "forest".to_owned(),
        };
        let mut json = serde_json::to_value(&project).unwrap();
        json["schemaVersion"] = serde_json::Value::from(1);
        fs::write(reference.project_file(), serde_json::to_vec(&json).unwrap()).unwrap();
        let loaded = store.load(&reference).unwrap();
        assert_eq!(
            loaded.style.background,
            nss_core::background_presets::migrated_wallpaper("forest")
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
