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

use djled_engine::color::oklab::LinearRgb;
use djled_engine::color::{ColorSurface, Keyframe, SurfaceConfig};

const REFERENCE: &str = include_str!("../../ui/src/color/reference.json");

fn document() -> serde_json::Value {
    serde_json::from_str(REFERENCE).expect("reference.json is not valid JSON")
}

#[test]
fn committed_reference_matches_this_implementation() {
    let doc = document();
    let cases = doc["cases"].as_array().expect("reference.json has no cases array");
    assert!(!cases.is_empty(), "reference.json contains no cases");

    for case in cases {
        let name = case["name"].as_str().unwrap_or("?");

        let keyframes: Vec<Keyframe> = case["keyframes"]
            .as_array()
            .unwrap_or_else(|| panic!("case '{name}' has no keyframes array"))
            .iter()
            .map(|k| {
                let base = Keyframe::new(
                    k["x"].as_f64().unwrap() as f32,
                    k["y"].as_f64().unwrap() as f32,
                    k["color"].as_str().unwrap(),
                );
                Keyframe { radius: k["radius"].as_f64().map(|r| r as f32), ..base }
            })
            .collect();

        let cfg = SurfaceConfig { keyframes, sigma: case["sigma"].as_f64().unwrap() as f32 };
        let surface = ColorSurface::new(&cfg).expect("reference config must be valid");

        let samples = case["samples"].as_array().expect("case has no samples array");
        assert!(!samples.is_empty(), "case '{name}' contains no samples");

        for s in samples {
            let (x, y) = (s["x"].as_f64().unwrap() as f32, s["y"].as_f64().unwrap() as f32);
            let got = surface.sample(x, y);

            for (channel, actual, expected) in [
                ("l", got.l, s["l"].as_f64().unwrap() as f32),
                ("a", got.a, s["a"].as_f64().unwrap() as f32),
                ("b", got.b, s["b"].as_f64().unwrap() as f32),
                ("alpha", got.alpha, s["alpha"].as_f64().unwrap() as f32),
            ] {
                assert!(
                    (actual - expected).abs() < 1e-6,
                    "{channel} at ({x}, {y}) in '{name}': got {actual}, reference says {expected}. \
                     Regenerate the fixture if this change was intended."
                );
            }

            let rgb = got.to_linear_rgb().clamped().over(LinearRgb::BLACK);
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
            assert_eq!(led.to_vec(), expected, "LED bytes at ({x}, {y}) in '{name}'");
        }
    }
}

/// The first case must describe the *default* palette, or the UI and engine
/// start from different colours before anything is edited.
#[test]
fn reference_tracks_the_default_surface() {
    let doc = document();
    let case = &doc["cases"][0];
    let default = SurfaceConfig::default();

    assert_eq!(case["name"].as_str().unwrap(), "default");

    let keyframes = case["keyframes"].as_array().unwrap();
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
    assert!((case["sigma"].as_f64().unwrap() as f32 - default.sigma).abs() < 1e-6);
}

/// Likewise for the area of effect: the fixture has to contain dead space, or
/// both implementations could ignore the radius entirely and still agree.
#[test]
fn reference_exercises_the_area_of_effect() {
    let doc = document();
    let cases = doc["cases"].as_array().unwrap();

    let confined = cases
        .iter()
        .flat_map(|c| c["keyframes"].as_array().unwrap())
        .filter(|k| k["radius"].is_number())
        .count();
    assert!(confined > 0, "no keyframe in the reference carries a radius");

    let baseline = cases
        .iter()
        .flat_map(|c| c["samples"].as_array().unwrap())
        .filter(|s| s["alpha"].as_f64().unwrap() == 0.0)
        .count();
    assert!(baseline > 0, "no sample falls in the dead space past every area of effect");
}

/// The fixture is only worth having for opacity if something in it is actually
/// translucent. An all-opaque reference would let both implementations get
/// opacity-weighted blending wrong and still agree.
#[test]
fn reference_exercises_opacity() {
    let doc = document();
    let cases = doc["cases"].as_array().unwrap();

    let alphas: Vec<f64> = cases
        .iter()
        .flat_map(|c| c["samples"].as_array().unwrap())
        .map(|s| s["alpha"].as_f64().unwrap())
        .collect();

    assert!(alphas.iter().any(|&a| a > 0.99), "no sample is opaque");
    assert!(alphas.iter().any(|&a| a < 0.1), "no sample is near-transparent");
    assert!(
        alphas.iter().any(|&a| (0.3..0.7).contains(&a)),
        "no sample is partially covered, which is where the blending maths shows"
    );
}
