//! Headless spectrum monitor.
//!
//! Renders the analyser's band levels as an ASCII bar graph, so the entire DSP
//! chain can be validated against real music before any hardware exists. This is
//! phase 0 of the project: the riskiest parts — loopback capture and the
//! analysis ladder — proved out first, with nothing else to hide behind.

use std::io::Write;
use std::time::{Duration, Instant};

use anyhow::Result;
use djled_engine::capture::LoopbackCapture;
use djled_engine::{Engine, EngineConfig};

const HEIGHT: usize = 24;
const TARGET_FPS: u64 = 30;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let probe_secs = match args.first().map(String::as_str) {
        Some("--probe") => Some(args.get(1).and_then(|s| s.parse().ok()).unwrap_or(3.0)),
        Some(other) => anyhow::bail!("unknown argument '{other}' (expected --probe [seconds])"),
        None => None,
    };

    let mut capture = LoopbackCapture::new(0.25)?;
    let cfg = EngineConfig::default();
    let mut engine = Engine::new(&cfg, capture.sample_rate());

    print_header(&capture, &engine);

    if let Some(secs) = probe_secs {
        return probe(&mut capture, &mut engine, secs);
    }

    let mut buf = vec![0.0f32; 8192];
    let mut last_draw = Instant::now();
    let frame = Duration::from_millis(1000 / TARGET_FPS);
    let mut first = true;

    loop {
        let n = capture.read(&mut buf);
        if n == 0 {
            // Loopback delivers nothing at all while the endpoint is idle, so
            // this is the normal state when no audio is playing, not an error.
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }

        engine.push(&buf[..n]);

        // The analyser runs at ~187 frames/s; redrawing that fast would just
        // burn a core on terminal I/O.
        if last_draw.elapsed() >= frame {
            draw(engine.levels(), first);
            last_draw = Instant::now();
            first = false;
        }
    }
}

/// Capture for a fixed duration and report what arrived, then exit.
///
/// Distinguishes the three states that matter when bringing this up: the stream
/// failed to open, the stream opened but nothing is playing, or audio is
/// flowing. Loopback legitimately delivers nothing while the endpoint is idle,
/// so silence is not by itself a fault.
fn probe(capture: &mut LoopbackCapture, engine: &mut Engine, secs: f64) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs_f64(secs);
    let mut buf = vec![0.0f32; 8192];
    let (mut samples, mut frames) = (0usize, 0usize);
    let mut peak = 0.0f32;
    let mut hottest = vec![0.0f32; engine.band_count()];

    while Instant::now() < deadline {
        let n = capture.read(&mut buf);
        if n == 0 {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        samples += n;
        peak = buf[..n].iter().fold(peak, |a, &s| a.max(s.abs()));

        if engine.push(&buf[..n]) {
            frames += 1;
            for (h, &l) in hottest.iter_mut().zip(engine.levels()) {
                *h = h.max(l);
            }
        }
    }

    let expected = capture.sample_rate() * secs;
    println!("probe: {secs:.1} s");
    println!(
        "  samples  {samples} ({:.0}% of the {expected:.0} expected)",
        100.0 * samples as f64 / expected
    );
    println!("  frames   {frames} analysis frames");
    println!("  peak     {peak:.4} ({:.1} dBFS)", 20.0 * peak.max(1e-9).log10());

    if samples == 0 {
        println!("\n  Stream opened but delivered nothing. WASAPI loopback is idle when the");
        println!("  output endpoint is idle — play some audio and probe again.");
        return Ok(());
    }
    if peak < 1e-6 {
        println!("\n  Samples arrived but all were silent (digital silence is still delivered).");
        return Ok(());
    }

    let plan = engine.plan();
    let mut ranked: Vec<usize> = (0..hottest.len()).collect();
    ranked.sort_by(|&a, &b| hottest[b].partial_cmp(&hottest[a]).unwrap());
    println!("\n  loudest bands");
    for &i in ranked.iter().take(5) {
        let b = &plan.bands[i];
        println!(
            "    {:6.0} - {:6.0} Hz  N={:<5} {:.2}",
            b.lo, b.hi, b.fft_size, hottest[i]
        );
    }
    Ok(())
}

fn print_header(capture: &LoopbackCapture, engine: &Engine) {
    println!("DJLED spectrum monitor");
    println!(
        "  device   {} ({} ch @ {:.0} Hz)",
        capture.device_name(),
        capture.channels(),
        capture.sample_rate()
    );
    println!("  bands    {}", engine.band_count());
    println!("  tiers");
    for line in engine.plan().describe_tiers() {
        println!("           {line}");
    }

    let unresolved: Vec<_> = engine.plan().unresolved().map(|(i, _)| i).collect();
    if !unresolved.is_empty() {
        println!("  WARNING  bands {unresolved:?} could not be resolved by any FFT size");
    }

    println!("\n  play something. ctrl-c to quit.\n");
}

fn draw(levels: &[f32], first: bool) {
    let mut out = String::with_capacity((levels.len() + 8) * (HEIGHT + 3));

    if !first {
        // Rewind over the bars plus the axis line drawn last time.
        out.push_str(&format!("\x1b[{}A", HEIGHT + 1));
    }

    for row in (0..HEIGHT).rev() {
        for &level in levels {
            let filled = level * HEIGHT as f32 - row as f32;
            out.push(match filled {
                f if f >= 0.75 => '█',
                f if f >= 0.5 => '▓',
                f if f >= 0.25 => '▒',
                f if f > 0.0 => '░',
                _ => ' ',
            });
            out.push(' ');
        }
        out.push('\n');
    }

    for i in 0..levels.len() {
        out.push_str(if i % 8 == 0 { "┬ " } else { "─ " });
    }
    out.push('\n');

    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(out.as_bytes());
    let _ = stdout.flush();
}
