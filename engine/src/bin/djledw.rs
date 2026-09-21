//! The same engine, with no console: what an installed release runs as.
//!
//! # Why this is a second binary
//!
//! The subsystem — whether Windows gives a process a console window — is a flag
//! in the executable header, decided when it is linked. It cannot be a command
//! line option, so the choice is either a console that flashes up on every boot
//! or a second binary, and a background service that blinks a terminal at the
//! desktop every time the machine starts is not one anybody would keep.
//!
//! The *code* is not duplicated: `main.rs` is included as a module, so `djled`
//! and `djledw` are one program with two headers. `DJLED_TRAY` is how this one
//! says which it is, since it has no command line of its own to put `--tray` on
//! — it is started by a shortcut, not typed.
//!
//! # Why stdout is redirected before anything else
//!
//! A windowed process has no standard handles. `println!` panics when the write
//! fails, so the engine's first line of output would be its last — the header
//! it prints at startup would take the process down before the strip ever lit.
//! Pointing the handles at a log file fixes that and is worth having anyway: a
//! service nobody can see needs somewhere to say that the serial port was
//! missing.

#![windows_subsystem = "windows"]

// The console binary's source, compiled a second time behind this header. The
// path is relative to this file, which is what puts it one directory up.
#[path = "../main.rs"]
mod cli;

fn main() -> anyhow::Result<()> {
    logging::redirect();
    unsafe { std::env::set_var("DJLED_TRAY", "1") };
    cli::main()
}

#[cfg(windows)]
mod logging {
    use std::io::Write;
    use std::os::windows::io::IntoRawHandle;
    use std::path::PathBuf;

    use windows_sys::Win32::System::Console::{
        STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    };

    /// Past this, the log is started again rather than appended to. It is a
    /// handful of lines per run — a file this size means something is wrong and
    /// repeating, and the useful half of it is the recent half.
    const MAX_LOG_BYTES: u64 = 1 << 20;

    /// Point stdout and stderr at `%LOCALAPPDATA%\DJLED\djled.log`.
    ///
    /// Everything here fails quietly. There is nowhere to report a failure to
    /// open the log *to*, and a strip that lights is worth more than a record
    /// of why it did not — the engine runs fine with its output going nowhere,
    /// which is what the invalid handles it starts with already mean.
    pub fn redirect() {
        let Some(path) = log_path() else { return };
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));

        // Appended across restarts, so a crash loop reads as one story;
        // started again once it is large, which is the only bound a file
        // nobody ever looks at will otherwise have.
        let large = std::fs::metadata(&path).map(|m| m.len() > MAX_LOG_BYTES).unwrap_or(false);

        // `append` rather than `write` is not a detail: the same handle backs
        // both std streams, and appending is what makes every write go to the
        // end of the file rather than to wherever the other one left the
        // position.
        let Ok(file) = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .truncate(false)
            .open(&path)
        else {
            return;
        };
        if large {
            let _ = file.set_len(0);
        }

        // Leaked on purpose: the handle has to outlive this function — it is
        // the process's stdout for the rest of the run — and dropping the
        // `File` would close it out from under everything that then printed.
        let handle = file.into_raw_handle();

        // Both, and stderr especially: a panic message is the one thing that
        // has to survive, and it does not go to stdout.
        unsafe {
            SetStdHandle(STD_OUTPUT_HANDLE, handle);
            SetStdHandle(STD_ERROR_HANDLE, handle);
        }

        // A run starts where the last one ended, so a log read a week later
        // still says which paragraph is this morning's.
        let _ = std::io::stdout().write_all(b"\n=== djled started ===\n");
    }

    fn log_path() -> Option<PathBuf> {
        let base = std::env::var_os("LOCALAPPDATA").or_else(|| std::env::var_os("APPDATA"))?;
        Some(PathBuf::from(base).join("DJLED").join("djled.log"))
    }
}

/// Nowhere else has a subsystem to hide from, and the attribute above is
/// ignored. The redirect would be too, so it is simply not there.
#[cfg(not(windows))]
mod logging {
    pub fn redirect() {}
}
