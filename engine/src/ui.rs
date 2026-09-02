//! WebSocket bridge to the editor UI.
//!
//! The engine stays a headless service and the UI is just a client. That keeps
//! the real-time path completely insulated: nothing the browser does can block
//! analysis, and the UI can be reloaded, closed, or never opened at all without
//! the strip noticing.
//!
//! # Threading
//!
//! One thread accepts connections and one more per client. Clients read a shared
//! snapshot under a mutex — the analysis thread writes it once per frame and
//! never waits on a reader, because the lock is held only for a memcpy.
//!
//! Commands travel the other way over an unbounded channel, drained by the
//! engine between frames. A UI that floods commands therefore cannot stall
//! rendering; it just queues.
//!
//! # Wire format
//!
//! JSON, because the volume is trivial on localhost and being able to watch the
//! traffic in devtools is worth more than the bytes. The one concession is the
//! strip preview, sent as a hex string rather than a nested array — at 600 LEDs
//! that is the difference between roughly 1 KB and 8 KB per frame.

use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::show::ShowConfig;
use crate::source::DeviceInfo;
use crate::stack::LayerStatus;

pub const DEFAULT_PORT: u16 = 9001;

/// How often each client is served. The analyser runs far faster; the UI is a
/// display, and 30 fps is past the point of visible improvement.
const SEND_INTERVAL: Duration = Duration::from_millis(33);

/// One layer's analysis, as the editor plots it.
///
/// Per layer rather than per frame because two layers can be listening to two
/// different devices on two different grids — 48 audio bands under 88 semitones
/// of MIDI — and the editor draws whichever one is selected.
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerFrame {
    /// Matches `Layer::id`, so a row keeps its plot across a reorder.
    pub id: String,
    /// Bar level per band, 0..1.
    pub levels: Vec<f32>,
    /// Band centre frequencies, so the UI can label its axis without
    /// reimplementing the band scale.
    pub centers: Vec<f32>,
}

/// What the UI is shown each frame.
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// Bottom layer first, matching the order of the show.
    pub layers: Vec<LayerFrame>,
    /// The strip as the firmware will drive it — the whole stack composited,
    /// which is the one thing no single layer can tell you.
    pub strip: String,
    pub connected: bool,
    pub dropped_frames: u64,
}

/// What the UI can change.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Command {
    /// Replace the whole show configuration, layers and all.
    ///
    /// One message rather than one per control, including on the hot path where
    /// dragging a keyframe sends one of these per pointer move. Applying it is
    /// cheap because each stage compares against what it already has, and a
    /// single message means the two sides cannot end up disagreeing about which
    /// half of an edit landed — which matters more with a stack than it did
    /// without one, since a reorder and a recolour can arrive together.
    Config { config: Box<ShowConfig> },
    /// Master brightness, 0..1. Not per layer: it is the power budget for the
    /// whole strip, and a layer that dimmed the ones under it would be a blend
    /// mode rather than a brightness.
    Brightness { value: f32 },
    /// Rescan the endpoints, e.g. after plugging an interface in. Handled by the
    /// engine rather than here so every client sees one list, and so the COM
    /// enumeration stays on the thread that owns the capture.
    ///
    /// Also retries any layer whose device would not open, because a rescan is
    /// exactly what someone does after plugging the missing one back in.
    ListSources,
    /// Ask for the current configuration, e.g. after a reload.
    RequestState,
}

