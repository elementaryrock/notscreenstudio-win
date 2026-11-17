//! Wire-format compatibility tests.
//!
//! `project.json` and `cursor.json` are shared with the macOS build, so their key names and
//! defaults are a published contract (PLAN.md §7.3). These tests pin the exact JSON the
//! Windows build writes, and check that a project written by the Mac build still loads.

use nss_core::{
    AspectRatio, Background, CameraBubble, CameraShape, CaptureSource, CursorTrack, MediaTime,
    PixelSize, Project, RGBAColor, RecordingAssets, Rect, Style, Timeline, Timestamp, ZoomBlock,
    ZoomFocus,
};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use uuid::Uuid;

/// Fixed identity so the golden bytes below are stable.
const TEST_UUID: Uuid = Uuid::from_u128(0x7f0c3a9e_2b7e_4c55_9d1d_0d4b7b8e2f11);
const REFERENCE_DATE: f64 = 978_307_200.0; // 2001-01-01T00:00:00Z, upstream's epoch

fn golden_project() -> Project {
    let assets = RecordingAssets {
        camera_file: Some("camera.mov".into()),
        mic_file: Some("mic.m4a".into()),
        start_host_time: 1.5,
        thumbnail_file: Some("thumb.png".into()),
        ..RecordingAssets::new(PixelSize::new(2560, 1440), 60.0, 12.5)
    };
    let mut p = Project::new("Demo", assets);
    p.id = TEST_UUID;
    p.created_at = Timestamp(1_700_000_000.0);
    p.modified_at = p.created_at;
    p.timeline = Timeline {
        trim: nss_core::TrimRange::new(0.5, Some(11.0)),
        zoom_blocks: vec![ZoomBlock {
            id: TEST_UUID,
            focus: ZoomFocus::Manual(nss_core::NormalizedPoint::new(0.3, 0.7)),
            is_auto: true,
            ease_in: 0.4,
            ease_out: 0.9,
            ..ZoomBlock::new(1.0, 3.0)
        }],
        auto_zoom_enabled: false,
    };
    p.style = Style {
        background: Background::Solid(RGBAColor::new(0.1, 0.2, 0.3, 1.0)),
        padding: 0.08,
        corner_radius: 0.02,
        shadow: nss_core::Shadow {
            radius: 0.04,
            opacity: 0.5,
            offset_x: 0.001,
            offset_y: 0.013,
        },
        aspect_ratio: AspectRatio::R9x16,
        cursor: nss_core::CursorStyle {
            size: 2.0,
            smoothing: 0.3,
            motion_blur: 0.6,
            click_ripples: false,
            hide_when_idle: true,
        },
        camera: CameraBubble {
            shape: CameraShape::Wide,
            roundness: 0.35,
            ..CameraBubble::default()
        },
        background_blur: 0.25,
    };
    p
}

#[test]
fn project_json_is_byte_identical_to_the_documented_format() {
    let text = serde_json::to_string_pretty(&golden_project()).unwrap();
    let expected = json!({
        "id": "7F0C3A9E-2B7E-4C55-9D1D-0D4B7B8E2F11",
        "name": "Demo",
        "createdAt": 1_700_000_000.0,
        "modifiedAt": 1_700_000_000.0,
        "assets": {
            "screenFile": "screen.mov",
            "cameraFile": "camera.mov",
            "micFile": "mic.m4a",
            "cursorFile": "cursor.json",
            "sourcePixelSize": { "width": 2560, "height": 1440 },
            "frameRate": 60.0,
            "duration": 12.5,
            "startHostTime": 1.5,
            "thumbnailFile": "thumb.png"
        },
        "timeline": {
            "trim": { "start": 0.5, "end": 11.0 },
            "zoomBlocks": [{
                "id": "7F0C3A9E-2B7E-4C55-9D1D-0D4B7B8E2F11",
                "start": 1.0,
                "end": 3.0,
                "scale": 2.0,
                "focus": { "manual": { "x": 0.3, "y": 0.7 } },
                "isAuto": true,
                "easeIn": 0.4,
                "easeOut": 0.9
            }],
            "autoZoomEnabled": false
        },
        "style": {
            "background": { "solid": { "r": 0.1, "g": 0.2, "b": 0.3, "a": 1.0 } },
            "padding": 0.08,
            "cornerRadius": 0.02,
            "shadow": { "radius": 0.04, "opacity": 0.5, "offsetX": 0.001, "offsetY": 0.013 },
            "aspectRatio": "r9x16",
            "cursor": {
                "size": 2.0,
                "smoothing": 0.3,
                "motionBlur": 0.6,
                "clickRipples": false,
                "hideWhenIdle": true
            },
            "camera": {
                "visible": true,
                "shape": "wide",
                "position": { "x": 0.88, "y": 0.82 },
                "size": 0.18,
                "border": { "width": 0.004, "color": { "r": 1.0, "g": 1.0, "b": 1.0, "a": 1.0 } },
                "mirror": true,
                "roundness": 0.35,
                "zoomedScale": 0.6
            },
            "backgroundBlur": 0.25
        },
        "schemaVersion": 2
    });
    let got: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(got, expected, "\n{text}");
}

