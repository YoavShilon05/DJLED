//! Twelve saved shows on disk, one of them live.
//!
//! # Why the engine owns these and not the editor
//!
//! Everything else the editor authors lives in the browser and is pushed down;
//! presets go the other way, and deliberately. A preset is switched *during a
//! set*, from a global hotkey, with the browser minimised or closed — see
//! [`crate::hotkeys`]. Something that has to work with no page open cannot be
//! stored in that page's `localStorage`, so the store is here and the editor
//! became a client of it like it is of everything else the engine knows.
//!
//! That inverts one rule the editor used to rely on: while a client is
//! connected, the *engine's* copy of the show is authoritative and the editor
//! adopts it, rather than the other way round.
//!
//! # There is no save button
//!
//! Editing a preset means selecting it and then editing the show; every config
//! the editor sends is stored into whichever slot is active. This matches how
//! the editor already treats its own state — it has never had a save button
//! either — and it is the only model where a hotkey switch is safe, because
//! there is never an unsaved edit in flight to lose when the show is replaced.
//!
//! The cost is that writes happen at pointer-drag rate, so they are coalesced:
//! an edit marks the store dirty and [`Presets::tick`] flushes at most once
//! every [`WRITE_DELAY`]. A switch flushes first, so the slot being left is
//! always on disk before the slot being entered replaces it in memory.
//!
//! # An empty slot
//!
//! A slot nothing has been authored into is *not* the same as a slot holding
//! the default show, and the difference is only visible at the two ways in:
//!
//! - Chosen in the preset bar, it opens the default show — a blank canvas, which
//!   is what someone deliberately picking an empty slot is asking for.
//! - Struck as a hotkey, it does nothing. A mis-hit during a set must not blank
//!   the wall, and a slot with nothing in it has nothing to show.
//!
//! # Failing to persist is not fatal
//!
//! A read-only profile, a full disk or a hand-mangled file costs the presets,
//! not the strip. Load falls back to twelve empty slots and a write failure is
//! reported through the status line — the engine keeps rendering either way.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::show::ShowConfig;

/// Slots, and therefore hotkeys: `ctrl+alt+F1` … `ctrl+alt+F12`. The keyboard
/// is what fixes this number — there is no thirteenth function key.
pub const SLOTS: usize = 12;

/// How long an edit sits in memory before it reaches the disk. Long enough that
/// dragging a keyframe is one write rather than sixty, short enough that
/// pulling the power after an edit loses nothing anyone would notice.
const WRITE_DELAY: Duration = Duration::from_millis(750);

/// Bumped only if the on-disk shape changes incompatibly. It has not, and a
/// file without it still loads — every field defaults.
const VERSION: u32 = 1;

/// One slot, as it sits on disk.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Slot {
    name: String,
    /// `None` for a slot nothing has been authored into. Not `ShowConfig`'s own
    /// default, because "never used" and "holds the default show" behave
    /// differently — see the module docs.
    show: Option<ShowConfig>,
}

/// What the editor is told about a slot: enough to label one chip of the
/// preset bar, and
/// nothing else. The shows themselves are far too big to send twelve of every
/// time a device is rescanned.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetInfo {
    pub name: String,
    /// Whether anything has been authored here.
    pub stored: bool,
}

/// The whole file.
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Stored {
    version: u32,
    /// Which slot was live when the engine last exited, so a restart comes back
    /// to the show that was on the wall.
    active: usize,
    slots: Vec<Slot>,
}

pub struct Presets {
    path: PathBuf,
    slots: Vec<Slot>,
    active: usize,
    dirty: bool,
    /// When the last write went out. Started in the past so the first edit
    /// after startup is not made to wait for a delay it never began.
    wrote: Instant,
    /// The last write failure, if any. Surfaced rather than swallowed: a
    /// preset silently not being saved is exactly the kind of thing nobody
    /// discovers until the set they needed it for.
    error: Option<String>,
}

impl Presets {
    /// Load from `path`, or start empty.
    ///
    /// Infallible on purpose. A missing file is the normal first run, and a
    /// corrupt one is not worth refusing to start over — the reason is kept in
    /// [`Presets::error`] and the slots come up blank.
    pub fn load(path: PathBuf) -> Self {
        let mut presets = Self {
            path,
            slots: (0..SLOTS).map(|i| Slot { name: default_name(i), show: None }).collect(),
            active: 0,
            dirty: false,
            wrote: Instant::now() - WRITE_DELAY,
            error: None,
        };

        match std::fs::read_to_string(&presets.path) {
            Ok(text) => match serde_json::from_str::<Stored>(&text) {
                Ok(stored) => {
                    for (slot, loaded) in presets.slots.iter_mut().zip(stored.slots) {
                        if !loaded.name.trim().is_empty() {
                            slot.name = loaded.name;
                        }
                        slot.show = loaded.show;
                    }
                    presets.active = stored.active.min(SLOTS - 1);
                }
                Err(e) => presets.error = Some(format!("{} is not readable: {e}", presets.path.display())),
            },
            // Absent is the first run, and says nothing worth reporting. Any
            // other read error does.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => presets.error = Some(format!("could not read {}: {e}", presets.path.display())),
        }

        presets
    }

