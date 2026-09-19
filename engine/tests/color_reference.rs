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
use djled_engine::color::{ColorSurface, Keyframe, SurfaceConfig, Timeline, TimelineKey};

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
        // Absent means off, which is what every case recorded before the flag
        // existed meant and what an older peer sends.
        let surface = ColorSurface::new(&cfg)
            .expect("reference config must be valid")
            .cycling(case["cycle"].as_bool().unwrap_or(false));

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

/// And for cycling: a fixture where nothing joins the axis would agree whether
/// the port measured a distance the short way round or not, since that is the
/// only thing the flag changes.
#[test]
fn reference_exercises_cycling() {
    let doc = document();
    let cycling: Vec<&serde_json::Value> = doc["cases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(doc["timelines"].as_array().unwrap())
        .filter(|c| c["cycle"].as_bool().unwrap_or(false))
        .collect();

    assert!(!cycling.is_empty(), "nothing in the reference joins the position axis");

    // A case that cycles but keeps every colour away from the seam proves
    // nothing either: the flag only shows where the short way round is the way
    // across the edge.
    let near_the_seam = cycling
        .iter()
        .flat_map(|c| match c["keyframes"].as_array() {
            Some(keys) => keys.clone(),
            None => c["keys"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|k| k["keyframes"].as_array().unwrap().clone())
                .collect(),
        })
        .any(|k| {
            let x = k["x"].as_f64().unwrap();
            !(0.15..=0.85).contains(&x)
        });
    assert!(near_the_seam, "no cycling keyframe sits near an edge, where wrapping shows");
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

/// The same contract for the field as a function of time.
///
/// Both implementations walk the same bracketing keys, blend the same way and
/// wrap the same way, so the Preview strip shows the instant of the loop the
/// wall is actually at. A drift here is worse than a static one: it looks
/// correct until somebody tries to place a key against the music.
#[test]
fn committed_reference_matches_the_timeline_implementation() {
    let doc = document();
    let timelines = doc["timelines"].as_array().expect("reference.json has no timelines array");
    assert!(!timelines.is_empty(), "reference.json contains no timelines");

    for case in timelines {
        let name = case["name"].as_str().unwrap_or("?");
        let mut surface = ColorSurface::animated(&timeline(case))
            .expect("reference timeline must be valid")
            .cycling(case["cycle"].as_bool().unwrap_or(false));

        let phases = case["phases"].as_array().expect("case has no phases array");
        assert!(!phases.is_empty(), "timeline '{name}' contains no phases");

        for phase in phases {
            let seconds = phase["seconds"].as_f64().unwrap();
            surface.seek(seconds);

            for s in phase["samples"].as_array().unwrap() {
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
                        "{channel} at ({x}, {y}) of '{name}' at {seconds}s: got {actual}, \
                         reference says {expected}. Regenerate the fixture if this was intended."
                    );
                }
            }
        }
    }
}

/// The fixture has to contain a span whose two ends disagree about whether a
/// keyframe exists, and one that opens an area of effect all the way out.
/// Without them both implementations could take the easy reading of each and
/// still agree with the file.
#[test]
fn reference_exercises_the_awkward_halves_of_a_blend() {
    let doc = document();
    let timelines = doc["timelines"].as_array().unwrap();

    let counts: Vec<Vec<usize>> = timelines
        .iter()
        .map(|t| {
            t["keys"]
                .as_array()
                .unwrap()
                .iter()
                .map(|k| k["keyframes"].as_array().unwrap().len())
                .collect()
        })
        .collect();
    assert!(
        counts.iter().any(|c| c.iter().min() != c.iter().max()),
        "no timeline in the fixture has a keyframe missing from one of its keys",
    );

    let radii: Vec<Option<f64>> = timelines
        .iter()
        .flat_map(|t| t["keys"].as_array().unwrap())
        .flat_map(|k| k["keyframes"].as_array().unwrap())
        .map(|f| f["radius"].as_f64())
        .collect();
    assert!(radii.iter().any(|r| r.is_some()), "no confined keyframe in the fixture");
    assert!(radii.iter().any(|r| r.is_none()), "no unconfined keyframe in the fixture");

    // And the loop has to actually move, or every phase could be the first key.
    let moved = timelines.iter().any(|t| {
        let phases = t["phases"].as_array().unwrap();
        let first: Vec<f64> =
            phases[0]["samples"].as_array().unwrap().iter().map(|s| s["l"].as_f64().unwrap()).collect();
        phases.iter().skip(1).any(|p| {
            p["samples"]
                .as_array()
                .unwrap()
                .iter()
                .zip(&first)
                .any(|(s, l)| (s["l"].as_f64().unwrap() - l).abs() > 1e-3)
        })
    });
    assert!(moved, "every phase in the fixture is the same field");
}

/// A recorded timeline, back in the shape the engine compiles.
fn timeline(case: &serde_json::Value) -> Timeline {
    Timeline {
        enabled: true,
        length: case["length"].as_f64().unwrap() as f32,
        keys: case["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| {
                TimelineKey::new(
                    k["at"].as_f64().unwrap() as f32,
                    SurfaceConfig {
                        sigma: k["sigma"].as_f64().unwrap() as f32,
                        keyframes: k["keyframes"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|f| Keyframe {
                                radius: f["radius"].as_f64().map(|r| r as f32),
                                ..Keyframe::new(
                                    f["x"].as_f64().unwrap() as f32,
                                    f["y"].as_f64().unwrap() as f32,
                                    f["color"].as_str().unwrap(),
                                )
                            })
                            .collect(),
                    },
                )
            })
            .collect(),
    }
}