#[test]
fn every_project_key_is_known() {
    // Guards against an accidental rename in the struct: the set of keys must equal the
    // documented set exactly, both ways.
    let v: Value = serde_json::to_value(golden_project()).unwrap();
    let keys = |v: &Value| -> BTreeSet<String> { v.as_object().unwrap().keys().cloned().collect() };
    let set =
        |names: &[&str]| -> BTreeSet<String> { names.iter().map(|s| (*s).to_owned()).collect() };

    assert_eq!(
        keys(&v),
        set(&[
            "id",
            "name",
            "createdAt",
            "modifiedAt",
            "assets",
            "timeline",
            "style",
            "schemaVersion"
        ])
    );
    assert_eq!(
        keys(&v["assets"]),
        set(&[
            "screenFile",
            "cameraFile",
            "micFile",
            "cursorFile",
            "sourcePixelSize",
            "frameRate",
            "duration",
            "startHostTime",
            "thumbnailFile"
        ])
    );
    assert_eq!(
        keys(&v["timeline"]),
        set(&["trim", "zoomBlocks", "autoZoomEnabled"])
    );
    assert_eq!(
        keys(&v["style"]),
        set(&[
            "background",
            "padding",
            "cornerRadius",
            "shadow",
            "aspectRatio",
            "cursor",
            "camera",
            "backgroundBlur"
        ])
    );
    assert_eq!(
        keys(&v["style"]["cursor"]),
        set(&[
            "size",
            "smoothing",
            "motionBlur",
            "clickRipples",
            "hideWhenIdle"
        ])
    );
    assert_eq!(
        keys(&v["style"]["camera"]),
        set(&[
            "visible",
            "shape",
            "position",
            "size",
            "border",
            "mirror",
            "roundness",
            "zoomedScale"
        ])
    );
    assert_eq!(
        keys(&v["style"]["shadow"]),
        set(&["radius", "opacity", "offsetX", "offsetY"])
    );
    assert_eq!(
        keys(&v["timeline"]["zoomBlocks"][0]),
        set(&["id", "start", "end", "scale", "focus", "isAuto", "easeIn", "easeOut"])
    );
}