    /// Where presets live when `--presets` does not say otherwise:
    /// `%APPDATA%\djled\presets.json`, beside every other application's
    /// settings rather than beside the binary, which may be in a build
    /// directory that gets wiped.
    pub fn default_path() -> PathBuf {
        match std::env::var_os("APPDATA") {
            Some(dir) => Path::new(&dir).join("djled").join("presets.json"),
            None => PathBuf::from("djled-presets.json"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// What the editor needs to draw its preset bar.
    pub fn info(&self) -> Vec<PresetInfo> {
        self.slots
            .iter()
            .map(|s| PresetInfo { name: s.name.clone(), stored: s.show.is_some() })
            .collect()
    }

    /// The show in a slot, or `None` if nothing was ever authored there.
    pub fn show(&self, slot: usize) -> Option<&ShowConfig> {
        self.slots.get(slot).and_then(|s| s.show.as_ref())
    }

    /// The live slot's show. `None` on a first run, where the engine falls back
    /// to the show the command line describes.
    pub fn active_show(&self) -> Option<&ShowConfig> {
        self.show(self.active)
    }

    /// Make `slot` live, and say what should now be on the wall.
    ///
    /// The slot being left is flushed first, so the switch cannot lose it. An
    /// empty slot yields `None` and the caller decides — the bar opens a
    /// blank canvas, the hotkey declines.
    pub fn select(&mut self, slot: usize) -> Option<&ShowConfig> {
        if slot < SLOTS && slot != self.active {
            self.flush();
            self.active = slot;
            self.dirty = true;
        }
        self.show(self.active)
    }

    /// Store a show into the live slot. Called for every config the editor
    /// sends, which is why it compares before it dirties: a redundant push
    /// should not cost a file write.
    ///
    /// Returns whether the slot's *listing* changed — which happens exactly
    /// once per slot, when the first show lands in it and it stops reading as
    /// empty. The caller uses that to push a new listing to the editor without
    /// pushing one on every pointer move.
    pub fn store(&mut self, show: &ShowConfig) -> bool {
        let Some(slot) = self.slots.get_mut(self.active) else { return false };
        // Serialised rather than compared field by field, because `ShowConfig`
        // is not `PartialEq` and making it so would mean deciding what an equal
        // float is — which is a question this has no need to answer.
        let same = slot
            .show
            .as_ref()
            .and_then(|held| serde_json::to_string(held).ok())
            .zip(serde_json::to_string(show).ok())
            .is_some_and(|(a, b)| a == b);
        if same {
            return false;
        }
        let was_empty = slot.show.is_none();
        slot.show = Some(show.clone());
        self.dirty = true;
        was_empty
    }

    /// Rename a slot. Blank falls back to the positional name rather than
    /// leaving an unlabelled chip in the preset bar.
    pub fn rename(&mut self, slot: usize, name: &str) {
        let Some(target) = self.slots.get_mut(slot) else { return };
        let name = name.trim();
        let name = if name.is_empty() { default_name(slot) } else { name.to_string() };
        if target.name != name {
            target.name = name;
            self.dirty = true;
        }
    }

    /// Write if there is something to write and the coalescing delay has
    /// passed. Called once per pass of the engine loop; costs one comparison
    /// when there is nothing to do.
    ///
    /// Returns whether a write was attempted, so the caller can look at
    /// [`Presets::error`] only when the answer could have changed.
    pub fn tick(&mut self) -> bool {
        if self.dirty && self.wrote.elapsed() >= WRITE_DELAY {
            self.flush();
            return true;
        }
        false
    }

    /// Write now, if dirty.
    pub fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.wrote = Instant::now();
        self.error = match self.write() {
            Ok(()) => None,
            Err(e) => Some(format!("could not save presets to {}: {e}", self.path.display())),
        };
    }

    /// Write via a temporary and rename over the top, so an interrupted write
    /// leaves the previous twelve presets intact rather than half a file.
    fn write(&self) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let stored = Stored {
            version: VERSION,
            active: self.active,
            slots: self.slots.clone(),
        };
        let json = serde_json::to_string_pretty(&stored)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.path)
    }
}

