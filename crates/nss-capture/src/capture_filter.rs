//! What to record and what to keep out of it.
//!
//! Port of upstream `NSSCapture/CaptureFilter.swift`. Upstream decided this on a hardware-free copy of
//! ScreenCaptureKit's `SCShareableContent`, because that type cannot be constructed in a test; this
//! port does the same with Windows facts (`EnumDisplayMonitors` / `EnumWindows` output), so the
//! decisions stay unit-tested.
//!
//! One thing works differently from macOS. ScreenCaptureKit can exclude a list of applications from a
//! display capture at filter-construction time. Windows Graphics Capture has no such list, so the
//! exclusion plan is applied by the recorder instead: every window it names gets
//! `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` before the session starts (ADR 0007), which
//! hides it from the capture while still letting the user see and drag it. The plan itself — "which of
//! the windows on screen are ours" — is the part worth testing, and it is the same decision upstream
//! made.

use nss_core::{CaptureSource, NssError, Rect};

use crate::capture_math::is_own_app;

/// Hardware-free copy of the parts of the shareable content that the plan needs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContentSnapshot {
    /// Monitor handles, as reported by the display enumeration.
    pub display_ids: Vec<u32>,
    pub apps: Vec<App>,
    pub windows: Vec<Window>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub process_id: u32,
    /// Executable file name, the closest Windows equivalent of a bundle ID.
    pub app_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// `HWND` as a plain integer.
    pub handle: u64,
    pub owner_process_id: Option<u32>,
    pub owner_app: Option<String>,
    pub title: String,
    pub is_on_screen: bool,
    pub is_cloaked: bool,
    pub is_tool: bool,
}

impl Window {
    /// A plain top-level window seen by the enumeration.
    #[must_use]
    pub fn new(handle: u64, owner_process_id: Option<u32>) -> Self {
        Self {
            handle,
            owner_process_id,
            owner_app: None,
            title: String::new(),
            is_on_screen: true,
            is_cloaked: false,
            is_tool: false,
        }
    }
}

/// What to keep out of a display or area capture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExclusionPlan {
    /// Processes whose windows (present and future) are excluded. Preferred, because it also covers
    /// windows created after the plan was built.
    pub process_ids: Vec<u32>,
    /// Individual windows. Used for a window no owning process could be resolved for, and as a
    /// fallback for windows the app knows about itself.
    pub window_handles: Vec<u64>,
}

impl ExclusionPlan {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.process_ids.is_empty() && self.window_handles.is_empty()
    }
}

/// What to record: a whole display, a sub-rectangle of one, or a single window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Display(u32),
    Window(u64),
}

/// The decision for one capture.
#[derive(Debug, Clone, PartialEq)]
pub struct FilterPlan {
    pub target: Target,
    /// Sub-rectangle of the display in points (area capture).
    pub area: Option<Rect>,
    pub is_window: bool,
    pub exclusion: ExclusionPlan,
}

/// Which apps and windows belong to this process.
///
/// Matches by process ID and by executable name, and adds any window handle the app reports for
/// itself, which covers a window whose owner could not be resolved (a layered or child window).
#[must_use]
pub fn exclusion(
    snapshot: &ContentSnapshot,
    own_process_id: u32,
    own_app_name: Option<&str>,
    own_window_handles: &[u64],
) -> ExclusionPlan {
    let is_own = |pid: Option<u32>, app: Option<&str>| {
        is_own_app(pid.unwrap_or(u32::MAX), app, own_process_id, own_app_name)
    };
    let mut process_ids: Vec<u32> = snapshot
        .apps
        .iter()
        .filter(|a| is_own(Some(a.process_id), a.app_name.as_deref()))
        .map(|a| a.process_id)
        .collect();
    let mut window_handles: Vec<u64> = Vec::new();
    for w in &snapshot.windows {
        if is_own(w.owner_process_id, w.owner_app.as_deref()) {
            window_handles.push(w.handle);
            // A process with no window of its own yet can still be identified through its windows.
            if let Some(pid) = w.owner_process_id {
                process_ids.push(pid);
            }
        }
    }
    let known: Vec<u64> = snapshot.windows.iter().map(|w| w.handle).collect();
    for handle in own_window_handles {
        if known.contains(handle) && !window_handles.contains(handle) {
            window_handles.push(*handle);
        }
    }
    process_ids.sort_unstable();
    process_ids.dedup();
    window_handles.sort_unstable();
    ExclusionPlan {
        process_ids,
        window_handles,
    }
}

