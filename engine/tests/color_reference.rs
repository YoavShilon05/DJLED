//! Asserts the committed colour reference still describes this implementation.
//!
//! `ui/src/color/reference.json` is the contract between the Rust colour
//! pipeline and its TypeScript port in the editor. The UI has a matching test.
//! This one catches drift on the Rust side — without it, a change here would
//! quietly invalidate the fixture and the UI test would start failing for a
//! reason that has nothing to do with the UI.
//!
//! If this fails after a deliberate change, regenerate:
//!
//! ```text
//! cargo run --example color_reference > ../ui/src/color/reference.json
//! ```

use djled_engine::color::{ColorSurface, Keyframe, SurfaceConfig};

const REFERENCE: &str = include_str!("../../ui/src/color/reference.json");

#[test]
fn committed_reference_matches_this_implementation() {
    let doc: serde_json::Value =
        serde_json::from_str(REFERENCE).expect("reference.json is not valid JSON");

    let keyframes: Vec<Keyframe> = doc["keyframes"]
        .as_array()
        .expect("reference.json has no keyframes array")
        .iter()
        .map(|k| {
            Keyframe::new(
                k["x"].as_f64().unwrap() as f32,
                k["y"].as_f64().unwrap() as f32,
                k["color"].as_str().unwrap(),
            )
        })
        .collect();

    let cfg = SurfaceConfig { keyframes, sigma: doc["sigma"].as_f64().unwrap() as f32 };
    let surface = ColorSurface::new(&cfg).expect("reference config must be valid");

    let samples = doc["samples"].as_array().expect("reference.json has no samples array");
    assert!(!samples.is_empty(), "reference.json contains no samples");

    for s in samples {
        let (x, y) = (s["x"].as_f64().unwrap() as f32, s["y"].as_f64().unwrap() as f32);
        let got = surface.sample(x, y);

        for (name, actual, expected) in [
            ("l", got.l, s["l"].as_f64().unwrap() as f32),
            ("a", got.a, s["a"].as_f64().unwrap() as f32),
            ("b", got.b, s["b"].as_f64().unwrap() as f32),
        ] {
            assert!(
                (actual - expected).abs() < 1e-6,
                "{name} at ({x}, {y}): got {actual}, reference says {expected}. \
                 Regenerate the fixture if this change was intended."
            );
        }

        let rgb = got.to_linear_rgb().clamped();
        let led = [
            (rgb.r * 255.0).round() as u8,
            (rgb.g * 255.0).round() as u8,
            (rgb.b * 255.0).round() as u8,
        ];
        let expected: Vec<u8> = s["led"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect();
        assert_eq!(led.to_vec(), expected, "LED bytes at ({x}, {y})");
    }
}

/// The fixture must describe the *default* palette, or the UI and engine start
/// from different colours before anything is edited.
#[test]
fn reference_tracks_the_default_surface() {
    let doc: serde_json::Value = serde_json::from_str(REFERENCE).unwrap();
    let default = SurfaceConfig::default();

    let keyframes = doc["keyframes"].as_array().unwrap();
    assert_eq!(
        keyframes.len(),
        default.keyframes.len(),
        "reference.json is stale — regenerate it"
    );

    for (recorded, current) in keyframes.iter().zip(&default.keyframes) {
        assert_eq!(recorded["color"].as_str().unwrap(), current.color);
        assert!((recorded["x"].as_f64().unwrap() as f32 - current.x).abs() < 1e-6);
        assert!((recorded["y"].as_f64().unwrap() as f32 - current.y).abs() < 1e-6);
    }
    assert!((doc["sigma"].as_f64().unwrap() as f32 - default.sigma).abs() < 1e-6);
}
