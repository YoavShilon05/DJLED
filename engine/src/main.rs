//! DJLED engine — capture, analyse, colour, and drive the strip.
//!
//! Runs with or without hardware. Without `--port` it uses a mock link, so the
//! whole pipeline can be watched in the terminal before the Arduino is wired up.

use std::io::Write;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};

use djled_engine::color::{RenderConfig, Renderer};
use djled_engine::hotkeys::Hotkeys;
use djled_engine::link::protocol::TestPattern;
use djled_engine::link::serial::DEFAULT_BAUD;
use djled_engine::link::{expand_bands_to_leds, Link, MockLink, SerialLink};
use djled_engine::presets::{Presets, SLOTS};
use djled_engine::source::{self, LiveSource, Source, SourceKind};
use djled_engine::stack::LiveStack;
use djled_engine::ui::{encode_strip, Command, LayerFrame, State, UiServer};
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

    /// Where the twelve presets are kept. Defaults to
    /// %APPDATA%\djled\presets.json.
    #[arg(long, value_name = "PATH")]
    presets: Option<std::path::PathBuf>,

    /// Run without the global ctrl+alt+F1..F12 hotkeys. The presets themselves
    /// still work from the editor; this only gives the combinations back to
    /// whatever else wants them.
    #[arg(long)]
    no_hotkeys: bool,
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
    // The show that was live when the engine last exited comes back, so a
    // restart mid-set puts the wall back where it was rather than at the
    // default. On a first run there is nothing saved and the flags describe the
    // show instead.
    let mut presets = Presets::load(args.presets.clone().unwrap_or_else(Presets::default_path));
    let show = match presets.active_show() {
        // A saved show carries a source per layer, which is more specific than
        // anything the command line can say — so the flags only override it
        // when one was actually typed. Silently ignoring `--source` would be
        // worse than either.
        Some(saved) => {
            let mut show = saved.clone();
            if let Some(source) = explicit_source(&args)? {
                show.layers[0].source = source;
            }
            show
        }
        // One layer to start with, listening to whatever the flags asked for.
        // The editor grows the stack from there; the command line deliberately
        // does not, because a stack is authored rather than typed.
        None => starting_show(led_count, select_source(&args)?),
    };
    // What the board says it will take, not what the protocol allows. A frame
    // over the firmware's own limit is dropped at the MCU without a word.
    let point_cap = link.max_points();

    if let Some(secs) = args.probe {
        let mut live =
            LiveSource::open(&show.base().source, &base, show.base(), CAPTURE_BUFFER_SECS)?;
        println!("DJLED\n  source   {}\n", live.describe());
        return probe(&mut live, secs);
    }

    let mut stack = LiveStack::open(&show, &base, CAPTURE_BUFFER_SECS);
    let mut renderer = build_renderer(args.brightness, &stack, led_count, point_cap)?;

    // Registered before the header so the banner reports what was actually
    // claimed, not what was asked for — a combination another application
    // already holds is refused, and a hotkey that silently does nothing is the
    // hardest kind to diagnose.
    let hotkeys = if args.no_hotkeys {
        None
    } else {
        match Hotkeys::start() {
            Ok(h) => Some(h),
            Err(e) => {
                println!("  hotkeys  unavailable: {e}");
                None
            }
        }
    };

    print_header(&stack, link.as_ref(), led_count, point_cap);
    print_presets(&presets, hotkeys.as_ref());

    // The UI is optional by design: the engine is a headless service and
    // nothing about the strip depends on a browser being attached.
    let server = if args.no_ui {
        None
    } else {
        let state = State {
            config: show.clone(),
            brightness: args.brightness,
            led_count,
            point_count: renderer.points(),
            db_floor: base.post.db_floor,
            db_ceil: base.post.db_ceil,
            devices: source::devices(),
            layers: stack.status(),
            presets: presets.info(),
            active_preset: presets.active(),
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
        &mut stack,
        show,
        &mut renderer,
        link.as_mut(),
        led_count,
        point_cap,
        server.as_ref(),
        &mut presets,
        hotkeys.as_ref(),
    )
}