/// Builds the plan for a source, or a friendly error when the source has gone away.
pub fn plan(
    source: CaptureSource,
    snapshot: &ContentSnapshot,
    own_process_id: u32,
    own_app_name: Option<&str>,
    own_window_handles: &[u64],
) -> Result<FilterPlan, NssError> {
    match source {
        CaptureSource::Display { id: display_id } => {
            if !snapshot.display_ids.contains(&display_id) {
                return Err(NssError::Unsupported(
                    "The selected display is no longer available.".into(),
                ));
            }
            Ok(FilterPlan {
                target: Target::Display(display_id),
                area: None,
                is_window: false,
                exclusion: exclusion(snapshot, own_process_id, own_app_name, own_window_handles),
            })
        }
        CaptureSource::Area {
            display_id, rect, ..
        } => {
            if !snapshot.display_ids.contains(&display_id) {
                return Err(NssError::Unsupported(
                    "The selected display is no longer available.".into(),
                ));
            }
            Ok(FilterPlan {
                target: Target::Display(display_id),
                area: Some(rect),
                is_window: false,
                exclusion: exclusion(snapshot, own_process_id, own_app_name, own_window_handles),
            })
        }
        CaptureSource::Window { id: window_id } => {
            if !snapshot
                .windows
                .iter()
                .any(|w| w.handle == u64::from(window_id))
            {
                return Err(NssError::Io("The selected window was closed.".into()));
            }
            // A single-window capture shows only that window, so nothing else can leak in.
            Ok(FilterPlan {
                target: Target::Window(u64::from(window_id)),
                area: None,
                is_window: true,
                exclusion: ExclusionPlan::default(),
            })
        }
    }
}

/// `NSS_CAPTURE_DEBUG=1` prints what the capture excluded, to stderr, so it can be checked by eye.
pub fn debug_enabled() -> bool {
    std::env::var("NSS_CAPTURE_DEBUG").is_ok_and(|v| v == "1")
}

/// Prints the plan when [`debug_enabled`], so a leaking HUD can be diagnosed without a debugger.
pub fn log_plan(
    plan: &FilterPlan,
    snapshot: &ContentSnapshot,
    own_process_id: u32,
    built_from: &str,
) {
    if debug_enabled() {
        eprintln!("{}", describe(plan, snapshot, own_process_id, built_from));
    }
}

