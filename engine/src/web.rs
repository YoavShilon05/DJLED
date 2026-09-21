//! Serves the editor itself, over HTTP, on a port nobody chose.
//!
//! # Why the engine serves it at all
//!
//! In development the editor comes from vite on 5173 and talks to the engine on
//! 9001. A release has no vite and no terminal: it is a tray icon that starts
//! with the machine, so "open the editor" has to mean a URL that already works.
//! The assets are baked into the binary at compile time (see `build.rs`) and
//! handed out from here, which is also what makes the release one file.
//!
//! # Why the port is random
//!
//! Nothing links to this and nothing bookmarks it — the tray menu is how it is
//! opened, and the menu knows the port because this process chose it. So it
//! binds `:0` and takes whatever the OS gives. A fixed port would be one more
//! thing to collide with on a machine that is, by definition, also running a
//! DAW.
//!
//! The WebSocket port is *not* random, and that asymmetry is deliberate: the
//! dev editor and the three smoke scripts all connect to 9001 by name. Instead
//! the page is told its socket on the way out, by injecting one line into
//! `index.html` — so a page served from here connects to the engine that served
//! it, and a page served by vite falls back to the default.
//!
//! # The server
//!
//! One accept thread, one short-lived thread per connection, no keep-alive. The
//! whole site is a handful of files fetched once, and a browser that wants six
//! at a time gets six threads for a few milliseconds each. Nothing here is on
//! the real-time path; the strip does not wait on a page load.
//!
//! It binds loopback only. This is a service on someone's PC, not a web server.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};

include!(concat!(env!("OUT_DIR"), "/ui_assets.rs"));

/// A cap on the request line, because the headers are read into memory before
/// anything is decided and the only client is a browser asking for a file.
const MAX_REQUEST: usize = 16 * 1024;

/// Long enough for a browser that has opened a connection and not yet decided
/// what to ask for, short enough that a stalled one does not hold a thread.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// A running editor server. Dropping it does not stop the threads — they live
/// as long as the process, like the UI server's, because the editor is
/// available for exactly as long as the engine is.
pub struct WebServer {
    port: u16,
}

impl WebServer {
    /// The port the OS gave us. The tray menu opens `http://127.0.0.1:{port}/`.
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/", self.port)
    }
}

/// Start serving the bundled editor, told which WebSocket port to point it at.
///
/// Fails if the editor was never bundled, rather than serving 404s: a blank
/// page behind a working tray icon is the hardest version of this to diagnose,
/// and the fix — build `ui/` — is worth saying out loud.
pub fn start(ws_port: u16) -> Result<WebServer> {
    if ASSETS.is_empty() {
        return Err(anyhow!(
            "the editor was not bundled into this build — run `npm run build` in ui/ and rebuild"
        ));
    }

    let listener = TcpListener::bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)))
        .context("could not bind a port for the editor")?;
    let port = listener.local_addr().context("the editor's listener has no address")?.port();

    std::thread::Builder::new()
        .name("editor-http".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                // One per connection and detached: a client that never sends a
                // request must not hold up the one behind it.
                let _ = std::thread::Builder::new()
                    .name("editor-http-conn".into())
                    .spawn(move || serve(stream, ws_port));
            }
        })
        .context("could not spawn the editor's HTTP thread")?;

    Ok(WebServer { port })
}

fn serve(mut stream: TcpStream, ws_port: u16) {
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let Some(path) = read_request_path(&mut stream) else {
        let _ =
            respond(&mut stream, "400 Bad Request", "text/plain; charset=utf-8", b"bad request");
        return;
    };

    match lookup(&path) {
        Some((name, body)) if name == "index.html" => {
            let page = inject_socket(body, ws_port);
            let _ = respond(&mut stream, "200 OK", "text/html; charset=utf-8", &page);
        }
        Some((name, body)) => {
            let _ = respond(&mut stream, "200 OK", content_type(name), body);
        }
        None => {
            let _ = respond(&mut stream, "404 Not Found", "text/plain; charset=utf-8", b"not found");
        }
    }
}

