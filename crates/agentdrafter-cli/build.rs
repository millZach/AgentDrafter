//! Embeds the viewer's page in the `agentdrafter` binary, for `agentdrafter
//! serve` to serve (docs/viewer.md).
//!
//! The page is built by `npm run build` in `viewer/`, into `viewer/dist/`,
//! which is not committed: CI builds it, and anyone with Node can. When it
//! has been built, every file of it is embedded. When it has not, a page
//! that says how to build it is embedded in its place, so a plain `cargo
//! build` works on a machine without Node, and serves everything but the
//! viewer.

use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo sets it"));
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo sets it"));
    let viewer = manifest.join("..").join("..").join("viewer");
    let dist = viewer.join("dist");
    let placeholder = manifest.join("src").join("viewer-not-built.html");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", placeholder.display());

    let mut files = Vec::new();
    let built = dist.join("index.html").is_file();
    if built {
        // Any change to the built page embeds it again.
        println!("cargo:rerun-if-changed={}", dist.display());
        collect(&dist, &dist, &mut files);
        files.sort();
    } else {
        // The viewer's folder changes when its page is built into it.
        println!("cargo:rerun-if-changed={}", viewer.display());
        files.push(("/index.html".to_owned(), placeholder));
    }

    let mut code = String::from(
        "/// Whether the viewer's page was built into this binary; without it,\n\
         /// a page that says how to build it is served in its place.\n",
    );
    code.push_str(&format!("pub const VIEWER_BUILT: bool = {built};\n\n"));
    code.push_str(
        "/// Each file of the viewer's page: its route, its content type and its\n\
         /// bytes.\n\
         pub static VIEWER_FILES: &[(&str, &str, &[u8])] = &[\n",
    );
    for (route, path) in &files {
        code.push_str(&format!(
            "    ({route:?}, {:?}, include_bytes!({:?})),\n",
            content_type(path),
            path.display().to_string()
        ));
    }
    code.push_str("];\n");
    fs::write(out.join("viewer_files.rs"), code).expect("OUT_DIR is writable");
}

/// Every file under `folder`, each with its route: its path from `root`,
/// with `/` between folders.
fn collect(root: &Path, folder: &Path, files: &mut Vec<(String, PathBuf)>) {
    let mut entries: Vec<_> = fs::read_dir(folder)
        .expect("the viewer's built page can be read")
        .map(|entry| entry.expect("the viewer's built page can be read").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(root, &path, files);
        } else {
            let route: Vec<String> = path
                .strip_prefix(root)
                .expect("under the root")
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            files.push((format!("/{}", route.join("/")), path));
        }
    }
}

/// The content type a file is served with, by its extension.
fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