/// Sent once on connect, on request, and whenever the engine announces a change,
/// so the UI can populate its editor with whatever the engine is actually using
/// rather than guessing.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub config: ShowConfig,
    pub brightness: f32,
    pub led_count: usize,
    /// Colours on the wire per frame, which is what the whole stack is rendered
    /// at. Narrowed to what the board says it will take.
    pub point_count: usize,
    /// Analyser dB window. The EQ is authored in these terms, so the editor
    /// needs them to preview a gain at the right size.
    pub db_floor: f32,
    pub db_ceil: f32,
    /// Everything selectable, as of the last scan — audio endpoints and MIDI
    /// ports in one list, because they are one choice. Global rather than per
    /// layer: what exists does not depend on who is listening to it.
    pub devices: Vec<DeviceInfo>,
    /// What each layer's selection actually resolved to, and what went wrong if
    /// anything did. Bottom layer first, matching the show.
    pub layers: Vec<LayerStatus>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Outbound<'a> {
    Frame(&'a Snapshot),
    State(&'a State),
    Error { message: String },
}

pub struct UiServer {
    snapshot: Arc<Mutex<Snapshot>>,
    state: Arc<Mutex<Announced>>,
    commands: Receiver<Command>,
    port: u16,
}

/// State with a counter, so a client can tell what it holds is stale without the
/// server tracking who has seen what.
struct Announced {
    state: State,
    version: u64,
}

impl UiServer {
    /// Bind and start accepting. Returns an error if the port is taken, rather
    /// than silently running without a UI.
    pub fn start(port: u16, initial: State) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .with_context(|| format!("could not bind 127.0.0.1:{port} for the UI server"))?;

        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let state = Arc::new(Mutex::new(Announced { state: initial, version: 0 }));
        let (tx, commands) = mpsc::channel();

        {
            let snapshot = Arc::clone(&snapshot);
            let state = Arc::clone(&state);
            std::thread::Builder::new()
                .name("ui-accept".into())
                .spawn(move || accept_loop(listener, snapshot, state, tx))
                .context("could not spawn the UI accept thread")?;
        }

        Ok(Self { snapshot, state, commands, port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Publish the latest frame. Called once per analysis frame; holds the lock
    /// only long enough to swap the contents.
    pub fn publish(&self, update: impl FnOnce(&mut Snapshot)) {
        if let Ok(mut guard) = self.snapshot.lock() {
            update(&mut guard);
        }
    }

    /// Keep the advertised state in step with what the engine is really using,
    /// without telling anyone.
    ///
    /// For changes the UI itself originated: a config edit sends one of these
    /// per pointer move, and echoing them back at drag rate would have the
    /// editor fighting the user's own hands. New clients still get the current
    /// values, which is the whole point of holding them.
    pub fn update_state(&self, update: impl FnOnce(&mut State)) {
        if let Ok(mut guard) = self.state.lock() {
            update(&mut guard.state);
        }
    }

    /// Change the state *and* push it to everyone connected.
    ///
    /// For changes the UI cannot predict — which endpoint is really being
    /// captured, at what rate, and whether opening it failed.
    pub fn announce_state(&self, update: impl FnOnce(&mut State)) {
        if let Ok(mut guard) = self.state.lock() {
            update(&mut guard.state);
            guard.version = guard.version.wrapping_add(1);
        }
    }

    /// Non-blocking drain of everything the UI has asked for.
    pub fn commands(&self) -> impl Iterator<Item = Command> + '_ {
        self.commands.try_iter()
    }
}

/// Hex-encode RGB triplets for the strip preview.
pub fn encode_strip(leds: &[[u8; 3]]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(leds.len() * 6);
    for px in leds {
        for &c in px {
            out.push(HEX[(c >> 4) as usize] as char);
            out.push(HEX[(c & 0x0F) as usize] as char);
        }
    }
    out
}

fn accept_loop(
    listener: TcpListener,
    snapshot: Arc<Mutex<Snapshot>>,
    state: Arc<Mutex<Announced>>,
    tx: Sender<Command>,
) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let snapshot = Arc::clone(&snapshot);
        let state = Arc::clone(&state);
        let tx = tx.clone();

        // A panic in one client thread must not take down the engine, so each
        // is isolated and simply ends if its connection misbehaves.
        let _ = std::thread::Builder::new()
            .name("ui-client".into())
            .spawn(move || {
                if let Err(e) = serve_client(stream, snapshot, state, tx) {
                    // Disconnects are routine; log at a level that does not
                    // clutter the terminal display.
                    let _ = e;
                }
            });
    }
}

