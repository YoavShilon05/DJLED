//! DJLED engine — capture, analyse, colour, and drive the strip.
//!
//! Runs with or without hardware. Without `--port` it uses a mock link, so the
//! whole pipeline can be watched in the terminal before the Arduino is wired up.

use std::io::Write;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};

use djled_engine::color::{Geometry, RenderConfig, Renderer};
use djled_engine::link::protocol::TestPattern;
use djled_engine::link::serial::DEFAULT_BAUD;
use djled_engine::link::{expand_bands_to_leds, Link, MockLink, SerialLink};
use djled_engine::source::{self, LiveSource, Source, SourceKind};
use djled_engine::ui::{encode_strip, Command, InputState, State, UiServer};
use djled_engine::{EngineConfig, ShowConfig};

const BAR_HEIGHT: usize = 20;
const TARGET_FPS: u64 = 60;

/// Ring depth between the audio callback and the analysis thread. It only needs
/// to cover scheduling jitter; a quarter second is generous and still bounds how
/// far behind the display can fall.
const CAPTURE_BUFFER_SECS: f32 = 0.25;

#[derive(Parser, Debug)]
#[command(about = "Audio- and MIDI-reactive LED wall engine", version)]
struct Args {
    /// Serial port to drive, e.g. COM3. Omit to run without hardware.
    #[arg(long)]
    port: Option<String>,

    #[arg(long, default_value_t = DEFAULT_BAUD)]
    baud: u32,

    /// LEDs on the strip. Ignored with --port; the firmware reports its own.
    #[arg(long, default_value_t = 150)]
    leds: usize,

    /// Frequency bands, and therefore colours sent per frame. Audio only — a
    /// MIDI source has one point per semitone.
    #[arg(long, default_value_t = 48)]
    bands: usize,

    /// List serial ports and exit.
    #[arg(long)]
    list_ports: bool,

    /// List audio devices and MIDI ports, then exit.
    #[arg(long)]
    list_devices: bool,

    /// What to listen to: an id from --list-devices, or any unique part of a
    /// device name. Omit for whatever the PC is playing.
    #[arg(long, value_name = "DEVICE")]
    source: Option<String>,

    /// Capture an input rather than what the PC is playing. On its own, the
    /// default recording device; with --source, it picks the capture half of an
    /// interface whose two halves share a name.
    #[arg(long)]
    input: bool,

    /// Listen to MIDI. On its own, the first MIDI input there is; with --source,
    /// the port whose name matches.
    #[arg(long)]
    midi: bool,

    /// Analyse a single channel, counting from 1 — one input of an interface,
    /// or one MIDI channel. Omit to take them all.
    #[arg(long, value_name = "N")]
    channel: Option<usize>,

    /// Listen for N seconds, report what arrived, and exit.
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

    if args.list_devices {
        list_devices();
        return Ok(());
    }

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

    let base = EngineConfig {
        scale: djled_engine::dsp::bands::BandScaleConfig {
            band_count: args.bands,
            ..Default::default()
        },
        ..Default::default()
    };
    let show = ShowConfig::spanning(led_count);
    // What the board says it will take, not what the protocol allows. A frame
    // over the firmware's own limit is dropped at the MCU without a word.
    let point_cap = link.max_points();
    let mut live = LiveSource::open(&select_source(&args)?, &base, &show, CAPTURE_BUFFER_SECS)?;
    let mut renderer = build_renderer(args.brightness, &show, &live, led_count, point_cap)?;

    print_header(&live, link.as_ref(), led_count, point_cap);

