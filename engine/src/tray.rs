//! The notification-area icon: what a release looks like when it is running.
//!
//! # Why there is one at all
//!
//! Installed, the engine has no window and no console — it starts with the
//! machine and drives a strip. That leaves two questions with no way to answer
//! them: *is it running*, and *how do I get at it*. The icon is the answer to
//! the first simply by being there, and its menu is the answer to the second:
//! open the editor, restart, stop.
//!
//! # The thread
//!
//! Like `hotkeys`, a message pump on a thread of its own, for the same reason:
//! a tray icon's callbacks arrive as window messages, a pump blocks, and the
//! strip does not wait on the desktop. The window is message-only
//! (`HWND_MESSAGE`) — it exists to receive `WM_TRAYICON` and nothing else, so
//! it is never shown, never in the taskbar and never in alt-tab.
//!
//! Opening the editor is handled here, because it is a shell call that has
//! nothing to do with the engine. Stop and restart are not: they are sent to
//! the run loop on a channel it drains beside the hotkeys, so shutting down
//! goes through the same orderly path as everything else rather than killing
//! the process from under the serial port.
//!
//! # Why the menu is three items
//!
//! Anything the show itself does belongs in the editor, which is one click
//! away. This menu is process control only, and a menu with a tenth item on it
//! is a menu somebody has to read during a set.

use std::sync::mpsc::{self, Receiver};

use anyhow::Result;

/// What the menu asked the engine to do. Opening the editor is not here: it
/// never reaches the engine.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// Shut the engine down. The wall goes dark and the icon goes away.
    Stop,
    /// Shut down and come back — a new process, with the same arguments.
    Restart,
}

/// A live tray icon.
pub struct Tray {
    events: Receiver<TrayEvent>,
    /// The message-only window, as an integer because a raw `HWND` is not
    /// `Send` and this one is owned by a thread that is not this one. Only ever
    /// used to post it a close.
    window: isize,
}

impl Tray {
    /// Put the icon up, pointed at the editor's URL, labelled with what the
    /// engine is driving.
    ///
    /// Both are captured rather than read later because the menu must work with
    /// nothing else running — including with the engine's loop wedged, which is
    /// one of the times somebody most wants the menu.
    ///
    /// `driving` is the link's own description, and it is on the tooltip
    /// because "the editor is alive and the strip is dark" has exactly one
    /// common cause: the engine was started without `--port` and is rendering
    /// into a mock link. Everything downstream of that looks perfect. Hovering
    /// the icon is the shortest path from the symptom to the answer.
    pub fn start(url: String, driving: String) -> Result<Self> {
        let (tx, events) = mpsc::channel();
        let window = imp::spawn(tx, url, driving)?;
        Ok(Self { events, window })
    }

    /// Menu commands since the last drain. Non-blocking, like `Hotkeys::drain`
    /// and drained in the same place, because the loop that calls it is
    /// rendering.
    pub fn drain(&self) -> impl Iterator<Item = TrayEvent> + '_ {
        self.events.try_iter()
    }

    /// Take the icon down.
    ///
    /// Worth doing explicitly: an icon whose process has gone is not removed
    /// by Windows until something makes it repaint, so a restart without this
    /// leaves a dead icon beside the live one until the mouse passes over.
    pub fn remove(&self) {
        imp::close(self.window);
    }
}

#[cfg(windows)]
mod imp {
    use std::sync::mpsc::Sender;

