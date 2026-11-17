//! Most-recently-used list of bundles.
//!
//! Port of upstream `NSSProject/RecentProjects.swift`. Pure value type; the store persists it to
//! `.recent-projects.json` in the library root. The JSON keeps Swift's `Codable` key names
//! (`items`, `limit`, `path`, `name`, `lastOpened`) and the CoreFoundation epoch for the date, so
//! a recents file written by the Mac build loads here and vice versa (PLAN.md §7.3).

use nss_core::Timestamp;
use serde::{Deserialize, Serialize};

/// One entry: a bundle path, the project's display name at the time it was opened, and when.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentProject {
    pub path: String,
    pub name: String,
    pub last_opened: Timestamp,
}

impl RecentProject {
    #[must_use]
    pub fn new(path: impl Into<String>, name: impl Into<String>, last_opened: Timestamp) -> Self {
        Self {
            path: path.into(),
            name: name.into(),
            last_opened,
        }
    }
}

/// The MRU list itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecentProjects {
    pub items: Vec<RecentProject>,
    pub limit: usize,
}

impl Default for RecentProjects {
    fn default() -> Self {
        Self::with_limit(Self::DEFAULT_LIMIT)
    }
}

impl RecentProjects {
    pub const DEFAULT_LIMIT: usize = 20;

    #[must_use]
    pub fn with_limit(limit: usize) -> Self {
        Self {
            items: Vec::new(),
            limit,
        }
    }

    /// Moves (or adds) a project to the front and trims to `limit`.
    pub fn touch(&mut self, path: &str, name: &str, at: Timestamp) {
        self.items.retain(|i| i.path != path);
        self.items.insert(0, RecentProject::new(path, name, at));
        if self.items.len() > self.limit {
            self.items.truncate(self.limit);
        }
    }

    pub fn remove(&mut self, path: &str) {
        self.items.retain(|i| i.path != path);
    }

    /// Drops entries whose bundle no longer exists. Returns true if anything changed.
    pub fn prune(&mut self, exists: impl Fn(&str) -> bool) -> bool {
        let before = self.items.len();
        self.items.retain(|i| exists(&i.path));
        self.items.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_mru_capped_and_prunable() {
        // Mirrors upstream `recentsListIsMruCappedAndPruned`.
        let mut r = RecentProjects::with_limit(3);
        let d = Timestamp::ZERO;
        for n in ["a", "b", "c", "d"] {
            r.touch(&format!("/{n}"), n, d);
        }
        assert_eq!(
            r.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            ["d", "c", "b"]
        );
        r.touch("/b", "b", d);
        assert_eq!(
            r.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            ["b", "d", "c"]
        );
        assert!(r.prune(|p| p != "/d"));
        assert_eq!(
            r.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            ["b", "c"]
        );
        assert!(!r.prune(|_| true));
        r.remove("/b");
        assert_eq!(
            r.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            ["c"]
        );
    }

    #[test]
    fn touching_an_existing_entry_moves_it_without_duplicating() {
        let mut r = RecentProjects::default();
        r.touch("p", "one", Timestamp::ZERO);
        r.touch("q", "two", Timestamp::ZERO);
        r.touch("p", "one-renamed", Timestamp::ZERO);
        assert_eq!(r.items.len(), 2);
        assert_eq!(r.items[0].name, "one-renamed");
    }

    #[test]
    fn json_uses_swifts_codable_key_names() {
        let mut r = RecentProjects::default();
        r.touch("C:\\lib\\a.nssproj", "A", Timestamp(1.5));
        let text = serde_json::to_string(&r).unwrap();
        assert!(text.contains("\"lastOpened\""), "{text}");
        assert!(text.contains("\"items\""), "{text}");
        assert!(text.contains("\"limit\""), "{text}");
        assert!(!text.contains("last_opened"), "{text}");
        let back: RecentProjects = serde_json::from_str(&text).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn a_mac_authored_recents_file_loads() {
        // CoreFoundation epoch: 700000000 s is 2023-03-08T... in Unix terms.
        let text = r#"{
            "items": [
                {"path": "/Users/x/Videos/notscreenstudio/Demo.nssproj", "name": "Demo", "lastOpened": 700000000}
            ],
            "limit": 20
        }"#;
        let r: RecentProjects = serde_json::from_str(text).unwrap();
        assert_eq!(r.items.len(), 1);
        assert_eq!(r.items[0].name, "Demo");
        assert!(
            (r.items[0].last_opened.unix_secs() - (700_000_000.0 + 978_307_200.0)).abs() < 1e-6
        );
    }
}