    if let Some(secs) = args.probe {
        return probe(&mut live, secs);
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
            band_count: live.points().min(point_cap),
            db_floor: base.post.db_floor,
            db_ceil: base.post.db_ceil,
            input: InputState {
                devices: source::devices(),
                source: live.source().clone(),
                device_name: live.device_name().to_string(),
                kind: live.kind(),
                channels: live.channels(),
                sample_rate: live.sample_rate(),
                error: None,
            },
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

    run(&mut live, &base, show, &mut renderer, link.as_mut(), led_count, point_cap, server.as_ref())
}

/// The source the flags ask for. Named devices are looked up now rather than at
/// open time so a typo fails with the list of what was meant, not a stream error.
fn select_source(args: &Args) -> Result<Source> {
    let kind = if args.midi {
        Some(SourceKind::Midi)
    } else if args.input {
        Some(SourceKind::Input)
    } else {
        None
    };

    let mut source = match (&args.source, kind) {
        (Some(spec), _) => Source::find(spec, kind)?,
        (None, Some(SourceKind::Midi)) => Source::default_midi(),
        (None, Some(SourceKind::Input)) => Source::default_input(),
        (None, _) => Source::default_output(),
    };
    // 1-based on the command line, 0-based everywhere else: nobody calls the
    // left input of an interface "channel 0", and MIDI channels are 1–16 in
    // every DAW there is.
    source.channel = args.channel.map(|n| n.max(1) - 1);
    Ok(source)
}

/// Build the renderer for whatever `live` currently produces.
///
/// Separated out because a source switch can change the *shape* of what it
/// produces — 48 bands become 88 semitones — and the renderer is built around
/// that shape.
fn build_renderer(
    brightness: f32,
    show: &ShowConfig,
    live: &LiveSource,
    led_count: usize,
    point_cap: usize,
) -> Result<Renderer> {
    Renderer::new(
        RenderConfig { brightness, ..Default::default() },
        &show.surface,
        show.layout(),
        show.intensity(),
        Geometry {
            centers: live.centers().to_vec(),
            // See `color::strip` for why these are strip positions rather
            // than bands. Narrowed to what the link will actually carry: the
            // renderer resamples across the axis, so fewer points costs a
            // little spatial resolution, where too many costs every frame.
            points: live.points().min(point_cap),
            leds: led_count,
        },
    )
    .map_err(anyhow::Error::msg)
}

fn list_devices() {
    let devices = source::devices();
    if devices.is_empty() {
        println!("no audio devices or MIDI ports found");
        return;
    }

    println!("sources  (* = system default)\n");
    let has_midi = devices.iter().any(|d| d.kind == SourceKind::Midi);
    let mut kind = None;
    for device in devices {
        if kind != Some(device.kind) {
            println!("  {}", match device.kind {
                SourceKind::Loopback => "loopback — what the PC is playing",
                SourceKind::Input => "input — microphones, line in, interface inputs",
                SourceKind::Midi => "midi — keyboards, and virtual cables from a DAW",
            });
            kind = Some(device.kind);
        }

        let format = match (device.kind, device.channels, device.sample_rate) {
            (SourceKind::Midi, _, _) => "16 channels".into(),
            (_, Some(c), Some(r)) => format!("{c} ch @ {r:.0} Hz"),
            // Nearly always an endpoint something else already holds, which is
            // information rather than a reason to hide it.
            _ => "unavailable — in use by another application?".into(),
        };
        println!("    {} {:<44} {format}", if device.is_default { "*" } else { " " }, device.name);
        println!("      --source \"{}\"", device.id);
    }

    println!("\n  A DAW driving an interface over ASIO owns it outright, so its loopback goes");
    println!("  silent. Select that interface's input to hear what is plugged into it, or");
    println!("  send it notes instead — see --midi.");

    if !has_midi {
        // Not a failure — nothing is plugged in and nothing has been set up.
        // This is where most people meet MIDI here, so it is worth the space.
        println!("\n  No MIDI inputs. A keyboard appears here as soon as it is plugged in.");
        println!("  To capture FL Studio's MIDI Out plugin there is one setup step, because");
        println!("  Windows cannot join a MIDI output to a MIDI input on its own:");
        println!("    1. install loopMIDI and create a port in it");
        println!("    2. in FL: Options -> MIDI settings -> Output, enable that port and note");
        println!("       the port number it is given");
        println!("    3. set the MIDI Out plugin's Port to that number");
        println!("  The loopMIDI port then appears above like any other input.");
    }
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
    live: &mut LiveSource,
    base: &EngineConfig,
    mut show: ShowConfig,
    renderer: &mut Renderer,
    link: &mut dyn Link,
    led_count: usize,
    point_cap: usize,
    server: Option<&UiServer>,
) -> Result<()> {
    let mut leds = vec![[0u8; 3]; led_count];
    let mut last_draw = Instant::now();
    let frame = Duration::from_millis(1000 / TARGET_FPS);
    let mut first = true;
    let mut dropped = 0u64;
    let mut status = String::new();

    // The initial config is applied through the same path an edit takes, so
    // startup cannot diverge from what the editor would produce.
    live.apply(&show, &show, true);

    loop {
        // Before the idle check, not after: a stream that has died delivers
        // nothing at all, so anything gated on samples arriving would never
        // report it. Without this the only symptom is bars that quietly stop.
        if let Some(fault) = live.take_fault() {
            status = format!("source: {fault}");
            announce_input(server, live, Some(fault));
        }

        // Drained before the audio, not after, and so on every pass rather than
        // only on the ones that complete an analysis frame.
        //
        // The order matters more than it looks: a silent source delivers no
        // samples at all, so commands gated behind audio arriving would never be
        // seen — and the one command that most needs to get through is the one
        // that moves off a source which has gone silent. A dead loopback would
        // otherwise be unescapable from the editor.
        //
        // Still before rendering, so an edit takes effect on this frame rather
        // than the next.
        if let Some(server) = server {
            for cmd in server.commands() {
                match cmd {
                    Command::Config { config } => {
                        let next = *config;
                        match renderer.set_surface(&next.surface) {
                            Ok(()) => {
                                // A config edit can move the note grid, which
                                // is the one case where an edit changes the
                                // shape the renderer was built around.
                                if live.apply(&next, &show, false) {
                                    match build_renderer(
                                        renderer.config().brightness,
                                        &next,
                                        live,
                                        led_count,
                                        point_cap,
                                    ) {
                                        Ok(r) => *renderer = r,
                                        Err(e) => status = format!("config rejected: {e}"),
                                    }
                                }
                                show = next.clone();
                                status.clear();
                                server.update_state(|s| {
                                    s.band_count = live.points().min(point_cap);
                                    s.config = next;
                                });
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
                    Command::SetSource { source } => {
                        match switch_source(
                            live, renderer, base, &show, &source, led_count, point_cap,
                        ) {
                            Ok(()) => {
                                status.clear();
                                server.update_state(|s| s.band_count = live.points().min(point_cap));
                                announce_input(Some(server), live, None);
                            }
                            // The old source is still running — opening the new
                            // one is what failed, and the alternative to keeping
                            // it is silence plus a message.
                            Err(e) => {
                                let message = format!("{e:#}");
                                status = format!("source: {message}");
                                announce_input(Some(server), live, Some(message));
                            }
                        }
                    }
                    Command::ListSources => {
                        // Costs a mix-format query per audio endpoint, so it
                        // happens when asked and not on a timer. The capture
                        // ring is a quarter second deep; this fits inside it
                        // comfortably.
                        let devices = source::devices();
                        server.announce_state(|s| s.input.devices = devices);
                    }
                    Command::RequestState => {}
                }
            }
        }

        if !live.poll() {
            // A source is genuinely idle when nothing is playing through it, and
            // loopback in particular delivers nothing at all rather than
            // silence. Not an error, so the loop just comes back around — with
            // the sleep keeping it off a core while it waits.
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }

        let pixels = renderer.render(live.levels());
        if !link.send(pixels)? {
            dropped += 1;
        }

        // The analyser runs far faster than any display needs; redrawing and
        // republishing at its rate would just burn a core on formatting.
        if last_draw.elapsed() >= frame {
            expand_bands_to_leds(pixels, &mut leds);

            if let Some(server) = server {
                let strip = encode_strip(&leds);
                let levels = live.levels().to_vec();
                let centers = live.centers().to_vec();
                let sample_rate = live.sample_rate();
                let out_of_range = live.out_of_range().unwrap_or(0);
                server.publish(|snap| {
                    snap.levels = levels;
                    snap.strip = strip;
                    snap.dropped_frames = dropped;
                    snap.notes_out_of_range = out_of_range;
                    snap.sample_rate = sample_rate;
                    snap.connected = true;
                    // Compared rather than assigned: the axis only moves on a
                    // source switch or a note-range edit, and a Vec assignment
                    // per frame would allocate for nothing.
                    if snap.centers != centers {
                        snap.centers = centers;
                    }
                });
            }

            draw(live.levels(), &leds, dropped, &status, first);
            last_draw = Instant::now();
            first = false;
        }
    }
}

/// Move to a different source, live.
///
/// The new one is opened *before* the old is dropped, so a source that cannot be
/// opened leaves the running one untouched rather than trading a working strip
/// for an error message.
///
/// The renderer is rebuilt unconditionally rather than only when the grid
/// changes shape. A switch already costs an open and a reset, the centres move
/// on nearly every one of them, and the alternative is a comparison that is
/// wrong once and dark forever.
fn switch_source(
    live: &mut LiveSource,
    renderer: &mut Renderer,
    base: &EngineConfig,
    show: &ShowConfig,
    source: &Source,
    led_count: usize,
    point_cap: usize,
) -> Result<()> {
    // A request that resolves to what is already open is answered in place. For
    // MIDI that is not a shortcut but a requirement — see `LiveSource::retune`.
    if live.retune(source) {
        return Ok(());
    }

    let mut next = LiveSource::open(source, base, show, CAPTURE_BUFFER_SECS)?;
    next.apply(show, show, true);
    let rebuilt = build_renderer(renderer.config().brightness, show, &next, led_count, point_cap)?;

    *live = next;
    *renderer = rebuilt;
    Ok(())
}

/// Tell the editor what is actually being listened to.
///
/// The device list is deliberately left alone — it is refreshed only when asked
/// for, and a switch does not change what exists.
fn announce_input(server: Option<&UiServer>, live: &LiveSource, error: Option<String>) {
    let Some(server) = server else { return };
    server.announce_state(|s| {
        s.input.source = live.source().clone();
        s.input.device_name = live.device_name().to_string();
        s.input.kind = live.kind();
        s.input.channels = live.channels();
        s.input.sample_rate = live.sample_rate();
        s.input.error = error;
    });
}

/// Listen for a fixed duration and report what arrived, then exit.
///
/// Separates the three states worth distinguishing during bring-up: the source
/// failed to open, it opened but nothing is coming through, or something is.
/// Loopback legitimately delivers nothing while the endpoint is idle, and so
/// does a MIDI port with nobody playing, so silence is not by itself a fault.
fn probe(live: &mut LiveSource, secs: f64) -> Result<()> {
    match live {
        LiveSource::Midi(_) => probe_midi(live, secs),
        LiveSource::Audio(_) => probe_audio(live, secs),
    }
}

fn probe_audio(live: &mut LiveSource, secs: f64) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs_f64(secs);
    let (mut samples, mut frames) = (0usize, 0usize);
    let mut peak = 0.0f32;
    let mut hottest = vec![0.0f32; live.levels().len()];

    while Instant::now() < deadline {
        // Both questions are asked every pass, and separately: the device
        // delivering samples and the analyser completing a frame are different
        // failures with different causes, and a probe that conflated them would
        // point at the wrong one.
        let framed = live.poll();
        let block = live.last_block();
        if block.is_empty() {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        samples += block.len();
        peak = block.iter().fold(peak, |a, &s| a.max(s.abs()));

        if framed {
            frames += 1;
            for (h, &l) in hottest.iter_mut().zip(live.levels()) {
                if l > *h {
                    *h = l;
                }
            }
        }
    }

    let expected = live.sample_rate() * secs;
    println!("probe: {secs:.1} s");
    println!(
        "  samples  {samples} ({:.0}% of the {expected:.0} expected)",
        100.0 * samples as f64 / expected
    );
    println!("  frames   {frames} analysis frames");
    println!("  peak     {peak:.4} ({:.1} dBFS)", 20.0 * peak.max(1e-9).log10());

    if let Some(fault) = live.take_fault() {
        println!("\n  The stream failed while probing: {fault}");
        return Ok(());
    }

    if samples == 0 {
        println!();
        match live.kind() {
            SourceKind::Loopback => {
                println!("  Stream opened but delivered nothing. WASAPI loopback is idle when the");
                println!("  output endpoint is idle — play some audio and probe again. If a DAW");
                println!("  is driving this device over ASIO, nothing will ever arrive here;");
                println!("  probe the interface's input instead, or send MIDI with --midi.");
            }
            _ => {
                println!("  Stream opened but delivered nothing, which a capture endpoint should");
                println!("  never do — it sends silence when idle. Check Windows microphone");
                println!("  privacy settings, and that nothing else holds the device.");
            }
        }
        return Ok(());
    }
    if peak < 1e-6 {
        println!("\n  Samples arrived but all were silent (digital silence is still delivered).");
        return Ok(());
    }

    let Some(engine) = live.engine() else { return Ok(()) };
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

/// The MIDI bring-up diagnostic: is anything arriving at all, on which channel,
/// and is it inside the range being displayed.
///
/// This is the flag to reach for when FL Studio's MIDI Out plugin appears to do
/// nothing, because it separates "the port is wrong" from "the notes are
/// arriving on a channel or in an octave that is being filtered out".
fn probe_midi(live: &mut LiveSource, secs: f64) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs_f64(secs);
    let mut peak = 0.0f32;
    let mut lit = 0usize;

    while Instant::now() < deadline {
        if !live.poll() {
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }
        let frame_peak = live.levels().iter().fold(0.0f32, |a, &b| a.max(b));
        peak = peak.max(frame_peak);
        if frame_peak > 0.0 {
            lit += 1;
        }
    }

    let out_of_range = live.out_of_range().unwrap_or(0);
    println!("probe: {secs:.1} s");
    println!("  port     {}", live.describe());
    println!("  frames   {lit} with a note sounding");
    println!("  peak     {peak:.2} of full velocity");
    println!("  dropped  {out_of_range} notes outside the displayed range");

    if lit == 0 && out_of_range == 0 {
        println!("\n  The port opened but no notes arrived.");
        println!("  · From a keyboard: check it is this port and not another one it also");
        println!("    presents, and that nothing else has the port open.");
        println!("  · From FL Studio: the MIDI Out plugin sends to a MIDI *output*, and this");
        println!("    listens on a MIDI *input* — the two only meet through a virtual cable.");
        println!("    Install loopMIDI, create a port, enable it under Options -> MIDI");
        println!("    settings -> Output, note the port number it is given, and set the MIDI");
        println!("    Out plugin's Port to that number.");
        println!("  · If a channel filter is set, everything on the other fifteen is dropped");
        println!("    before it is ever counted. Probe again without --channel.");
    } else if lit == 0 {
        println!("\n  Notes are arriving, but all of them are outside the range being shown.");
        println!("  Widen the note range in the editor, or transpose what is being played.");
    }
    Ok(())
}

fn print_header(live: &LiveSource, link: &dyn Link, led_count: usize, point_cap: usize) {
    println!("DJLED");
    println!("  source   {}", live.describe());
    println!("  output   {}", link.describe());

    match live.engine() {
        Some(engine) => {
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
        }
        None => {
            println!("  notes    {} semitones across {led_count} LEDs", live.centers().len());
        }
    }

    // The failure this prevents is silent and looks like a hardware fault: the
    // firmware drops any frame over its own limit without replying, so the
    // spectrum, the preview and the bars above all stay correct while the strip
    // goes dark. Points are narrowed to fit instead, and that is worth saying
    // out loud because it is a real loss of resolution.
    if live.points() > point_cap {
        println!(
            "  NOTE     {} points narrowed to {point_cap} for the wire — the firmware's\n\
             {:11}MAX_BANDS. Raise it in firmware/djled/djled.ino and reflash for\n\
             {:11}the full resolution.",
            live.points(),
            "",
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