#[test]
fn a_project_written_by_the_mac_build_loads() {
    // A v1-era document: legacy `wallpaper` background, Round 1 bubble shape, no
    // `schemaVersion`, no `backgroundBlur`, no cursor shapes. Everything else is what the
    // Mac encoder actually emits.
    let text = r#"{
      "id": "A1B2C3D4-E5F6-4A7B-8C9D-0E1F2A3B4C5D",
      "name": "Mac recording",
      "createdAt": 780000000.5,
      "modifiedAt": 780000123.25,
      "assets": {
        "screenFile": "screen.mov",
        "cameraFile": null,
        "micFile": null,
        "cursorFile": "cursor.json",
        "sourcePixelSize": { "width": 3024, "height": 1964 },
        "frameRate": 60,
        "duration": 42.25
      },
      "timeline": {
        "trim": { "start": 0, "end": null },
        "zoomBlocks": [
          { "id": "11111111-2222-3333-4444-555555555555", "start": 2, "end": 4, "scale": 2, "focus": { "auto": {} } }
        ]
      },
      "style": {
        "background": { "wallpaper": { "name": "ocean" } },
        "padding": 0.05,
        "cornerRadius": 0.02,
        "shadow": { "radius": 0.03, "opacity": 0.4, "offsetX": 0, "offsetY": 0.012 },
        "aspectRatio": "r16x9",
        "cursor": { "size": 1.5, "smoothing": 0.5, "motionBlur": 0.4, "clickRipples": true, "hideWhenIdle": false },
        "camera": { "visible": false, "shape": { "roundedRect": { "radius": 0.2 } }, "position": { "x": 0.9, "y": 0.8 }, "size": 0.2, "mirror": true }
      }
    }"#;
    let p: Project = serde_json::from_str(text).unwrap();

    assert_eq!(p.name, "Mac recording");
    assert_eq!(
        p.id,
        Uuid::parse_str("A1B2C3D4-E5F6-4A7B-8C9D-0E1F2A3B4C5D").unwrap()
    );
    assert_eq!(p.created_at.0, 780_000_000.5);
    assert_eq!(p.created_at.unix_secs(), 1_758_307_200.5);
    assert_eq!(p.assets.source_pixel_size, PixelSize::new(3024, 1964));
    assert_eq!(p.assets.duration, 42.25);
    assert_eq!(
        p.assets.start_host_time, 0.0,
        "a v1 file has no startHostTime"
    );
    assert_eq!(p.assets.thumbnail_file, None);
    assert!(p.timeline.auto_zoom_enabled, "the key was only added later");
    assert_eq!(p.timeline.zoom_blocks.len(), 1);
    assert_eq!(p.timeline.zoom_blocks[0].focus, ZoomFocus::Auto {});
    assert_eq!(p.timeline.zoom_blocks[0].ease_in, ZoomBlock::DEFAULT_EASE);

    assert_eq!(
        p.style.background,
        Background::Wallpaper {
            name: "ocean".into()
        }
    );
    assert_eq!(p.style.background_blur, 0.0);
    assert!(!p.style.camera.visible);
    // Round 1 `roundedRect(radius: 0.2)` migrates to square with roundness 0.4.
    assert_eq!(p.style.camera.shape, CameraShape::Square);
    assert!((p.style.camera.roundness - 0.4).abs() < 1e-9);
    assert!(
        p.style.camera.border.is_none(),
        "a missing border key is 'no border'"
    );
    assert_eq!(p.style.camera.zoomed_scale, 0.6, "Round 2 default");

    assert_eq!(p.schema_version, Project::CURRENT_SCHEMA_VERSION);

    // Saving it back writes a complete, current document that still loads.
    let again: Project = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert_eq!(again, p);
}

#[test]
fn timeline_json_keeps_swift_key_names() {
    // `autoZoomEnabled` is camelCase even though the Rust field is snake_case, and
    // `zoomBlocks` must not become `zoom_blocks`.
    let text = serde_json::to_string(&Timeline::default()).unwrap();
    assert!(text.contains("\"zoomBlocks\""), "{text}");
    assert!(text.contains("\"autoZoomEnabled\""), "{text}");
    assert!(!text.contains("zoom_blocks"), "{text}");
    assert!(!text.contains("auto_zoom"), "{text}");
}

#[test]
fn cursor_json_keeps_swift_key_names() {
    let mut t = CursorTrack::default();
    t.samples.push(nss_core::CursorSample::new(0.0, 0.1, 0.2));
    t.clicks.push(nss_core::ClickEvent::new(
        0.5,
        0.1,
        0.2,
        nss_core::MouseButton::Left,
        nss_core::ClickKind::Down,
    ));
    t.shapes.push(nss_core::CursorShapeChange::new(
        0.25,
        nss_core::CursorShape::PointingHand,
    ));
    let text = serde_json::to_string(&t).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        v,
        json!({
            "samples": [{ "t": 0.0, "x": 0.1, "y": 0.2 }],
            "clicks": [{ "t": 0.5, "x": 0.1, "y": 0.2, "button": "left", "kind": "down" }],
            "sampleRate": 120.0,
            "shapes": [{ "t": 0.25, "shape": "pointingHand" }]
        })
    );
}

#[test]
fn capture_source_json_uses_swift_ids() {
    let area = CaptureSource::Area {
        display_id: 2,
        rect: Rect::new(0.0, 0.0, 100.0, 50.0),
    };
    let v: Value = serde_json::to_value(area).unwrap();
    assert_eq!(
        v,
        json!({
            "area": {
                "displayID": 2,
                "rect": {
                    "origin": { "x": 0.0, "y": 0.0 },
                    "size": { "width": 100.0, "height": 50.0 }
                }
            }
        })
    );
    let back: CaptureSource = serde_json::from_value(v).unwrap();
    assert_eq!(back, area);
}