    use anyhow::{Context, Result, anyhow};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, ShellExecuteW,
        Shell_NotifyIconW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
        DispatchMessageW, GetCursorPos, GetMessageW, HWND_MESSAGE, IDI_APPLICATION, LoadIconW,
        MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, PostQuitMessage, RegisterClassW, SendMessageW,
        SW_SHOWNORMAL, SetForegroundWindow, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON,
        TrackPopupMenu, TranslateMessage, WM_APP, WM_CLOSE, WM_DESTROY, WM_LBUTTONDBLCLK,
        WM_RBUTTONUP, WNDCLASSW,
    };

    use super::TrayEvent;

    /// The icon's callback message. Anything from `WM_APP` up is the
    /// application's to define, and this window receives nothing else.
    const WM_TRAYICON: u32 = WM_APP + 1;

    const ID_OPEN: u32 = 1;
    const ID_RESTART: u32 = 2;
    const ID_STOP: u32 = 3;

    /// The icon's id within this window. One icon, so one id, and the same one
    /// has to be given to `NIM_DELETE` or the icon is not the one removed.
    const ICON_ID: u32 = 1;

    thread_local! {
        /// The pump's end of the channel and the URL to open, reachable from
        /// the window procedure — which Windows calls with no room for a
        /// context pointer of ours unless we go through the window, and it runs
        /// on this thread by definition.
        static CONTEXT: std::cell::RefCell<Option<(Sender<TrayEvent>, String)>> =
            const { std::cell::RefCell::new(None) };
    }

    /// Create the window, add the icon, and pump until asked to stop. Returns
    /// the window handle once the icon is actually up, so a caller that goes on
    /// to print "tray: ready" is not printing a lie.
    pub fn spawn(tx: Sender<TrayEvent>, url: String, driving: String) -> Result<isize> {
        let (ready, started) = std::sync::mpsc::channel::<Result<isize, String>>();

        std::thread::Builder::new()
            .name("tray".into())
            .spawn(move || {
                CONTEXT.with(|c| *c.borrow_mut() = Some((tx, url)));

                let window = match create(&driving) {
                    Ok(w) => w,
                    Err(e) => {
                        let _ = ready.send(Err(e.to_string()));
                        return;
                    }
                };

                if ready.send(Ok(window as isize)).is_err() {
                    // The caller gave up waiting; take the icon back down
                    // rather than leaving one nobody owns.
                    remove_icon(window);
                    return;
                }

                let mut msg: MSG = unsafe { std::mem::zeroed() };
                while unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
                    unsafe {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            })
            .context("could not spawn the tray thread")?;

        started
            .recv()
            .context("the tray thread stopped before it put an icon up")?
            .map_err(|e| anyhow!(e))
    }

    /// Take the icon down and stop the pump.
    ///
    /// Sent rather than posted, which is the difference between an icon that is
    /// gone and one that usually is: a sent message blocks until the pump has
    /// handled it, and this is called on the way out of `main` — a posted one
    /// would race the process's own exit and lose about as often as it won,
    /// leaving a dead icon behind until something made the tray repaint.
    pub fn close(window: isize) {
        if window != 0 {
            unsafe { SendMessageW(window as HWND, WM_CLOSE, 0, 0) };
        }
    }

    fn create(driving: &str) -> Result<HWND> {
        let class = wide("DJLEDTray");
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };

        let mut wc: WNDCLASSW = unsafe { std::mem::zeroed() };
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = instance;
        wc.lpszClassName = class.as_ptr();
        // A class name already registered by an earlier start in this process
        // comes back 0; creating the window then still works, so the return is
        // deliberately not checked.
        unsafe { RegisterClassW(&wc) };

        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                wide("DJLED").as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            )
        };
        if window.is_null() {
            return Err(anyhow!("could not create the tray's message window"));
        }

        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = window;
        data.uID = ICON_ID;
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        data.uCallbackMessage = WM_TRAYICON;
        // The stock application icon. The engine has no icon resource of its
        // own and a missing one is a blank square in the tray, which reads as
        // broken rather than as unbranded.
        data.hIcon = unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) };
        let tip = wide(&format!("DJLED — {driving}"));
        // Truncated rather than refused: the tooltip is the least important
        // thing here and a field one character short of the buffer is still a
        // working icon. The null is part of `tip`, so a truncation that lands
        // on it is still terminated.
        let fits = tip.len().min(data.szTip.len());
        data.szTip[..fits].copy_from_slice(&tip[..fits]);

        if unsafe { Shell_NotifyIconW(NIM_ADD, &data) } == 0 {
            return Err(anyhow!("the notification area refused the icon"));
        }
        Ok(window)
    }

    fn remove_icon(window: HWND) {
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = window;
        data.uID = ICON_ID;
        unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
    }

    unsafe extern "system" fn wndproc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // Every call below is a Win32 one, and the whole body is the
        // unsafe block the 2024 edition wants said out loud.
        unsafe {
            match message {
                WM_TRAYICON => {
                    // The mouse event is in the low word of the *lparam*; the
                    // high word is the icon id, which is ours by construction.
                    match (lparam as u32) & 0xffff {
                        WM_RBUTTONUP => show_menu(window),
                        // The obvious gesture for the obvious action.
                        WM_LBUTTONDBLCLK => open_editor(),
                        _ => {}
                    }
                    0
                }
                WM_CLOSE => {
                    remove_icon(window);
                    PostQuitMessage(0);
                    0
                }
                WM_DESTROY => {
                    PostQuitMessage(0);
                    0
                }
                _ => DefWindowProcW(window, message, wparam, lparam),
            }
        }
    }

    fn show_menu(window: HWND) {
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            return;
        }
        // Greyed rather than absent when there is no editor to open — the
        // port it would have been on was taken, almost always by a second
        // engine. A menu item that is there and does nothing is the version of
        // this nobody can diagnose; one that is missing is the version nobody
        // notices.
        let (open_flags, open_label) = match editor_url() {
            Some(_) => (MF_STRING, "Open editor"),
            None => (MF_STRING | MF_GRAYED, "Editor unavailable — see the log"),
        };
        unsafe {
            AppendMenuW(menu, open_flags, ID_OPEN as usize, wide(open_label).as_ptr());
            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
            AppendMenuW(menu, MF_STRING, ID_RESTART as usize, wide("Restart").as_ptr());
            AppendMenuW(menu, MF_STRING, ID_STOP as usize, wide("Stop").as_ptr());
        }

        let mut point = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut point) };

        // Documented requirement: without the foreground window being ours the
        // menu does not dismiss when the mouse is clicked away from it, and is
        // left hanging over the desktop.
        unsafe { SetForegroundWindow(window) };

        // TPM_RETURNCMD hands the chosen id straight back rather than posting a
        // WM_COMMAND, so the whole menu is one function with no second entry
        // point to keep in step.
        let chosen = unsafe {
            TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_BOTTOMALIGN,
                point.x,
                point.y,
                0,
                window,
                std::ptr::null(),
            )
        } as u32;
        unsafe { DestroyMenu(menu) };

        match chosen {
            ID_OPEN => open_editor(),
            ID_RESTART => send(TrayEvent::Restart),
            ID_STOP => send(TrayEvent::Stop),
            _ => {}
        }
    }

    /// Hand the URL to the shell, which opens it in whatever the default
    /// browser is. Not `Command::new("cmd")`: that flashes a console window,
    /// which is the one thing an installed background service must never do.
    fn open_editor() {
        let Some(url) = editor_url() else { return };
        {
            let open = wide("open");
            let target = wide(&url);
            unsafe {
                ShellExecuteW(
                    std::ptr::null_mut(),
                    open.as_ptr(),
                    target.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOWNORMAL as i32,
                )
            };
        }
    }

    /// The URL the menu opens, or `None` when the editor never came up — an
    /// empty string is how `main` says so, since a tray with no menu at all
    /// would be a worse answer to a port collision than a tray with one item
    /// greyed out.
    fn editor_url() -> Option<String> {
        CONTEXT.with(|c| {
            let borrowed = c.borrow();
            let (_, url) = borrowed.as_ref()?;
            if url.is_empty() { None } else { Some(url.clone()) }
        })
    }

    fn send(event: TrayEvent) {
        CONTEXT.with(|c| {
            let borrowed = c.borrow();
            if let Some((tx, _)) = borrowed.as_ref() {
                let _ = tx.send(event);
            }
        });
    }

    /// Null-terminated UTF-16, which is what every `…W` call here wants.
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

/// Everywhere else there is no notification area, and no pretending there is.
/// Gated for the same reason `hotkeys` is: this one would fail to *compile*
/// rather than at run time, and take the build with it.
#[cfg(not(windows))]
mod imp {
    use std::sync::mpsc::Sender;

    use anyhow::{Result, anyhow};

    use super::TrayEvent;

    pub fn spawn(_tx: Sender<TrayEvent>, _url: String, _driving: String) -> Result<isize> {
        Err(anyhow!("the tray icon needs Windows; run the engine in a terminal instead"))
    }

    pub fn close(_window: isize) {}
}
