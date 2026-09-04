//! System-wide `ctrl+alt+F1` … `ctrl+alt+F12`, one per preset slot.
//!
//! # Why these are global and not a keydown handler in the editor
//!
//! The moment a preset is worth switching is the moment nobody is looking at
//! the editor — the browser is behind a DAW, or minimised, or was never opened.
//! A shortcut that only works when a particular window has focus is not a
//! shortcut for this. `RegisterHotKey` claims the combination from the whole
//! desktop, so the key works with the editor closed and equally well with it
//! focused: the OS delivers the message here before any application sees it.
//!
//! `ctrl+alt+F1`…`F12` rather than something shorter because the combination
//! has to be one no DAW, browser or media player has already taken, and one
//! that cannot be struck by accident while playing. Nothing here is reachable
//! from a single key.
//!
//! # The thread
//!
//! A hotkey registered against a null window is owned by the *calling thread*
//! and its `WM_HOTKEY` is posted to that thread's message queue — so
//! registration and the message pump have to be the same thread, and it must be
//! a thread that does nothing else. It is not the analysis thread: a message
//! pump blocks, and the strip does not wait on the keyboard.
//!
//! Slots arrive on a channel the engine drains alongside the UI's commands, so
//! a hotkey and an editor edit take exactly the same path into the show.
//!
//! # Refusal is normal
//!
//! `RegisterHotKey` fails if another application already holds a combination,
//! and there is nothing to be done about it from here — that application will
//! not give it up. So a refusal is reported rather than treated as an error:
//! eleven working hotkeys and one that belongs to something else is a usable
//! state, and being told which one is missing is the difference between that
//! and a key that seems broken.

use std::sync::mpsc::{self, Receiver};

use anyhow::Result;

use crate::presets::SLOTS;

/// Registered hotkeys and the slots they have fired.
pub struct Hotkeys {
    fired: Receiver<usize>,
    /// Slots whose combination was refused, in order. Held so the startup
    /// banner can name them.
    refused: Vec<usize>,
}

impl Hotkeys {
    /// Register all twelve and start pumping.
    ///
    /// Blocks until the registrations have been attempted — a few microseconds
    /// — so the caller can print what it got rather than what it asked for.
    pub fn start() -> Result<Self> {
        let (tx, fired) = mpsc::channel();
        let refused = imp::spawn(tx)?;
        Ok(Self { fired, refused })
    }

    /// Slots struck since the last drain. Non-blocking, like the UI's command
    /// drain, and for the same reason: the loop that calls this is rendering.
    pub fn drain(&self) -> impl Iterator<Item = usize> + '_ {
        self.fired.try_iter()
    }

    pub fn refused(&self) -> &[usize] {
        &self.refused
    }

    /// How the combination for a slot reads, for anything that has to say it
    /// out loud. 0-based slot, 1-based key, which is the only place those two
    /// meet.
    pub fn label(slot: usize) -> String {
        format!("ctrl+alt+F{}", slot + 1)
    }
}

#[cfg(windows)]
mod imp {
    use std::sync::mpsc::Sender;

    use anyhow::{Context, Result};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, VK_F1,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, WM_HOTKEY, WM_USER,
    };

    use super::SLOTS;

    /// Registration and the pump on one thread, because a null-window hotkey
    /// belongs to whichever thread registered it. Returns the slots that were
    /// refused, once they are known.
    pub fn spawn(tx: Sender<usize>) -> Result<Vec<usize>> {
        let (ready, registered) = std::sync::mpsc::channel();

        std::thread::Builder::new()
            .name("hotkeys".into())
            .spawn(move || {
                let mut msg: MSG = unsafe { std::mem::zeroed() };

                // Forces the thread's message queue into existence before
                // anything is posted to it. A queue is created lazily on the
                // first message call, and `RegisterHotKey` is not one — a
                // hotkey struck in the window between the two would be
                // delivered to a queue that does not exist yet and dropped.
                unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), WM_USER, WM_USER, PM_NOREMOVE) };

                let mut refused = Vec::new();
                for slot in 0..SLOTS {
                    // MOD_NOREPEAT: holding the keys down is one switch, not
                    // sixty a second. Every switch rebuilds a stack.
                    let ok = unsafe {
                        RegisterHotKey(
                            std::ptr::null_mut(),
                            slot as i32,
                            MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
                            (VK_F1 as u32) + slot as u32,
                        )
                    };
                    if ok == 0 {
                        refused.push(slot);
                    }
                }

                if ready.send(refused).is_err() {
                    return; // the caller gave up waiting; nothing to pump for
                }

                // `GetMessageW` returns 0 for WM_QUIT and -1 for an error, and
                // there is no recovering from either — the thread is only here
                // to pump.
                while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
                    if msg.message == WM_HOTKEY {
                        let slot = msg.wParam as usize;
                        if slot < SLOTS && tx.send(slot).is_err() {
                            return; // the engine is gone
                        }
                    }
                }
            })
            .context("could not spawn the hotkey thread")?;

        registered
            .recv()
            .context("the hotkey thread stopped before it registered anything")
    }
}

/// Everywhere else there is no `RegisterHotKey`, and no pretending there is.
///
/// The rest of the engine has Windows assumptions it does not gate, but those
/// fail at *run* time on a device that is not there. This one would fail at
/// compile time and take the build with it, so it is the one that is gated —
/// presets still work from the dropdown, which is all this changes.
#[cfg(not(windows))]
mod imp {
    use std::sync::mpsc::Sender;

    use anyhow::{Result, anyhow};

    pub fn spawn(_tx: Sender<usize>) -> Result<Vec<usize>> {
        Err(anyhow!("global hotkeys need Windows; select presets in the editor instead"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one thing here that is testable without a desktop: slots are
    /// 0-based and function keys are 1-based, and the label is where that is
    /// translated. Off by one and every hotkey in the UI names the wrong key.
    #[test]
    fn labels_number_function_keys_from_one() {
        assert_eq!(Hotkeys::label(0), "ctrl+alt+F1");
        assert_eq!(Hotkeys::label(11), "ctrl+alt+F12");
        assert_eq!(Hotkeys::label(SLOTS - 1), "ctrl+alt+F12");
    }
}
