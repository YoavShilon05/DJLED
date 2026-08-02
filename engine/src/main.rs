//! DJLED engine — capture, analyse, colour, and drive the strip.
//!
//! Runs with or without hardware. Without `--port` it uses a mock link, so the
//! whole pipeline can be watched in the terminal before the Arduino is wired up.

use std::io::Write;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};

use djled_engine::capture::LoopbackCapture;
use djled_engine::color::{Geometry, RenderConfig, Renderer};
use djled_engine::link::protocol::TestPattern;
use djled_engine::link::serial::DEFAULT_BAUD;
use djled_engine::link::{expand_bands_to_leds, Link, MockLink, SerialLink};
use djled_engine::ui::{encode_strip, Command, State, UiServer};
use djled_engine::{Engine, EngineConfig, ShowConfig};

const BAR_HEIGHT: usize = 20;
const TARGET_FPS: u64 = 60;

#[derive(Parser, Debug)]
#[command(about = "Audio-reactive LED wall engine", version)]
struct Args {
    /// Serial port to drive, e.g. COM3. Omit to run without hardware.
    #[arg(long)]
    port: Option<String>,

    #[arg(long, default_value_t = DEFAULT_BAUD)]
    baud: u32,

    /// LEDs on the strip. Ignored with --port; the firmware reports its own.
    #[arg(long, default_value_t = 150)]
    leds: usize,

    /// Frequency bands, and therefore colours sent per frame.
    #[arg(long, default_value_t = 48)]
    bands: usize,

    /// List serial ports and exit.
    #[arg(long)]
    list_ports: bool,

    /// Capture for N seconds, report what arrived, and exit.
    #[arg(long, value_name = "SECONDS")]
    probe: Option<f64>,

    /// Show a wiring diagnostic pattern instead of audio.
    #[arg(long, value_enum)]
    test: Option<Pattern>,

    /// Master brightness, 0..1. The software half of the power budget.
    #[arg(long, default_value_t = 1.0)]
    brightness: f32,

    /// Port for the editor UI to connect to.
    #[arg(long, default_value_t = djled_engine::ui::DEFAULT_PORT)]
    ui_port: u16,

