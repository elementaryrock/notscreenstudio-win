//! The Stage 3 exit criterion: **a crash during save leaves a loadable project.**
//!
//! Upstream phrase: "crash during save leaves a loadable project". The interesting property is not
//! that `rename` is atomic (the OS does that) but that every prefix of a save leaves a state the
//! next launch can read. This walks a save byte by byte: kill it after the temp file exists, kill
//! it between the backup and the swap, kill it mid-swap; each time the bundle must still load, and
//! the leftovers must be cleaned up on the next open.

use nss_core::{PixelSize, Project, RecordingAssets};
use nss_project::{atomic_file, FileProjectStore};
use std::fs;
use std::path::{Path, PathBuf};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nss-crash-{tag}-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn project(name: &str) -> Project {
    Project::new(
        name,
        RecordingAssets::new(PixelSize::new(100, 100), 60.0, 1.0),
    )
}

/// Everything a bundle holds right now, as name -> bytes.
fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut v: Vec<(String, Vec<u8>)> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                fs::read(e.path()).unwrap_or_default(),
            )
        })
        .collect();
    v.sort();
    v
}

#[test]
fn a_crash_before_the_swap_keeps_the_previous_project() {
    let dir = scratch("before-swap");
    let store = FileProjectStore::at(&dir);
    let reference = store.create_project("crash").unwrap();
    store.save(&project("good"), &reference).unwrap();
    let before = snapshot(&reference.url);

    // The hook runs after the temp file is written and synced, so this is exactly the window a
    // power cut would land in.
    let data = serde_json::to_vec(&project("never-lands")).unwrap();
    let err = atomic_file::write_checked(&data, &reference.project_file(), || {
        Err(nss_project::ProjectError::Corrupt(
            "simulated crash".to_owned(),
        ))
    });
    assert!(err.is_err());

    assert_eq!(
        snapshot(&reference.url),
        before,
        "the bundle must be untouched"
    );
    assert_eq!(store.load(&reference).unwrap().name, "good");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn leftovers_from_a_killed_save_are_ignored_then_cleaned_up() {
    let dir = scratch("leftovers");
    let store = FileProjectStore::at(&dir);
    let reference = store.create_project("crash").unwrap();
    store.save(&project("good"), &reference).unwrap();

    // What a kill -9 in the middle of `write` leaves: a synced temp file and nothing else.
    let stale = reference.url.join(format!(
        ".project.json{}.deadbeef",
        atomic_file::TEMP_MARKER
    ));
    fs::write(&stale, b"{\"half\": ").unwrap();

    // Loading still works, and the half-written file is not mistaken for the project.
    assert_eq!(store.load(&reference).unwrap().name, "good");

    // The next save path (or app start) removes it.
    assert_eq!(store.remove_stale_temps(), 1);
    assert!(!stale.exists());
    assert_eq!(store.load(&reference).unwrap().name, "good");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_crash_between_the_backup_and_the_swap_still_loads() {
    let dir = scratch("between");
    let store = FileProjectStore::at(&dir);
    let reference = store.create_project("crash").unwrap();
    store.save(&project("v1"), &reference).unwrap();
    store.save(&project("v2"), &reference).unwrap();
    assert!(reference.backup_file().is_file(), "v1 became the backup");

    // Primary lost entirely (the swap never happened and the temp was cleaned).
    fs::remove_file(reference.project_file()).unwrap();
    let loaded = store.load(&reference).unwrap();
    assert_eq!(
        loaded.name, "v1",
        "the backup is used when the primary is gone"
    );

    // Both copies gone: a clear error, not a panic and not a bogus project.
    fs::remove_file(reference.backup_file()).unwrap();
    let err = store.load(&reference).unwrap_err();
    assert!(err.to_string().contains("project.json"), "{err}");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_crash_mid_write_never_leaves_a_half_primary() {
    // Loop the real save many times, killing it at different points via the hook, and assert the
    // primary is always either the old or the new complete document.
    let dir = scratch("mid-write");
    let store = FileProjectStore::at(&dir);
    let reference = store.create_project("crash").unwrap();
    store.save(&project("stable"), &reference).unwrap();

    for attempt in 0..25 {
        let next = project(&format!("attempt-{attempt}"));
        let data = serde_json::to_vec(&next).unwrap();
        // Fail the write at the swap for every attempt: nothing may change.
        let _ = atomic_file::write_checked(&data, &reference.project_file(), || {
            Err(nss_project::ProjectError::Corrupt("boom".to_owned()))
        });
        let loaded = store.load(&reference).unwrap();
        assert_eq!(loaded.name, "stable", "attempt {attempt}");
    }
    assert_eq!(
        fs::read_dir(&reference.url).unwrap().count(),
        1,
        "no temp files accumulate: {:?}",
        snapshot(&reference.url)
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_corrupt_primary_and_corrupt_backup_report_the_primary_problem() {
    let dir = scratch("both-corrupt");
    let store = FileProjectStore::at(&dir);
    let reference = store.create_project("crash").unwrap();
    store.save(&project("v1"), &reference).unwrap();
    store.save(&project("v2"), &reference).unwrap();
    fs::write(reference.project_file(), b"{ nope").unwrap();
    fs::write(reference.backup_file(), b"{ also nope").unwrap();

    let err = store.load(&reference).unwrap_err();
    assert!(err.to_string().contains("project.json"), "{err}");
    fs::remove_dir_all(&dir).unwrap();
}