/// One readable line per fact: what was excluded and why.
#[must_use]
pub fn describe(
    plan: &FilterPlan,
    snapshot: &ContentSnapshot,
    own_process_id: u32,
    filter_built_from: &str,
) -> String {
    let mut lines = vec![format!(
        "[nss capture] filter ({filter_built_from}) target={:?} ownPID={own_process_id}",
        plan.target
    )];
    let ex = &plan.exclusion;
    if plan.is_window {
        lines.push("  single-window capture: nothing else is visible, no exclusion needed".into());
    } else if ex.is_empty() {
        lines.push(
            "  WARNING: nothing excluded. This app's windows WILL appear in the recording.".into(),
        );
    } else {
        lines.push(format!("  excluded apps (pid): {:?}", ex.process_ids));
        lines.push(format!(
            "  own windows seen by the window enumeration ({}): {}",
            ex.window_handles.len(),
            ex.window_handles
                .iter()
                .map(|h| {
                    let title = snapshot
                        .windows
                        .iter()
                        .find(|w| w.handle == *h)
                        .map_or("", |w| w.title.as_str());
                    format!("{h}[{title}]")
                })
                .collect::<Vec<_>>()
                .join(", ")
        ));
        if ex.process_ids.is_empty() {
            lines.push("  no owning process found; excluding by window handle only".into());
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN_PID: u32 = 500;
    const OWN_APP: &str = "notscreenstudio.exe";

    fn snapshot(apps: Vec<App>, windows: Vec<Window>, displays: Vec<u32>) -> ContentSnapshot {
        ContentSnapshot {
            display_ids: displays,
            apps,
            windows,
        }
    }

    fn app(pid: u32, name: &str) -> App {
        App {
            process_id: pid,
            app_name: Some(name.to_owned()),
        }
    }

    fn window(handle: u64, pid: u32, title: &str) -> Window {
        let owner_app = if pid == OWN_PID {
            OWN_APP.to_owned()
        } else {
            "explorer.exe".to_owned()
        };
        Window {
            title: title.to_owned(),
            owner_app: Some(owner_app),
            ..Window::new(handle, Some(pid))
        }
    }

    fn plan_for(snap: &ContentSnapshot, source: CaptureSource, own: &[u64]) -> FilterPlan {
        plan(source, snap, OWN_PID, Some(OWN_APP), own).expect("plan")
    }

    #[test]
    fn our_app_listed_by_the_enumeration_is_excluded_by_pid() {
        let snap = snapshot(
            vec![app(10, "explorer.exe"), app(OWN_PID, OWN_APP)],
            vec![],
            vec![1, 2],
        );
        let p = plan_for(&snap, CaptureSource::Display { id: 1 }, &[]);
        assert_eq!(p.exclusion.process_ids, [OWN_PID]);
        assert_eq!(p.target, Target::Display(1));
    }

    #[test]
    fn the_app_name_matches_even_when_the_pid_differs() {
        let snap = snapshot(vec![app(777, "NotScreenStudio.EXE")], vec![], vec![1]);
        let p = plan_for(&snap, CaptureSource::Display { id: 1 }, &[]);
        assert_eq!(p.exclusion.process_ids, [777]);
    }

    #[test]
    fn an_app_missing_from_the_list_is_found_through_its_windows() {
        let snap = snapshot(
            vec![app(10, "explorer.exe")],
            vec![
                window(90, 10, "File Explorer"),
                window(91, OWN_PID, "HUD"),
                window(92, OWN_PID, "Bubble"),
            ],
            vec![1],
        );
        let p = plan_for(&snap, CaptureSource::Display { id: 1 }, &[]);
        assert_eq!(p.exclusion.process_ids, [OWN_PID]);
        assert_eq!(p.exclusion.window_handles, [91, 92]);
        assert!(!p.exclusion.is_empty());
    }

    #[test]
    fn window_handles_from_the_app_are_a_fallback_when_no_owner_is_reported() {
        let snap = snapshot(
            vec![],
            vec![Window::new(90, None), Window::new(91, None)],
            vec![1],
        );
        let p = plan_for(&snap, CaptureSource::Display { id: 1 }, &[91, 4242]);
        assert!(p.exclusion.process_ids.is_empty());
        assert_eq!(
            p.exclusion.window_handles,
            [91],
            "4242 unknown, 90 not ours"
        );
    }

    #[test]
    fn a_reported_handle_is_not_duplicated() {
        let snap = snapshot(vec![], vec![window(91, OWN_PID, "HUD")], vec![1]);
        let p = plan_for(&snap, CaptureSource::Display { id: 1 }, &[91]);
        assert_eq!(p.exclusion.window_handles, [91]);
    }

    #[test]
    fn other_apps_are_never_excluded() {
        let snap = snapshot(
            vec![app(10, "explorer.exe")],
            vec![Window {
                owner_app: Some("explorer.exe".into()),
                ..Window::new(90, Some(10))
            }],
            vec![1],
        );
        assert!(plan_for(&snap, CaptureSource::Display { id: 1 }, &[])
            .exclusion
            .is_empty());
    }

    #[test]
    fn an_empty_app_name_does_not_match() {
        let snap = snapshot(
            vec![App {
                process_id: 11,
                app_name: Some(String::new()),
            }],
            vec![],
            vec![1],
        );
        let p = plan(
            CaptureSource::Display { id: 1 },
            &snap,
            OWN_PID,
            Some(""),
            &[],
        )
        .expect("plan");
        assert!(p.exclusion.is_empty());
    }

    #[test]
    fn area_capture_keeps_the_rect_and_excludes_our_windows() {
        let rect = Rect::new(10.0, 20.0, 300.0, 200.0);
        let snap = snapshot(vec![app(OWN_PID, OWN_APP)], vec![], vec![1, 2]);
        let p = plan_for(
            &snap,
            CaptureSource::Area {
                display_id: 2,
                rect,
            },
            &[],
        );
        assert_eq!(p.target, Target::Display(2));
        assert_eq!(p.area, Some(rect));
        assert!(!p.is_window);
        assert_eq!(p.exclusion.process_ids, [OWN_PID]);
    }

    #[test]
    fn a_zero_area_still_targets_the_display() {
        let snap = snapshot(vec![], vec![], vec![2]);
        let p = plan_for(
            &snap,
            CaptureSource::Area {
                display_id: 2,
                rect: Rect::ZERO,
            },
            &[],
        );
        assert_eq!(p.target, Target::Display(2));
        assert_eq!(p.area, Some(Rect::ZERO));
    }

    #[test]
    fn window_capture_needs_no_exclusion() {
        let snap = snapshot(vec![], vec![window(90, 10, "Editor")], vec![1]);
        let p = plan_for(&snap, CaptureSource::Window { id: 90 }, &[]);
        assert_eq!(p.target, Target::Window(90));
        assert!(p.is_window);
        assert!(p.exclusion.is_empty());
    }

    #[test]
    fn missing_sources_get_friendly_errors() {
        let snap = snapshot(vec![], vec![], vec![1]);
        let display = plan(
            CaptureSource::Display { id: 9 },
            &snap,
            OWN_PID,
            Some(OWN_APP),
            &[],
        );
        assert!(matches!(display, Err(NssError::Unsupported(_))));
        let area = plan(
            CaptureSource::Area {
                display_id: 9,
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
            },
            &snap,
            OWN_PID,
            Some(OWN_APP),
            &[],
        );
        assert!(matches!(area, Err(NssError::Unsupported(_))));
        let window = plan(
            CaptureSource::Window { id: 9 },
            &snap,
            OWN_PID,
            Some(OWN_APP),
            &[],
        );
        assert!(matches!(window, Err(NssError::Io(_))));
    }

    #[test]
    fn a_later_refresh_picks_up_windows_created_after_the_first_plan() {
        let before = snapshot(vec![app(10, "explorer.exe")], vec![], vec![1]);
        assert!(plan_for(&before, CaptureSource::Display { id: 1 }, &[])
            .exclusion
            .is_empty());
        let after = snapshot(
            vec![app(10, "explorer.exe")],
            vec![window(91, OWN_PID, "HUD")],
            vec![1],
        );
        assert_eq!(
            plan_for(&after, CaptureSource::Display { id: 1 }, &[]).exclusion,
            ExclusionPlan {
                process_ids: vec![OWN_PID],
                window_handles: vec![91],
            }
        );
    }

    #[test]
    fn the_debug_text_lists_what_was_excluded() {
        let snap = snapshot(vec![], vec![window(91, OWN_PID, "HUD")], vec![1]);
        let p = plan_for(&snap, CaptureSource::Display { id: 1 }, &[]);
        let text = describe(&p, &snap, OWN_PID, "initial");
        assert!(text.contains("excluded apps (pid): [500]"), "{text}");
        assert!(text.contains("91[HUD]"), "{text}");
    }

    #[test]
    fn the_debug_text_warns_when_nothing_is_excluded() {
        let snap = snapshot(vec![], vec![], vec![1]);
        let p = plan_for(&snap, CaptureSource::Display { id: 1 }, &[]);
        assert!(describe(&p, &snap, OWN_PID, "initial").contains("WARNING"));
    }
}