/// The path from the request line, percent-decoding nothing: vite emits asset
/// names that need no escaping, and a path that arrives escaped simply misses
/// and falls back to the page.
fn read_request_path(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..read]);
        // Only the request line is wanted, and it ends at the first newline —
        // waiting for the blank line after the headers would mean waiting on a
        // client that is entitled to take its time.
        if buf.contains(&b'\n') || buf.len() >= MAX_REQUEST {
            break;
        }
    }

    let line = buf.split(|&b| b == b'\n').next()?;
    let line = String::from_utf8_lossy(line);
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    if method != "GET" && method != "HEAD" {
        return None;
    }
    let target = parts.next()?;
    Some(target.split(['?', '#']).next().unwrap_or("/").to_string())
}

/// Resolve a URL path to a bundled file.
///
/// Anything unknown is the page, because the editor is a single-page app and a
/// reload on a route it invented itself must not 404. A path that escapes the
/// bundle cannot: there is no filesystem here to escape onto, only this table.
fn lookup(path: &str) -> Option<(&'static str, &'static [u8])> {
    let key = path.trim_start_matches('/');
    let key = if key.is_empty() { "index.html" } else { key };
    ASSETS
        .iter()
        .find(|(name, _)| *name == key)
        .or_else(|| ASSETS.iter().find(|(name, _)| *name == "index.html"))
        .map(|(name, body)| (*name, *body))
}

/// Tell the page which socket to talk to, by writing one global in front of the
/// bundle.
///
/// Injected rather than built in, because the same `ui/dist` is what vite serves
/// in development against a fixed 9001 — see `ui/src/engine.ts`, which reads
/// this and falls back when it is absent.
fn inject_socket(html: &[u8], ws_port: u16) -> Vec<u8> {
    let tag = format!("<script>window.__DJLED_WS__=\"ws://127.0.0.1:{ws_port}\";</script>");
    let text = String::from_utf8_lossy(html);
    match text.find("</head>") {
        Some(at) => {
            let mut out = String::with_capacity(text.len() + tag.len());
            out.push_str(&text[..at]);
            out.push_str(&tag);
            out.push_str(&text[at..]);
            out.into_bytes()
        }
        // A page with no head is not a page vite built; serve it untouched and
        // let the fallback in `engine.ts` do its job.
        None => html.to_vec(),
    }
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// One response, then the connection closes.
///
/// `no-store` throughout: every file vite emits changes name when it changes
/// content except `index.html`, which does not, and a cached page pointing at
/// an asset from the previous build is a white screen after an upgrade. The
/// files are already in memory on the same machine, so caching buys nothing
/// measurable.
fn respond(stream: &mut TcpStream, status: &str, mime: &str, body: &[u8]) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The injected global has to land inside the document and before the
    /// bundle that reads it — after `</head>` would be a script that runs too
    /// late, and the editor would connect to the fallback port.
    #[test]
    fn the_socket_is_injected_before_the_head_closes() {
        let page = inject_socket(b"<html><head><title>x</title></head><body></body></html>", 9001);
        let page = String::from_utf8(page).unwrap();
        let injected = page.find("__DJLED_WS__").expect("the global is written");
        assert!(injected < page.find("</head>").unwrap());
        assert!(page.contains("ws://127.0.0.1:9001"));
    }

    /// A page the injector does not recognise is served rather than mangled.
    #[test]
    fn a_page_without_a_head_is_left_alone() {
        assert_eq!(inject_socket(b"<p>hello</p>", 9001), b"<p>hello</p>");
    }

    #[test]
    fn content_types_are_by_extension_and_default_to_bytes() {
        assert_eq!(content_type("assets/index-a1b2.js"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(content_type("noextension"), "application/octet-stream");
    }
}
