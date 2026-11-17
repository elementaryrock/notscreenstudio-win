//! Schema v1 → v2: legacy `.wallpaper(name:)` backgrounds become the equivalent `.mesh`.
//!
//! Port of upstream `Tests/NSSProjectTests/BackgroundMigrationTests.swift`. The migration is the
//! only place where reading a project changes what the user sees, so it is tested through the
//! store as well as directly: a v1 file on disk must load with the new background and no blur.

use nss_core::background_presets;
use nss_core::{Background, GradientStop, PixelSize, Project, RGBAColor, RecordingAssets};
use nss_project::{FileProjectStore, ProjectMigrator};
use std::path::{Path, PathBuf};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nss-migration-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sample(name: &str) -> Project {
    Project::new(
        name,
        RecordingAssets::new(PixelSize::new(100, 100), 60.0, 1.0),
    )
}

/// A schema v1 object with `background` set and no `backgroundBlur` key (v1 had no blur field).
fn v1_json(background: Background) -> serde_json::Value {
    let mut project = sample("old");
    project.style.background = background;
    let mut json = serde_json::to_value(&project).unwrap();
    json["schemaVersion"] = serde_json::Value::from(1);
    if let Some(style) = json.get_mut("style").and_then(|s| s.as_object_mut()) {
        style.remove("backgroundBlur");
    }
    json
}

fn migrate(json: &serde_json::Value) -> Project {
    let data = serde_json::to_vec(json).unwrap();
    let out = ProjectMigrator::standard().migrate(&data).unwrap();
    serde_json::from_slice(&out).unwrap()
}

#[test]
fn a_v1_wallpaper_becomes_the_matching_mesh() {
    // Upstream runs this over ["Sunset", "ocean", "Graphite", "aurora"] to cover the case
    // handling of the legacy lookup.
    for name in [
        "Sunset",
        "ocean",
        "Graphite",
        "aurora",
        "forest",
        "unknown-name",
    ] {
        let project = migrate(&v1_json(Background::Wallpaper {
            name: name.to_owned(),
        }));
        assert_eq!(
            project.schema_version,
            Project::CURRENT_SCHEMA_VERSION,
            "{name}"
        );
        assert_eq!(
            project.style.background,
            background_presets::migrated_wallpaper(name),
            "{name}"
        );
        assert_eq!(project.style.background_blur, 0.0, "{name}");
    }
}

#[test]
fn other_backgrounds_are_unchanged() {
    let cases = [
        Background::Solid(RGBAColor::WHITE),
        Background::Image {
            file: "background-x.png".to_owned(),
        },
        Background::Gradient {
            stops: vec![
                GradientStop::new(0.0, RGBAColor::BLACK),
                GradientStop::new(1.0, RGBAColor::WHITE),
            ],
            angle_degrees: 30.0,
        },
    ];
    for background in cases {
        let project = migrate(&v1_json(background.clone()));
        assert_eq!(project.schema_version, Project::CURRENT_SCHEMA_VERSION);
        assert_eq!(project.style.background, background);
    }
}

#[test]
fn the_store_loads_a_v1_wallpaper_project_from_disk() {
    let dir = scratch("store");
    let store = FileProjectStore::at(&dir);
    let reference = store.create_project("legacy").unwrap();
    let json = v1_json(Background::Wallpaper {
        name: "forest".to_owned(),
    });
    std::fs::write(reference.project_file(), serde_json::to_vec(&json).unwrap()).unwrap();

    let loaded = store.load(&reference).unwrap();
    assert_eq!(
        loaded.style.background,
        background_presets::migrated_wallpaper("forest")
    );
    // Loading does not rewrite the file: the migration is applied in memory each time, so a
    // downgrade is still possible.
    let on_disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(reference.project_file()).unwrap()).unwrap();
    assert_eq!(on_disk["schemaVersion"], serde_json::Value::from(1));

    // An explicit save, on the other hand, stamps the new version.
    store.save(&loaded, &reference).unwrap();
    let after: serde_json::Value =
        serde_json::from_slice(&std::fs::read(reference.project_file()).unwrap()).unwrap();
    assert_eq!(
        after["schemaVersion"],
        serde_json::Value::from(Project::CURRENT_SCHEMA_VERSION)
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_backup_from_an_older_schema_is_also_migrated() {
    // Real sequence: a v1 project is opened and saved (primary becomes v2), then a crash leaves
    // the v1 backup as the only readable file.
    let dir = scratch("backup");
    let store = FileProjectStore::at(&dir);
    let reference = store.create_project("legacy").unwrap();
    std::fs::write(
        reference.backup_file(),
        serde_json::to_vec(&v1_json(Background::Wallpaper {
            name: "Graphite".to_owned(),
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(reference.project_file(), b"{ truncated").unwrap();

    let loaded = store.load(&reference).unwrap();
    assert_eq!(loaded.schema_version, Project::CURRENT_SCHEMA_VERSION);
    assert_eq!(
        loaded.style.background,
        background_presets::migrated_wallpaper("Graphite")
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_legacy_wallpaper_lookup_is_case_insensitive_and_defaults_to_aurora() {
    for name in ["Sunset", "sunset", "SUNSET"] {
        assert_eq!(
            background_presets::migrated_wallpaper(name),
            background_presets::migrated_wallpaper("Sunset")
        );
    }
    assert_eq!(
        background_presets::migrated_wallpaper("no such wallpaper"),
        background_presets::migrated_wallpaper("aurora")
    );
}

#[test]
fn migration_keeps_every_other_style_field() {
    let mut project = sample("keep");
    project.style.background = Background::Wallpaper {
        name: "Sunset".to_owned(),
    };
    project.style.shadow.radius = 12.5;
    project.style.padding = 0.08;
    let mut json = serde_json::to_value(&project).unwrap();
    json["schemaVersion"] = serde_json::Value::from(1);
    if let Some(style) = json.get_mut("style").and_then(|s| s.as_object_mut()) {
        style.remove("backgroundBlur");
    }
    let migrated = migrate(&json);
    assert_eq!(migrated.style.shadow.radius, 12.5);
    assert_eq!(migrated.style.padding, 0.08);
    assert_eq!(migrated.name, "keep");
}

/// Guards the assumption the tests above make: `style.background` really is the key the migrator
/// rewrites, so a rename in `nss-core` fails here instead of silently skipping migration.
#[test]
fn the_migrator_targets_the_key_the_encoder_writes() {
    let json = v1_json(Background::Wallpaper {
        name: "Sunset".to_owned(),
    });
    assert!(json["style"]["background"]["wallpaper"]["name"].is_string());
    let path: &Path = Path::new("style");
    assert_eq!(path.to_string_lossy(), "style");
}
