//! Bakes the built editor into the binary.
//!
//! A release has to be one file that can sit in `%LOCALAPPDATA%` and start with
//! the machine — a folder of loose assets beside it is one `xcopy` away from a
//! blank page, and the editor is the only way to author a show. So `ui/dist` is
//! read at compile time and emitted as `include_bytes!` entries; see `web.rs`
//! for the server that hands them out.
//!
//! A missing or empty `ui/dist` is not an error. The engine is a headless
//! service and builds perfectly well without a browser front end — `cargo test`
//! and every CLI diagnostic have nothing to do with it — so the table simply
//! comes out empty and `web::start` says the editor was not bundled. Failing
//! the build instead would mean `cargo test` needed npm.

use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets this");
    let dist = Path::new(&manifest).join("..").join("ui").join("dist");

    // Rebuilds when the editor is rebuilt. Named on the directory as well as on
    // each file, so a file *added* by a later `vite build` is noticed too — a
    // per-file list alone cannot see something that did not exist yet.
    println!("cargo:rerun-if-changed={}", dist.display());

    let mut table = String::from("pub static ASSETS: &[(&str, &[u8])] = &[\n");
    if dist.is_dir() {
        collect(&dist, &dist, &mut table);
    }
    table.push_str("];\n");

    let out = Path::new(&std::env::var("OUT_DIR").expect("cargo sets this")).join("ui_assets.rs");
    std::fs::write(&out, table).expect("could not write the asset table");
}

/// Depth-first walk, emitting one entry per file keyed by its path relative to
/// `dist` with forward slashes — that key is compared against a request path,
/// and a URL does not have backslashes in it whatever Windows thinks.
fn collect(root: &Path, dir: &Path, out: &mut String) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, out);
            continue;
        }
        let Ok(rel) = path.strip_prefix(root) else { continue };
        let key = rel.components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        println!("cargo:rerun-if-changed={}", path.display());
        // Raw strings: a Windows path is full of backslashes and none of the
        // paths vite emits contain a quote.
        let _ = writeln!(out, "    ({key:?}, include_bytes!(r\"{}\")),", path.display());
    }
}