fn serve_client(
    stream: TcpStream,
    snapshot: Arc<Mutex<Snapshot>>,
    state: Arc<Mutex<Announced>>,
    tx: Sender<Command>,
) -> Result<()> {
    stream.set_nodelay(true).ok();
    let mut socket = tungstenite::accept(stream).context("websocket handshake failed")?;

    // Reads must not block the send loop, so the socket is polled instead.
    socket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(1)))
        .ok();

    let mut sent = send_state(&mut socket, &state)?;

    loop {
        // Drain anything the client has sent.
        loop {
            match socket.read() {
                Ok(tungstenite::Message::Text(text)) => {
                    match serde_json::from_str::<Command>(&text) {
                        Ok(Command::RequestState) => sent = send_state(&mut socket, &state)?,
                        Ok(cmd) => {
                            if tx.send(cmd).is_err() {
                                return Ok(()); // engine is gone
                            }
                        }
                        Err(e) => {
                            let msg = serde_json::to_string(&Outbound::Error {
                                message: format!("could not parse command: {e}"),
                            })?;
                            socket.send(tungstenite::Message::Text(msg.into()))?;
                        }
                    }
                }
                Ok(tungstenite::Message::Close(_)) => return Ok(()),
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    break
                }
                Err(e) => return Err(e.into()),
            }
        }

        // Cheap enough to check every frame: one lock and an integer compare,
        // against a counter the engine only moves when the UI could not have
        // known what changed.
        let stale = {
            let guard = state.lock().map_err(|_| anyhow::anyhow!("state lock poisoned"))?;
            guard.version != sent
        };
        if stale {
            sent = send_state(&mut socket, &state)?;
        }

        let payload = {
            let guard = snapshot.lock().map_err(|_| anyhow::anyhow!("snapshot lock poisoned"))?;
            serde_json::to_string(&Outbound::Frame(&guard))?
        };
        socket.send(tungstenite::Message::Text(payload.into()))?;

        std::thread::sleep(SEND_INTERVAL);
    }
}