fn default_name(slot: usize) -> String {
    format!("Preset {}", slot + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch path that does not collide between tests running in parallel.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("djled-preset-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{tag}-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn show_with(threshold: f32) -> ShowConfig {
        let mut show = ShowConfig::spanning(150);
        show.layers[0].threshold = threshold;
        show
    }

    #[test]
    fn a_fresh_store_is_twelve_empty_slots() {
        let presets = Presets::load(scratch("fresh"));
        assert_eq!(presets.info().len(), SLOTS);
        assert!(presets.info().iter().all(|p| !p.stored));
        assert_eq!(presets.info()[0].name, "Preset 1");
        assert_eq!(presets.info()[11].name, "Preset 12");
        assert_eq!(presets.active(), 0);
        assert!(presets.active_show().is_none());
        assert!(presets.error().is_none());
    }

    /// The whole point: what was on the wall comes back after a restart, in the
    /// slot it was in.
    #[test]
    fn slots_and_the_live_one_survive_a_restart() {
        let path = scratch("restart");

        let mut presets = Presets::load(path.clone());
        presets.store(&show_with(-40.0));
        presets.rename(0, "Warm");
        presets.select(4);
        presets.store(&show_with(-20.0));
        presets.flush();

        let reloaded = Presets::load(path.clone());
        assert_eq!(reloaded.active(), 4);
        assert_eq!(reloaded.info()[0].name, "Warm");
        assert!(reloaded.info()[0].stored);
        assert!(reloaded.info()[4].stored);
        assert!(!reloaded.info()[1].stored);
        assert_eq!(reloaded.show(0).unwrap().base().threshold, -40.0);
        assert_eq!(reloaded.active_show().unwrap().base().threshold, -20.0);

        std::fs::remove_file(path).ok();
    }

    /// Switching away must not lose the slot being left, even though writes are
    /// otherwise delayed — the delay is what makes this worth asserting.
    #[test]
    fn switching_flushes_the_slot_being_left() {
        let path = scratch("flush-on-switch");

        let mut presets = Presets::load(path.clone());
        presets.store(&show_with(-33.0));
        presets.select(7);

        let reloaded = Presets::load(path.clone());
        assert_eq!(reloaded.show(0).unwrap().base().threshold, -33.0);

        std::fs::remove_file(path).ok();
    }

    /// Selecting an empty slot reports that it is empty rather than handing
    /// back the show that was already live — which would silently make the
    /// hotkey a no-op that looked like a switch.
    #[test]
    fn an_empty_slot_yields_nothing_to_show() {
        let mut presets = Presets::load(scratch("empty"));
        presets.store(&show_with(-40.0));
        assert!(presets.select(3).is_none());
        assert_eq!(presets.active(), 3);
        // ...and the slot left behind still holds what it held.
        assert_eq!(presets.show(0).unwrap().base().threshold, -40.0);
    }

    /// The editor sends a config per pointer move and most of them change
    /// nothing. Each one must not cost a file write.
    #[test]
    fn storing_an_unchanged_show_does_not_dirty_the_store() {
        let mut presets = Presets::load(scratch("redundant"));
        assert!(presets.store(&show_with(-40.0)), "the first show into a slot fills it");
        presets.flush();

        assert!(!presets.store(&show_with(-40.0)));
        assert!(!presets.dirty, "an identical show marked the store dirty");

        // A different show still dirties the store — it just no longer changes
        // how the slot is listed.
        assert!(!presets.store(&show_with(-41.0)));
        assert!(presets.dirty);
    }

    #[test]
    fn a_blank_name_falls_back_to_the_positional_one() {
        let mut presets = Presets::load(scratch("names"));
        presets.rename(2, "  Bass  ");
        assert_eq!(presets.info()[2].name, "Bass");
        presets.rename(2, "   ");
        assert_eq!(presets.info()[2].name, "Preset 3");
    }

    /// Out of range is ignored rather than panicking: these indices come off
    /// the wire, where a stale editor can name a slot that does not exist.
    #[test]
    fn out_of_range_slots_are_ignored() {
        let mut presets = Presets::load(scratch("range"));
        presets.rename(99, "nowhere");
        presets.select(99);
        assert_eq!(presets.active(), 0);
        assert!(presets.show(99).is_none());
        assert_eq!(presets.info().len(), SLOTS);
    }

    /// A hand-mangled file costs the presets, not the engine.
    #[test]
    fn a_corrupt_file_loads_as_empty_slots_with_a_reason() {
        let path = scratch("corrupt");
        std::fs::write(&path, "{ not json").unwrap();

        let presets = Presets::load(path.clone());
        assert_eq!(presets.info().len(), SLOTS);
        assert!(presets.info().iter().all(|p| !p.stored));
        assert!(presets.error().is_some());

        std::fs::remove_file(path).ok();
    }

    /// A file written before a slot count or a field existed still loads, for
    /// the same reason every `ShowConfig` field carries a default.
    #[test]
    fn a_short_or_partial_file_fills_the_rest_in() {
        let path = scratch("partial");
        std::fs::write(&path, r#"{"active":1,"slots":[{"name":"Only","show":{"threshold":-12}}]}"#)
            .unwrap();

        let presets = Presets::load(path.clone());
        assert_eq!(presets.info().len(), SLOTS);
        assert_eq!(presets.info()[0].name, "Only");
        assert!(presets.info()[0].stored);
        assert!(!presets.info()[1].stored);
        assert_eq!(presets.active(), 1);
        // A pre-stack show folds into one layer here exactly as it does on the
        // wire, because it is the same `Deserialize`.
        assert_eq!(presets.show(0).unwrap().layers.len(), 1);
        assert_eq!(presets.show(0).unwrap().base().threshold, -12.0);

        std::fs::remove_file(path).ok();
    }
}
