//! The viewer's page, as `agentdrafter serve` serves it: the files `npm run
//! build` wrote into `viewer/dist/`, embedded in the binary when it was
//! built (`build.rs`), or a page that says how to build them.
//!
//! These are the page's own files and nothing else: no drawing, no file of
//! the machine's. The page draws a sheet from the display list it asks the
//! HTTP API for, as any client does, with the token when the server needs
//! one.

include!(concat!(env!("OUT_DIR"), "/viewer_files.rs"));

/// One of the page's files.
#[derive(Debug, PartialEq, Eq)]
pub struct File {
    pub content_type: &'static str,
    pub bytes: &'static [u8],
    /// Whether its name carries a fingerprint of its bytes, as the built
    /// page's assets' do, so a browser may keep it for good.
    pub fingerprinted: bool,
}

/// The page's file at `path`, a request's path without its query: `/`
/// is the page itself.
pub fn file(path: &str) -> Option<File> {
    let path = if path == "/" { "/index.html" } else { path };
    VIEWER_FILES
        .iter()
        .find(|(route, _, _)| *route == path)
        .map(|&(route, content_type, bytes)| File {
            content_type,
            bytes,
            fingerprinted: VIEWER_BUILT && route.starts_with("/assets/"),
        })
}

/// What the page may do, as its Content-Security-Policy says: run its own
/// scripts and WebAssembly, use its own styles, and send requests to this
/// server only; nothing inline, nothing from elsewhere, and never inside
/// another site's frame.
pub const POLICY: &str = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; \
     style-src 'self'; img-src 'self' data:; connect-src 'self'; base-uri 'none'; \
     form-action 'none'; frame-ancestors 'none'";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_is_served_at_the_root_and_nothing_else_is_a_file() {
        let page = file("/").unwrap();
        assert_eq!(page.content_type, "text/html; charset=utf-8");
        assert!(!page.fingerprinted, "the page itself is asked for afresh");
        assert_eq!(file("/index.html"), Some(page));
        for path in [
            "/commands",
            "/../Cargo.toml",
            "/assets/",
            "/index.html/",
            "",
            "/x",
        ] {
            assert_eq!(file(path), None, "{path}");
        }
        for (route, content_type, bytes) in VIEWER_FILES {
            assert!(route.starts_with('/'), "{route}");
            assert!(!route.contains(".."), "{route}");
            assert!(!content_type.is_empty() && !bytes.is_empty(), "{route}");
        }
        if VIEWER_BUILT {
            assert!(
                VIEWER_FILES
                    .iter()
                    .any(|(route, kind, _)| route.ends_with(".wasm") && *kind == "application/wasm"),
                "the built page carries its WebAssembly module"
            );
        } else {
            assert_eq!(
                VIEWER_FILES.len(),
                1,
                "only the page that says how to build it"
            );
        }
    }
}
