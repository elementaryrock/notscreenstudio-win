//! Upgrades an old `project.json` to the current schema before it is decoded.
//!
//! Port of upstream `NSSProject/ProjectMigrator.swift`. A migration step turns version N JSON
//! into version N+1 JSON; steps are registered by their source version. Steps are plain `fn`
//! pointers (not closures) so the registry is `const`-buildable and cannot capture state that
//! would make a migration depend on when it ran.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::errors::{ProjectError, ProjectMigrationError};

/// One migration step: rewrite the vN object into vN+1 in place.
pub type Step = fn(&mut Map<String, Value>) -> Result<(), ProjectMigrationError>;

/// Migrates `project.json` bytes forward to `current_version`.
pub struct ProjectMigrator {
    pub current_version: i32,
    steps: BTreeMap<i32, Step>,
}

impl std::fmt::Debug for ProjectMigrator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectMigrator")
            .field("current_version", &self.current_version)
            .field("steps", &self.steps.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Default for ProjectMigrator {
    fn default() -> Self {
        Self::standard()
    }
}

impl ProjectMigrator {
    #[must_use]
    pub fn new(current_version: i32, steps: impl IntoIterator<Item = (i32, Step)>) -> Self {
        Self {
            current_version,
            steps: steps.into_iter().collect(),
        }
    }

    /// The migrations this build ships: v1 → v2 replaces a legacy `.wallpaper(name:)` background
    /// with the equivalent `.mesh`. Add the next step as `(2, migrate_v2_whatever)`.
    #[must_use]
    pub fn standard() -> Self {
        Self::new(
            nss_core::Project::CURRENT_SCHEMA_VERSION,
            [(1, migrate_v1_wallpaper as Step)],
        )
    }

    /// Returns the input untouched when it is already current, otherwise JSON at
    /// `current_version`.
    pub fn migrate(&self, data: &[u8]) -> Result<Vec<u8>, ProjectError> {
        let parsed: Value = serde_json::from_slice(data)
            .map_err(|_| ProjectError::Corrupt("project.json is not valid JSON".to_owned()))?;
        let Value::Object(mut json) = parsed else {
            return Err(ProjectError::Corrupt(
                "project.json is not valid JSON".to_owned(),
            ));
        };

        // A missing version means v1, matching `Project`'s decoder. Non-integer values are
        // corrupt rather than silently treated as v1; compare in i64 to avoid truncation.
        let raw_version: i64 = match json.get("schemaVersion") {
            None | Some(Value::Null) => 1,
            Some(v) => {
                if let Some(n) = v.as_i64() {
                    n
                } else if v.as_u64().is_some() {
                    return Err(ProjectMigrationError::NewerSchema {
                        found: i32::MAX,
                        supported: self.current_version,
                    }
                    .into());
                } else {
                    return Err(ProjectError::Corrupt(
                        "project.json has a non-integer schemaVersion".to_owned(),
                    ));
                }
            }
        };
        if raw_version == i64::from(self.current_version) {
            return Ok(data.to_vec());
        }
        if raw_version > i64::from(self.current_version) {
            return Err(ProjectMigrationError::NewerSchema {
                found: i32::try_from(raw_version).unwrap_or(i32::MAX),
                supported: self.current_version,
            }
            .into());
        }
        let mut version = i32::try_from(raw_version)
            .map_err(|_| ProjectMigrationError::NoMigration { from: i32::MIN })?;
        while version < self.current_version {
            let Some(step) = self.steps.get(&version) else {
                return Err(ProjectMigrationError::NoMigration { from: version }.into());
            };
            step(&mut json)?;
            version += 1;
            json.insert("schemaVersion".to_owned(), Value::from(version));
        }
        // `Map` is a BTreeMap (sorted keys), which is what upstream asks JSONSerialization for.
        let out = serde_json::to_vec(&Value::Object(json))
            .map_err(|e| ProjectError::Corrupt(format!("cannot re-encode project.json: {e}")))?;
        Ok(out)
    }
}

/// v1 → v2: rewrites `style.background` = `{"wallpaper": {"name": n}}` to the equivalent
/// `{"mesh": ...}`. Anything else is left alone (decoding fills defaults for the new v2 fields).
fn migrate_v1_wallpaper(json: &mut Map<String, Value>) -> Result<(), ProjectMigrationError> {
    let Some(Value::Object(style)) = json.get_mut("style") else {
        return Ok(());
    };
    let Some(Value::Object(background)) = style.get("background") else {
        return Ok(());
    };
    let Some(Value::Object(wallpaper)) = background.get("wallpaper") else {
        return Ok(());
    };
    let name = wallpaper
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let migrated = nss_core::background_presets::migrated_wallpaper(&name);
    let encoded =
        serde_json::to_value(&migrated).map_err(|_| ProjectMigrationError::NewerSchema {
            found: 1,
            supported: 1,
        })?;
    style.insert("background".to_owned(), encoded);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nss_core::{Background, PixelSize, Project, RecordingAssets};

    fn sample(name: &str) -> Project {
        Project::new(
            name,
            RecordingAssets::new(PixelSize::new(100, 100), 60.0, 1.0),
        )
    }

    fn as_object(data: &[u8]) -> Map<String, Value> {
        match serde_json::from_slice(data).unwrap() {
            Value::Object(m) => m,
            other => panic!("expected an object, got {other}"),
        }
    }

    #[test]
    fn steps_run_in_order_and_the_version_is_stamped() {
        fn add_one(json: &mut Map<String, Value>) -> Result<(), ProjectMigrationError> {
            let name = json
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            json.insert("name".to_owned(), Value::from(format!("{name}+1")));
            Ok(())
        }
        fn add_two(json: &mut Map<String, Value>) -> Result<(), ProjectMigrationError> {
            let name = json
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            json.insert("name".to_owned(), Value::from(format!("{name}+2")));
            Ok(())
        }
        let m = ProjectMigrator::new(3, [(1, add_one as Step), (2, add_two as Step)]);
        let mut old = as_object(&serde_json::to_vec(&sample("p")).unwrap());
        old.insert("schemaVersion".to_owned(), Value::from(1));
        let out = m
            .migrate(&serde_json::to_vec(&Value::Object(old)).unwrap())
            .unwrap();
        let json = as_object(&out);
        assert_eq!(json["name"], Value::from("p+1+2"));
        assert_eq!(json["schemaVersion"], Value::from(3));
    }

    #[test]
    fn a_missing_version_is_v1_and_current_input_is_returned_byte_for_byte() {
        let current = ProjectMigrator::standard();
        let mut obj = as_object(&serde_json::to_vec(&sample("p")).unwrap());
        obj.remove("schemaVersion");
        let migrated = as_object(
            &current
                .migrate(&serde_json::to_vec(&Value::Object(obj.clone())).unwrap())
                .unwrap(),
        );
        assert_eq!(
            migrated["schemaVersion"],
            Value::from(Project::CURRENT_SCHEMA_VERSION),
            "a missing version is v1 and is migrated"
        );

        let mut at_current = obj;
        at_current.insert(
            "schemaVersion".to_owned(),
            Value::from(Project::CURRENT_SCHEMA_VERSION),
        );
        let data = serde_json::to_vec(&Value::Object(at_current)).unwrap();
        assert_eq!(current.migrate(&data).unwrap(), data);
    }

    #[test]
    fn a_newer_schema_is_rejected_and_a_gap_has_no_migration() {
        let current = ProjectMigrator::standard();
        let mut obj = as_object(&serde_json::to_vec(&sample("p")).unwrap());
        obj.insert(
            "schemaVersion".to_owned(),
            Value::from(Project::CURRENT_SCHEMA_VERSION + 1),
        );
        let err = current
            .migrate(&serde_json::to_vec(&Value::Object(obj)).unwrap())
            .unwrap_err();
        assert!(matches!(
            err,
            ProjectError::Migration(ProjectMigrationError::NewerSchema { .. })
        ));

        let gap = ProjectMigrator::new(3, []);
        let mut obj = as_object(&serde_json::to_vec(&sample("p")).unwrap());
        obj.insert("schemaVersion".to_owned(), Value::from(1));
        let err = gap
            .migrate(&serde_json::to_vec(&Value::Object(obj)).unwrap())
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            ProjectMigrationError::NoMigration { from: 1 }.to_string()
        );
    }

    #[test]
    fn garbage_is_reported_as_a_corrupt_project() {
        let current = ProjectMigrator::standard();
        assert!(matches!(
            current.migrate(b"{ truncated"),
            Err(ProjectError::Corrupt(_))
        ));
        assert!(matches!(
            current.migrate(b"[1, 2, 3]"),
            Err(ProjectError::Corrupt(_))
        ));
    }

    #[test]
    fn a_wallpaper_background_becomes_the_matching_mesh() {
        let mut p = sample("old");
        p.style.background = Background::Wallpaper {
            name: "forest".to_owned(),
        };
        let mut obj = as_object(&serde_json::to_vec(&p).unwrap());
        obj.insert("schemaVersion".to_owned(), Value::from(1));
        let out = ProjectMigrator::standard()
            .migrate(&serde_json::to_vec(&Value::Object(obj)).unwrap())
            .unwrap();
        let migrated: Project = serde_json::from_slice(&out).unwrap();
        assert_eq!(migrated.schema_version, Project::CURRENT_SCHEMA_VERSION);
        assert_eq!(
            migrated.style.background,
            nss_core::background_presets::migrated_wallpaper("forest")
        );
    }

    #[test]
    fn a_backgroundless_or_styeless_project_is_left_alone() {
        let mut obj = as_object(&serde_json::to_vec(&sample("p")).unwrap());
        obj.remove("style");
        obj.insert("schemaVersion".to_owned(), Value::from(1));
        let out = ProjectMigrator::standard()
            .migrate(&serde_json::to_vec(&Value::Object(obj)).unwrap())
            .unwrap();
        let migrated: Project = serde_json::from_slice(&out).unwrap();
        assert_eq!(
            migrated.style.background,
            nss_core::Style::default().background
        );
    }
}