#[test]
fn background_variants_use_swift_external_tags() {
    let cases = [
        (
            Background::Gradient {
                stops: vec![nss_core::GradientStop::new(0.0, RGBAColor::WHITE)],
                angle_degrees: 45.0,
            },
            json!({ "gradient": { "stops": [{ "location": 0.0, "color": { "r": 1.0, "g": 1.0, "b": 1.0, "a": 1.0 } }], "angleDegrees": 45.0 } }),
        ),
        (
            Background::Image {
                file: "bg.png".into(),
            },
            json!({ "image": { "file": "bg.png" } }),
        ),
        (
            Background::SystemWallpaper {
                path: "C:/w.jpg".into(),
            },
            json!({ "systemWallpaper": { "path": "C:/w.jpg" } }),
        ),
        (
            Background::Mesh {
                points: vec![nss_core::MeshPoint {
                    x: 0.0,
                    y: 1.0,
                    color: RGBAColor::BLACK,
                }],
                warp: 0.06,
            },
            json!({ "mesh": { "points": [{ "x": 0.0, "y": 1.0, "color": { "r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0 } }], "warp": 0.06 } }),
        ),
    ];
    for (bg, expected) in cases {
        let v: Value = serde_json::to_value(&bg).unwrap();
        assert_eq!(v, expected);
        let back: Background = serde_json::from_value(v).unwrap();
        assert_eq!(back, bg);
    }
}

#[test]
fn timestamps_use_the_corefoundation_epoch() {
    // Swift's `Date` is seconds since 2001-01-01Z; 0.0 must mean exactly that.
    let t: Timestamp = serde_json::from_str("0.0").unwrap();
    assert_eq!(t.unix_secs(), REFERENCE_DATE);
    assert_eq!(serde_json::to_string(&Timestamp(0.0)).unwrap(), "0.0");
    // A recording timestamp an hour in is just raw seconds.
    let parsed: Timestamp = serde_json::from_str("3600").unwrap();
    assert_eq!(parsed.0, 3600.0);
    assert_eq!(parsed.unix_secs(), REFERENCE_DATE + 3600.0);
}

#[test]
fn duration_type_is_plain_seconds() {
    // MediaTime is a bare f64 on the wire, in every position.
    let t: (MediaTime, MediaTime) = serde_json::from_str("[0.5, 3]").unwrap();
    assert_eq!(t, (0.5, 3.0));
    let v: Value = serde_json::to_value(Timeline::default()).unwrap();
    assert_eq!(v["trim"], json!({ "start": 0.0, "end": null }));
}

proptest! {
    /// Any in-range style survives a save/load cycle unchanged. Style is the type most
    /// often edited by hand in `project.json`, so a lossy field would corrupt user work.
    #[test]
    fn arbitrary_styles_round_trip(
        padding in 0.0f64..0.3,
        corner_radius in 0.0f64..0.2,
        background_blur in 0.0f64..1.0,
        cursor_size in 0.5f64..4.0,
        smoothing in 0.0f64..1.0,
        zoomed_scale in 0.1f64..1.0,
        blur in 0.0f64..1.0,
        ripple in any::<bool>(),
    ) {
        let style = Style {
            padding,
            corner_radius,
            background_blur,
            cursor: nss_core::CursorStyle {
                size: cursor_size,
                smoothing,
                motion_blur: blur,
                click_ripples: ripple,
                hide_when_idle: !ripple,
            },
            camera: CameraBubble {
                zoomed_scale,
                ..CameraBubble::default()
            },
            ..Style::default()
        };
        let text = serde_json::to_string(&style).unwrap();
        let back: Style = serde_json::from_str(&text).unwrap();
        prop_assert_eq!(back, style);
    }

    /// A zoom block never loses its timing, whatever the times: the motion model depends on
    /// these being bit-exact after a reload.
    #[test]
    fn arbitrary_zoom_blocks_round_trip(start in 0.0f64..3600.0, len in 0.3f64..60.0, scale in 1.0f64..8.0, auto in any::<bool>()) {
        let block = ZoomBlock {
            is_auto: auto,
            ..ZoomBlock::new(start, start + len)
        };
        let scaled = ZoomBlock { scale, ..block.clone() };
        let text = serde_json::to_string(&scaled).unwrap();
        let back: ZoomBlock = serde_json::from_str(&text).unwrap();
        prop_assert_eq!(back, scaled);
    }
}