/// Send the current state, returning the version that went out.
fn send_state(
    socket: &mut tungstenite::WebSocket<TcpStream>,
    state: &Arc<Mutex<Announced>>,
) -> Result<u64> {
    let (payload, version) = {
        let guard = state.lock().map_err(|_| anyhow::anyhow!("state lock poisoned"))?;
        (serde_json::to_string(&Outbound::State(&guard.state))?, guard.version)
    };
    socket.send(tungstenite::Message::Text(payload.into()))?;
    Ok(version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::SurfaceConfig;

    #[test]
    fn strip_encodes_as_lowercase_hex() {
        assert_eq!(encode_strip(&[[0, 0, 0]]), "000000");
        assert_eq!(encode_strip(&[[255, 255, 255]]), "ffffff");
        assert_eq!(encode_strip(&[[0x1A, 0x2B, 0x3C], [4, 5, 6]]), "1a2b3c040506");
        assert_eq!(encode_strip(&[]), "");
    }

    /// 600 LEDs is where this project is headed; the hex encoding is what keeps
    /// the per-frame payload reasonable there.
    #[test]
    fn strip_payload_stays_small_at_600_leds() {
        let leds = vec![[0x80u8; 3]; 600];
        assert_eq!(encode_strip(&leds).len(), 3600);
    }

    /// The stack crosses the wire whole, sources included — which is the change
    /// layers made to this message. There is no longer a command that moves one
    /// device: a layer names what it listens to, and that travels with the rest
    /// of what the layer looks like.
    #[test]
    fn commands_parse_from_the_ui_wire_format() {
        let cmd: Command = serde_json::from_str(r#"{"type":"brightness","value":0.5}"#).unwrap();
        assert!(matches!(cmd, Command::Brightness { value } if (value - 0.5).abs() < 1e-6));

        let cmd: Command = serde_json::from_str(r#"{"type":"requestState"}"#).unwrap();
        assert!(matches!(cmd, Command::RequestState));

        let cmd: Command = serde_json::from_str(r#"{"type":"listSources"}"#).unwrap();
        assert!(matches!(cmd, Command::ListSources));

        // Deeper delimiter: the hex colour contains `"#`, which would close a
        // single-hash raw string.
        let json = r##"{"type":"config","config":{"layers":[
            {"id":"a","surface":{"keyframes":[{"x":0,"y":1,"color":"#ff0000"}],"sigma":0.25}}
        ]}}"##;
        let cmd: Command = serde_json::from_str(json).unwrap();
        match cmd {
            Command::Config { config } => {
                assert_eq!(config.layers.len(), 1);
                assert_eq!(config.base().surface.keyframes.len(), 1);
                assert_eq!(config.base().surface.keyframes[0].color, "#ff0000");
            }
            other => panic!("parsed as {other:?}"),
        }
    }

    /// A whole stack, with a different device on each layer. The device ids are
    /// opaque and full of punctuation, so this is also checking they survive the
    /// trip intact — that used to be its own message and is now part of this one.
    #[test]
    fn a_stack_of_layers_parses_with_its_sources() {
        let json = r#"{"type":"config","config":{"layers":[
            {
                "id":"bass","name":"Bass","opacity":1,
                "source":{"id":"wasapi:{0.0.1.00000000}.{9d}","kind":"loopback","channel":0},
                "mirror":true,"threshold":-50,"sampleLength":512
            },
            {
                "id":"keys","name":"Keys","opacity":0.5,"enabled":false,
                "source":{"id":"midi:loopMIDI Port","kind":"midi","channel":9}
            },
            { "id":"amb", "source":{"kind":"input"} }
        ]}}"#;

        match serde_json::from_str::<Command>(json).unwrap() {
            Command::Config { config } => {
                assert_eq!(config.layers.len(), 3);

                let bass = &config.layers[0];
                assert!(bass.mirror);
                assert_eq!(bass.threshold, -50.0);
                assert_eq!(bass.hop(), 512);
                assert_eq!(bass.source.id.as_deref(), Some("wasapi:{0.0.1.00000000}.{9d}"));
                assert_eq!(bass.source.channel, Some(0));

                let keys = &config.layers[1];
                assert_eq!(keys.source.kind, crate::source::SourceKind::Midi);
                assert_eq!(keys.source.id.as_deref(), Some("midi:loopMIDI Port"));
                assert_eq!(keys.source.channel, Some(9));
                assert!(!keys.contributes(), "a disabled layer still claims to contribute");

                // No id: follow the system default for that direction.
                assert_eq!(config.layers[2].source, crate::source::Source::default_input());
            }
            other => panic!("parsed as {other:?}"),
        }
    }

    /// A config written before layers existed had every field at the top level.
    /// It has to keep arriving as the one-layer show it always was, or a saved
    /// preset would be dropped as unparseable.
    #[test]
    fn a_pre_stack_config_command_still_parses() {
        let json = r#"{"type":"config","config":{"mirror":true,"threshold":-50,"sampleLength":512}}"#;
        match serde_json::from_str::<Command>(json).unwrap() {
            Command::Config { config } => {
                assert_eq!(config.layers.len(), 1);
                assert!(config.base().mirror);
                assert_eq!(config.base().threshold, -50.0);
                assert_eq!(config.base().hop(), 512);
            }
            other => panic!("parsed as {other:?}"),
        }
    }


    /// A malformed command must be reported, not crash the client thread.
    #[test]
    fn malformed_commands_are_rejected_cleanly() {
        assert!(serde_json::from_str::<Command>(r#"{"type":"nonsense"}"#).is_err());
        assert!(serde_json::from_str::<Command>("not json").is_err());
    }

    /// Round-trips the surface through the wire format, so an edit made in the
    /// UI reconstructs exactly.
    #[test]
    fn surface_survives_a_round_trip() {
        let original = SurfaceConfig::default();
        let json = serde_json::to_string(&original).unwrap();
        let back: SurfaceConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(back.keyframes.len(), original.keyframes.len());
        for (a, b) in original.keyframes.iter().zip(&back.keyframes) {
            assert_eq!(a.color, b.color);
            assert!((a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6);
        }
        assert!((back.sigma - original.sigma).abs() < 1e-6);
    }
}