    /// Run without the UI server, leaving the port free.
    #[arg(long)]
    no_ui: bool,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Pattern {
    /// Red, green, blue in turn — confirms channel order.
    Rgb,
    /// A dot walking the strip — confirms LED count and direction.
    Chase,
    /// Full white — reveals voltage droop and where injection is needed.
    White,
}

impl From<Pattern> for TestPattern {
    fn from(p: Pattern) -> Self {
        match p {
            Pattern::Rgb => TestPattern::Rgb,
            Pattern::Chase => TestPattern::Chase,
            Pattern::White => TestPattern::White,
        }
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.list_ports {
        let ports = SerialLink::available();
        if ports.is_empty() {
            println!("no serial ports found");
        } else {
            println!("serial ports:");
            for p in ports {
                println!("  {p}");
            }
        }
        return Ok(());
    }

    let (mut link, led_count): (Box<dyn Link>, usize) = match &args.port {
        Some(path) => {
            let link = SerialLink::open(path, args.baud)
                .with_context(|| available_ports_hint(path))?;
            let count = link.hello().led_count as usize;
            (Box::new(link), count)
        }
        None => (Box::new(MockLink::new(args.leds)), args.leds),
    };

    if let Some(pattern) = args.test {
        link.send_test(pattern.into())?;
        println!("{}", link.describe());
        println!("showing {:?} pattern — ctrl-c to stop", pattern);
        // Hold the port open; the firmware keeps rendering on its own.
        loop {
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    let mut capture = LoopbackCapture::new(0.25)?;
    let mut cfg = EngineConfig {
        scale: djled_engine::dsp::bands::BandScaleConfig {
            band_count: args.bands,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut engine = Engine::new(&cfg, capture.sample_rate());

    let show = ShowConfig::spanning(led_count);
    let mut renderer = Renderer::new(
        RenderConfig { brightness: args.brightness, ..Default::default() },
        &show.surface,
        show.layout(),
        show.intensity(),
        Geometry {
            centers: engine.centers().to_vec(),
            // One colour per band keeps the wire format exactly as it was; see
            // `color::strip` for why these are strip positions, not bands.
            points: engine.band_count(),
            leds: led_count,
        },
    )
    .map_err(anyhow::Error::msg)?;

    print_header(&capture, &engine, link.as_ref(), led_count);

    if let Some(secs) = args.probe {
        return probe(&mut capture, &mut engine, secs);
    }

    // The UI is optional by design: the engine is a headless service and
    // nothing about the strip depends on a browser being attached.
    let server = if args.no_ui {
        None
    } else {
        let state = State {
            config: show.clone(),
            brightness: args.brightness,
            led_count,
            band_count: engine.band_count(),
            db_floor: cfg.post.db_floor,
            db_ceil: cfg.post.db_ceil,
        };
        match UiServer::start(args.ui_port, state) {
            Ok(s) => {
                println!("  editor   ws://127.0.0.1:{} — run `npm run dev` in ui/\n", s.port());
                Some(s)
            }
            Err(e) => {
                println!("  editor   unavailable: {e}\n");
                None
            }
        }
    };

    run(
        &mut capture,
        &mut engine,
        &mut cfg,
        show,
        &mut renderer,
        link.as_mut(),
        led_count,
        server.as_ref(),
    )
}

fn available_ports_hint(requested: &str) -> String {
    let ports = SerialLink::available();
    if ports.is_empty() {
        format!("could not open {requested}, and no serial ports are visible at all")
    } else {
        format!("could not open {requested}. Available: {}", ports.join(", "))
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    capture: &mut LoopbackCapture,
    engine: &mut Engine,
    engine_cfg: &mut EngineConfig,
    mut show: ShowConfig,
    renderer: &mut Renderer,
    link: &mut dyn Link,
    led_count: usize,
    server: Option<&UiServer>,
) -> Result<()> {
    let mut buf = vec![0.0f32; 8192];
    let mut leds = vec![[0u8; 3]; led_count];
    let mut last_draw = Instant::now();
    let frame = Duration::from_millis(1000 / TARGET_FPS);
    let mut first = true;
    let mut dropped = 0u64;
    let mut status = String::new();

    let centers: Vec<f32> = engine.centers().to_vec();
    let sample_rate = capture.sample_rate();

    // The initial config is applied through the same path an edit takes, so
    // startup cannot diverge from what the editor would produce.
    apply_show(engine, engine_cfg, renderer, &show, &show, true, sample_rate);

    loop {
        let n = capture.read(&mut buf);
        if n == 0 {
            // Loopback is genuinely idle when nothing is playing; not an error.
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }

        if !engine.push(&buf[..n]) {
            continue;
        }

        // Applied before rendering so an edit takes effect on this frame rather
        // than the next one.
        if let Some(server) = server {
            for cmd in server.commands() {
                match cmd {
                    Command::Config { config } => {
                        let next = *config;
                        match renderer.set_surface(&next.surface) {
                            Ok(()) => {
                                apply_show(
                                    engine,
                                    engine_cfg,
                                    renderer,
                                    &next,
                                    &show,
                                    false,
                                    sample_rate,
                                );
                                show = next.clone();
                                status.clear();
                                server.update_state(|s| s.config = next);
                            }
                            // Reported through the status line rather than
                            // stdout: printing here would tear the display.
                            // The rest of the config is dropped with it, so a
                            // half-applied edit is not left behind.
                            Err(e) => status = format!("config rejected: {e}"),
                        }
                    }
                    Command::Brightness { value } => {
                        let mut cfg = renderer.config().clone();
                        cfg.brightness = value.clamp(0.0, 1.0);
                        renderer.set_config(cfg);
                        server.update_state(|s| s.brightness = value.clamp(0.0, 1.0));
                    }
                    Command::RequestState => {}
                }
            }
        }

        let pixels = renderer.render(engine.levels());
        if !link.send(pixels)? {
            dropped += 1;
        }

        // The analyser runs far faster than any display needs; redrawing and
        // republishing at its rate would just burn a core on formatting.
        if last_draw.elapsed() >= frame {
            expand_bands_to_leds(pixels, &mut leds);

            if let Some(server) = server {
                let strip = encode_strip(&leds);
                let levels = engine.levels().to_vec();
                server.publish(|snap| {
                    snap.levels = levels;
                    snap.strip = strip;
                    snap.dropped_frames = dropped;
                    snap.sample_rate = sample_rate;
                    snap.connected = true;
                    if snap.centers.len() != centers.len() {
                        snap.centers = centers.clone();
                    }
                });
            }

            draw(engine.levels(), &leds, dropped, &status, first);
            last_draw = Instant::now();
            first = false;
        }
    }
}

/// Push a configuration into the stages it touches.
///
/// Everything is compared against what is already in force, because the two
/// expensive cases must not fire on every keyframe drag: resampling the EQ walks
/// every band, and a hop change rebuilds the analyser outright — which resets
/// the floor tracker and the ballistics, so it is worth a visible hiccup only
/// when the user actually asked for it.
fn apply_show(
    engine: &mut Engine,
    engine_cfg: &mut EngineConfig,
    renderer: &mut Renderer,
    next: &ShowConfig,
    current: &ShowConfig,
    force: bool,
    sample_rate: f64,
) {
    if force || next.hop() != current.hop() {
        engine_cfg.hop = next.hop();
        *engine = Engine::new(engine_cfg, sample_rate);
        // A rebuilt analyser has no EQ or ballistics, so both are reinstalled
        // below regardless of whether they were what changed.
        engine.set_eq(&next.eq);
        engine.set_decay(next.decay());
    } else {
        if !eq_matches(&next.eq, &current.eq) {
            engine.set_eq(&next.eq);
        }
        if next.decay() != current.decay() {
            engine.set_decay(next.decay());
        }
    }

    renderer.set_layout(next.layout());
    renderer.set_intensity(next.intensity());
}

/// Structural comparison; `EqBand` is not `PartialEq` because it holds floats
/// and an exact-equality derive on those would be a trap elsewhere.
fn eq_matches(a: &[djled_engine::dsp::eq::EqBand], b: &[djled_engine::dsp::eq::EqBand]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.kind == y.kind && x.hz == y.hz && x.gain == y.gain && x.q == y.q
        })
}

/// Capture for a fixed duration and report what arrived, then exit.
///
/// Separates the three states worth distinguishing during bring-up: the stream
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
        println!("    {:6.0} - {:6.0} Hz  N={:<5} {:.2}", b.lo, b.hi, b.fft_size, hottest[i]);
    }
    Ok(())
}

fn print_header(capture: &LoopbackCapture, engine: &Engine, link: &dyn Link, led_count: usize) {
    println!("DJLED");
    println!(
        "  audio    {} ({} ch @ {:.0} Hz)",
        capture.device_name(),
        capture.channels(),
        capture.sample_rate()
    );
    println!("  output   {}", link.describe());
    println!("  bands    {} across {led_count} LEDs", engine.band_count());
    println!("  tiers");
    for line in engine.plan().describe_tiers() {
        println!("           {line}");
    }

    let unresolved: Vec<_> = engine.plan().unresolved().map(|(i, _)| i).collect();
    if !unresolved.is_empty() {
        println!(
            "  WARNING  bands {unresolved:?} cannot be resolved by any available FFT size.\n\
             {:11}Lower --bands, or raise the bottom of the range.",
            ""
        );
    }

    println!("\n  play something. ctrl-c to quit.\n");
}

fn draw(levels: &[f32], leds: &[[u8; 3]], dropped: u64, status: &str, first: bool) {
    let mut out = String::with_capacity(levels.len() * (BAR_HEIGHT + 4) + leds.len() * 24);

    if !first {
        // Rewind over the bars, the strip preview and the status line.
        out.push_str(&format!("\x1b[{}A", BAR_HEIGHT + 3));
    }

    for row in (0..BAR_HEIGHT).rev() {
        for &level in levels {
            let filled = level * BAR_HEIGHT as f32 - row as f32;
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

    // The strip as the firmware will actually drive it, at the same width as
    // the bars so the two line up.
    let width = levels.len() * 2;
    for col in 0..width {
        let start = col * leds.len() / width;
        let end = ((col + 1) * leds.len() / width).max(start + 1).min(leds.len());
        let n = (end - start) as u32;
        let mut acc = [0u32; 3];
        for px in &leds[start..end] {
            for c in 0..3 {
                acc[c] += px[c] as u32;
            }
        }
        out.push_str(&format!("\x1b[48;2;{};{};{}m ", acc[0] / n, acc[1] / n, acc[2] / n));
    }
    out.push_str("\x1b[0m\n");

    out.push_str(&format!(
        "  {} LEDs   dropped frames: {dropped}   {status}\x1b[K\n",
        leds.len()
    ));

    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(out.as_bytes());
    let _ = stdout.flush();
}