/// The show the engine starts with: one layer, spanning the strip, listening to
/// whatever the flags selected.
fn starting_show(led_count: usize, source: Source) -> ShowConfig {
    let mut show = ShowConfig::spanning(led_count);
    show.layers[0].source = source;
    show
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

/// The source the flags name, but only if any of them were actually typed.
///
/// A saved preset already says what each of its layers listens to, and that is
/// more specific than anything one flag can express — so it stands unless
/// somebody explicitly asked for something else on this run. `--channel` counts
/// as asking: it is meaningless without a source, and reaches the same layer.
fn explicit_source(args: &Args) -> Result<Option<Source>> {
    if args.source.is_none() && !args.input && !args.midi && args.channel.is_none() {
        return Ok(None);
    }
    select_source(args).map(Some)
}

/// Build the renderer for whatever the stack currently produces.
///
/// Separated out because a source switch or a layer added can change the
/// *shape* of what it produces — 48 bands become 88 semitones, one layer
/// becomes three — and the renderer is built around that shape.
///
/// Points are the widest grid any layer has, narrowed to what the link will
/// actually carry. The renderer resamples across the axis, so fewer points
/// costs a little spatial resolution where too many costs every frame.
fn build_renderer(
    brightness: f32,
    stack: &LiveStack,
    led_count: usize,
    point_cap: usize,
) -> Result<Renderer> {
    Renderer::stacked(
        RenderConfig { brightness, ..Default::default() },
        &stack.visuals(),
        stack.points().min(point_cap).max(1),
        led_count,
    )
    .map_err(anyhow::Error::msg)
}

/// Put a show on the wall: the stack first, then the colour half.
///
/// The order is load-bearing. The renderer is built around the shape the stack
/// reports — how many layers, on what grids — so applying the colour half
/// against a shape that has already moved is how a stack ends up painting one
/// layer with another one's field.
///
/// Returns whether the stack changed *shape*, which is the only case that
/// forces a rebuild rather than a reconfigure.
fn apply_show(
    next: &ShowConfig,
    stack: &mut LiveStack,
    renderer: &mut Renderer,
    led_count: usize,
    point_cap: usize,
) -> Result<bool, String> {
    let reshaped = stack.apply(next);
    if reshaped {
        *renderer = build_renderer(renderer.config().brightness, stack, led_count, point_cap)
            .map_err(|e| e.to_string())?;
    } else {
        renderer.set_layers(&stack.visuals())?;
    }
    Ok(reshaped)
}

/// Make a slot live and say what should now be on the wall, or `None` to leave
/// the wall alone.
///
/// `blank_if_empty` is the one difference between the two ways in. Chosen from
/// the editor's preset bar it is true — picking an empty slot from a row of
/// them is a
/// deliberate request for a blank canvas. Struck as a hotkey it is false, so a
/// mis-hit during a set cannot blank the wall.
fn load_preset(
    slot: usize,
    blank_if_empty: bool,
    presets: &mut Presets,
    led_count: usize,
) -> Option<ShowConfig> {
    if slot >= SLOTS || (!blank_if_empty && presets.show(slot).is_none()) {
        return None;
    }
    // Selecting flushes the slot being left, so the switch cannot lose it.
    match presets.select(slot) {
        Some(show) => Some(show.clone()),
        None => Some(ShowConfig::spanning(led_count)),
    }
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
    stack: &mut LiveStack,
    mut show: ShowConfig,
    renderer: &mut Renderer,
    link: &mut dyn Link,
    led_count: usize,
    point_cap: usize,
    server: Option<&UiServer>,
    presets: &mut Presets,
    hotkeys: Option<&Hotkeys>,
) -> Result<()> {
    let mut leds = vec![[0u8; 3]; led_count];
    let mut last_draw = Instant::now();
    let frame = Duration::from_millis(1000 / TARGET_FPS);
    let mut first = true;
    let mut dropped = 0u64;
    let mut status = String::new();

    loop {
        // Before the idle check, not after: a stream that has died delivers
        // nothing at all, so anything gated on samples arriving would never
        // report it. Without this the only symptom is bars that quietly stop —
        // and with a stack, bars that quietly stop on *one row*.
        let faults = stack.take_faults();
        if !faults.is_empty() {
            status = faults
                .iter()
                .map(|(id, fault)| format!("{id}: {fault}"))
                .collect::<Vec<_>>()
                .join("  ");
            let layers = stack.status();
            if let Some(server) = server {
                server.announce_state(|s| s.layers = layers);
            }
        }

        // Drained before the audio, not after, and so on every pass rather than
        // only on the ones that complete an analysis frame.
        //
        // The order matters more than it looks: a silent source delivers no
        // samples at all, so commands gated behind audio arriving would never be
        // seen — and the one command that most needs to get through is the one
        // that moves a layer off a source which has gone silent. A dead loopback
        // would otherwise be unescapable from the editor.
        //
        // Still before rendering, so an edit takes effect on this frame rather
        // than the next.
        if let Some(server) = server {
            for cmd in server.commands() {
                match cmd {
                    Command::Config { config } => {
                        let next = *config;
                        match apply_show(&next, stack, renderer, led_count, point_cap) {
                            Ok(reshaped) => {
                                // Every edit lands in the live slot; there is
                                // no save button, and so nothing is ever in
                                // flight when a hotkey replaces the show.
                                // `filled` is true once per slot ever — the
                                // edit that stops it reading as empty.
                                let filled = presets.store(&next);
                                show = next.clone();
                                status.clear();
                                let (points, layers) = (renderer.points(), stack.status());
                                let listing = filled.then(|| presets.info());
                                server.update_state(|s| {
                                    s.point_count = points;
                                    s.layers = layers;
                                    s.config = next;
                                    if let Some(listing) = listing {
                                        s.presets = listing;
                                    }
                                });
                                // Pushed only when the stack changed shape, and
                                // that condition is doing real work in both
                                // directions. A layer just added has no device
                                // resolved yet as far as the editor knows, and
                                // nothing else would ever tell it — so its row
                                // would read "offline" until a reconnect.
                                // Announcing on every edit instead would echo a
                                // config back at drag rate and have the editor
                                // fighting the user's own hands.
                                if reshaped || filled {
                                    server.announce_state(|_| {});
                                }
                            }
                            // Reported through the status line rather than
                            // stdout: printing here would tear the display. The
                            // rest of the config goes with it, so a half-applied
                            // edit is never left behind.
                            Err(e) => status = format!("config rejected: {e}"),
                        }
                    }
                    Command::Brightness { value } => {
                        let mut cfg = renderer.config().clone();
                        cfg.brightness = value.clamp(0.0, 1.0);
                        renderer.set_config(cfg);
                        server.update_state(|s| s.brightness = value.clamp(0.0, 1.0));
                    }
                    Command::ListSources => {
                        // Costs a mix-format query per audio endpoint, so it
                        // happens when asked and not on a timer. The capture
                        // ring is a quarter second deep; this fits inside it
                        // comfortably.
                        let devices = source::devices();
                        // A rescan is what someone does after plugging the
                        // missing interface back in, so it is also the natural
                        // moment to retry the layers that could not open.
                        if stack.retry_failed(&show) {
                            match build_renderer(
                                renderer.config().brightness,
                                stack,
                                led_count,
                                point_cap,
                            ) {
                                Ok(r) => {
                                    *renderer = r;
                                    status.clear();
                                }
                                Err(e) => status = format!("config rejected: {e}"),
                            }
                        }
                        let layers = stack.status();
                        server.announce_state(|s| {
                            s.devices = devices;
                            s.layers = layers;
                        });
                    }
                    // The preset bar's half of the feature; the keyboard's
                    // half is below, and both end up here.
                    Command::SelectPreset { slot } => {
                        if let Some(next) = load_preset(slot, true, presets, led_count) {
                            match apply_show(&next, stack, renderer, led_count, point_cap) {
                                Ok(_) => {
                                    show = next.clone();
                                    status.clear();
                                    let (points, layers) = (renderer.points(), stack.status());
                                    let (listing, active) = (presets.info(), presets.active());
                                    // Announced rather than merely updated: the
                                    // whole show has been replaced by something
                                    // other than the editor's own hands, which
                                    // is the one case it has to be told about.
                                    server.announce_state(|s| {
                                        s.point_count = points;
                                        s.layers = layers;
                                        s.config = next;
                                        s.presets = listing;
                                        s.active_preset = active;
                                    });
                                }
                                Err(e) => status = format!("preset {} rejected: {e}", slot + 1),
                            }
                        }
                    }
                    Command::RenamePreset { slot, name } => {
                        presets.rename(slot, &name);
                        let listing = presets.info();
                        server.announce_state(|s| s.presets = listing);
                    }
                    Command::RequestState => {}
                }
            }
        }

        // Drained on every pass, beside the UI's commands and for the same
        // reason: the moment a preset most needs switching is the moment the
        // source has gone silent and nothing else is arriving.
        //
        // Outside the `server` block, because the hotkeys are the half of this
        // that works with no editor attached — which is the whole reason they
        // are registered with the OS rather than handled in the browser.
        if let Some(hotkeys) = hotkeys {
            for slot in hotkeys.drain() {
                let Some(next) = load_preset(slot, false, presets, led_count) else {
                    // An empty slot, said out loud: a hotkey that does nothing
                    // and reports nothing is indistinguishable from one that
                    // failed to register.
                    status = format!("{} is empty", Hotkeys::label(slot));
                    continue;
                };
                match apply_show(&next, stack, renderer, led_count, point_cap) {
                    Ok(_) => {
                        show = next.clone();
                        status.clear();
                        if let Some(server) = server {
                            let (points, layers) = (renderer.points(), stack.status());
                            let (listing, active) = (presets.info(), presets.active());
                            server.announce_state(|s| {
                                s.point_count = points;
                                s.layers = layers;
                                s.config = next;
                                s.presets = listing;
                                s.active_preset = active;
                            });
                        }
                    }
                    Err(e) => status = format!("preset {} rejected: {e}", slot + 1),
                }
            }
        }

        // Coalesced writes: an edit marks the store dirty and this is where it
        // reaches the disk, at most once every 750 ms rather than once per
        // pointer move. Costs one comparison when there is nothing to save.
        if presets.tick() {
            if let Some(e) = presets.error() {
                status = e.to_string();
            }
        }

        if !stack.poll() {
            // A source is genuinely idle when nothing is playing through it, and
            // loopback in particular delivers nothing at all rather than
            // silence. Not an error, so the loop just comes back around — with
            // the sleep keeping it off a core while it waits.
            std::thread::sleep(Duration::from_millis(2));
            continue;
        }

        let pixels = renderer.render_stack(&stack.all_levels());
        if !link.send(pixels)? {
            dropped += 1;
        }

        // The analyser runs far faster than any display needs; redrawing and
        // republishing at its rate would just burn a core on formatting.
        if last_draw.elapsed() >= frame {
            expand_bands_to_leds(pixels, &mut leds);

            if let Some(server) = server {
                let strip = encode_strip(&leds);
                let frames: Vec<LayerFrame> = show
                    .layers
                    .iter()
                    .enumerate()
                    .map(|(i, layer)| LayerFrame {
                        id: layer.id.clone(),
                        levels: stack.levels(i).to_vec(),
                        centers: stack.centers(i).to_vec(),
                    })
                    .collect();
                server.publish(|snap| {
                    snap.layers = frames;
                    snap.strip = strip;
                    snap.dropped_frames = dropped;
                    snap.connected = true;
                });
            }

            draw(stack, &leds, dropped, &status, first);
            last_draw = Instant::now();
            first = false;
        }
    }
}

/// Listen for a fixed duration and report what arrived, then exit.
///
/// Separates the three states worth distinguishing during bring-up: the source
/// failed to open, it opened but nothing is coming through, or something is.
/// Loopback legitimately delivers nothing while the endpoint is idle, and so
/// does a MIDI port with nobody playing, so silence is not by itself a fault.
fn probe(live: &mut LiveSource, secs: f64) -> Result<()> {
    if live.is_midi() { probe_midi(live, secs) } else { probe_audio(live, secs) }
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

fn print_header(stack: &LiveStack, link: &dyn Link, led_count: usize, point_cap: usize) {
    println!("DJLED");
    println!("  source   {}", stack.describe(0));
    println!("  output   {}", link.describe());

    match stack.engine(0) {
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
            println!("  notes    {} semitones across {led_count} LEDs", stack.centers(0).len());
        }
    }

    // The failure this prevents is silent and looks like a hardware fault: the
    // firmware drops any frame over its own limit without replying, so the
    // spectrum, the preview and the bars above all stay correct while the strip
    // goes dark. Points are narrowed to fit instead, and that is worth saying
    // out loud because it is a real loss of resolution.
    if stack.points() > point_cap {
        println!(
            "  NOTE     {} points narrowed to {point_cap} for the wire, the firmware own\n\
             {:11}MAX_BANDS. Raise it in firmware/djled/djled.ino and reflash for\n\
             {:11}the full resolution.",
            stack.points(),
            "",
            ""
        );
    }

    println!("\n  play something. ctrl-c to quit.\n");
}

/// What the twelve slots hold and which keys reach them.
///
/// Printed after the header rather than inside it because a preset can be
/// switched with no editor open, and someone doing that from the keyboard alone
/// needs to be told two things the engine cannot show any other way: which slot
/// is live, and which combinations were refused by another application. A
/// hotkey that was never registered is otherwise silent — indistinguishable
/// from one that fired and did nothing.
fn print_presets(presets: &Presets, hotkeys: Option<&Hotkeys>) {
    let info = presets.info();
    let filled = info.iter().filter(|p| p.stored).count();
    let live = &info[presets.active()];

    println!(
        "  presets  {} of {SLOTS} saved · live: {} ({})",
        filled,
        live.name,
        Hotkeys::label(presets.active())
    );
    println!("           {}", presets.path().display());

    match hotkeys {
        Some(keys) if keys.refused().is_empty() => {
            println!("           ctrl+alt+F1..F12 switch presets, from any window")
        }
        Some(keys) => {
            // Named individually, because "some hotkeys failed" is not
            // actionable and "ctrl+alt+F4 is taken" is.
            let taken: Vec<String> = keys.refused().iter().map(|&s| Hotkeys::label(s)).collect();
            println!("           ctrl+alt+F1..F12 switch presets, except {}", taken.join(", "));
            println!("           — those are held by another application, which will not give");
            println!("             them up while it is running.");
        }
        None => println!("           hotkeys off; switch presets in the editor"),
    }

    if let Some(e) = presets.error() {
        println!("  WARNING  {e}");
    }

    println!();
}

/// The terminal display: the bottom layer bars over the composited strip.
///
/// Only the bottom layer is plotted, deliberately. The strip underneath is the
/// whole stack, which is the thing worth watching without a browser; drawing
/// every layer would turn a glance into a reading exercise, and the editor is
/// where a stack is meant to be looked at.
fn draw(stack: &LiveStack, leds: &[[u8; 3]], dropped: u64, status: &str, first: bool) {
    let levels = stack.levels(0);
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
        "  {} LEDs   {} {} on {} {}   dropped frames: {dropped}   {status}\x1b[K\n",
        leds.len(),
        stack.len(),
        if stack.len() == 1 { "layer" } else { "layers" },
        stack.feed_count(),
        if stack.feed_count() == 1 { "device" } else { "devices" },
    ));

    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_all(out.as_bytes());
    let _ = stdout.flush();
}
