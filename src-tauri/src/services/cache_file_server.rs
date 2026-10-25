//! Serves the universal cache's image files to the page over loopback HTTP.
//! Tauri's `asset` protocol reads each file inside WebView2's
//! WebResourceRequested handler, which runs on the UI thread, so a cached image
//! waited for whatever that thread was doing while a CDN image did not. Here
//! requests go through WebView2's network service and files are read on the
//! blocking pool, a few at a time. Windows only: the page keeps the asset
//! protocol everywhere else (utils/cacheFileUrl.ts).
//!
//! URL: `http://127.0.0.1:{port}/cache/{token}/{folder}/{file}`. The token is
//! new each launch; only the cache's image folders are served, never the
//! manifest beside them.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Serialize;
use tokio::sync::Semaphore;
use warp::http::{Response, StatusCode};
use warp::Filter;

/// The universal cache folders the page draws from (`CacheType::dir_name`).
const CACHE_DIRS: [&str; 5] = ["badges", "emotes", "third-party-badges", "cosmetics", "streamnook-art"];

/// Files read at once. A history batch asks for dozens together; reading them
/// all at the same moment only queues them on the disk.
const READ_PERMITS: usize = 4;

static READS: Semaphore = Semaphore::const_new(READ_PERMITS);
static SERVER: OnceLock<CacheFileServer> = OnceLock::new();

/// Where the page fetches cache files.
#[derive(Clone, Debug, Serialize)]
pub struct CacheFileServer {
    pub port: u16,
    pub token: String,
}

/// The running server, once bound.
pub fn info() -> Option<CacheFileServer> {
    SERVER.get().cloned()
}

/// Bind once, on a loopback port the OS picks. A failure leaves the page on
/// the asset protocol.
pub fn start() {
    if !cfg!(windows) || SERVER.get().is_some() {
        return;
    }
    tauri::async_runtime::spawn(async {
        let listener = match tokio::net::TcpListener::bind(("127.0.0.1", 0)).await {
            Ok(l) => l,
            Err(e) => {
                log::warn!("[CacheFiles] no loopback port ({e}); cached images stay on the asset protocol");
                return;
            }
        };
        let port = match listener.local_addr() {
            Ok(a) => a.port(),
            Err(e) => {
                log::warn!("[CacheFiles] no local address ({e}); cached images stay on the asset protocol");
                return;
            }
        };
        let token = uuid::Uuid::new_v4().simple().to_string();
        let file_token = token.clone();
        let ping_token = token.clone();
        let files = warp::get()
            .and(warp::path("cache"))
            .and(warp::path::param::<String>())
            .and(warp::path::param::<String>())
            .and(warp::path::param::<String>())
            .and(warp::path::end())
            .and_then(move |tok: String, dir: String, file: String| {
                let allowed = tok == file_token;
                async move {
                    Ok::<_, std::convert::Infallible>(if allowed { serve(dir, file).await } else { status(StatusCode::NOT_FOUND) })
                }
            });
        let ping = warp::get()
            .and(warp::path("cache"))
            .and(warp::path::param::<String>())
            .and(warp::path("ping"))
            .and(warp::path::end())
            .map(move |tok: String| status(if tok == ping_token { StatusCode::NO_CONTENT } else { StatusCode::NOT_FOUND }));
        if SERVER.set(CacheFileServer { port, token }).is_err() {
            return;
        }
        log::info!("[CacheFiles] serving cached images on 127.0.0.1:{port}");
        warp::serve(ping.or(files)).incoming(listener).run().await;
    });
}

fn status(code: StatusCode) -> Response<bytes::Bytes> {
    Response::builder()
        .status(code)
        .header("Access-Control-Allow-Origin", "*")
        .body(bytes::Bytes::new())
        .unwrap_or_else(|_| Response::new(bytes::Bytes::new()))
}

async fn serve(dir: String, file: String) -> Response<bytes::Bytes> {
    let Ok(_permit) = READS.acquire().await else {
        return status(StatusCode::SERVICE_UNAVAILABLE);
    };
    let read = tokio::task::spawn_blocking(move || {
        let root = crate::services::universal_cache_service::get_universal_cache_dir().ok()?;
        let path = resolve(&root, &dir, &file)?;
        std::fs::read(path).ok()
    })
    .await;
    match read {
        Ok(Some(bytes)) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", sniff(&bytes))
            // The URL carries this launch's token, so a stored copy could never
            // be asked for again: keep none (the WebView2 cache would otherwise
            // gain a duplicate of every image each launch).
            .header("Cache-Control", "no-store")
            .header("Access-Control-Allow-Origin", "*")
            .body(bytes::Bytes::from(bytes))
            .unwrap_or_else(|_| status(StatusCode::INTERNAL_SERVER_ERROR)),
        _ => status(StatusCode::NOT_FOUND),
    }
}

/// The file a request names, only when it is a plain file inside one of the
/// cache's image folders.
fn resolve(root: &Path, dir: &str, file: &str) -> Option<PathBuf> {
    if !CACHE_DIRS.contains(&dir) {
        return None;
    }
    let file = urlencoding::decode(file).ok()?;
    if file.is_empty() || file.contains(['/', '\\', ':']) || file.contains("..") {
        return None;
    }
    let canon_root = std::fs::canonicalize(root).ok()?;
    let canon = std::fs::canonicalize(root.join(dir).join(file.as_ref())).ok()?;
    canon.starts_with(&canon_root).then_some(canon)
}

/// The image type from its first bytes (many cached badges are `.bin` files
/// that hold PNG; some cosmetics `.bin` files hold WebP).
fn sniff(b: &[u8]) -> &'static str {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        "image/gif"
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        "image/webp"
    } else if b.len() >= 12 && &b[4..8] == b"ftyp" && (&b[8..12] == b"avif" || &b[8..12] == b"avis") {
        "image/avif"
    } else if is_svg(b) {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    }
}

fn is_svg(b: &[u8]) -> bool {
    let start = b.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(b.len());
    let head = &b[start..b.len().min(start + 256)];
    let head = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    head.starts_with(b"<svg") || (head.starts_with(b"<?xml") && head.windows(4).any(|w| w == b"<svg"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sn-cachefiles-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("emotes")).unwrap();
        std::fs::write(root.join("emotes").join("a@1x.webp"), b"RIFF0000WEBPVP8 ").unwrap();
        std::fs::write(root.join("manifest.json"), b"{}").unwrap();
        root
    }

    #[test]
    fn only_plain_files_in_the_image_folders_resolve() {
        let root = temp_root("scope");
        assert!(resolve(&root, "emotes", "a%401x.webp").is_some(), "encoded @ decodes");
        assert!(resolve(&root, "emotes", "a@1x.webp").is_some());
        assert!(resolve(&root, "emotes", "missing.webp").is_none());
        assert!(resolve(&root, "manifest.json", "x").is_none(), "not an image folder");
        assert!(resolve(&root, "emotes", "..%5Cmanifest.json").is_none());
        assert!(resolve(&root, "emotes", "..%2Fmanifest.json").is_none());
        assert!(resolve(&root, "emotes", "C%3A%5Cx").is_none());
        assert!(resolve(&root, "emotes", "").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn types_come_from_the_bytes() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), "image/png");
        assert_eq!(sniff(b"GIF89a.."), "image/gif");
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), "image/jpeg");
        assert_eq!(sniff(b"RIFF0000WEBPVP8 "), "image/webp");
        assert_eq!(sniff(b"\0\0\0\x1cftypavif0000"), "image/avif");
        assert_eq!(sniff(b"  <svg xmlns=''/>"), "image/svg+xml");
        assert_eq!(sniff(b"<?xml version='1.0'?><svg/>"), "image/svg+xml");
        assert_eq!(sniff(b"hello"), "application/octet-stream");
    }

    #[test]
    fn the_folders_match_the_cache_types() {
        let src = include_str!("universal_cache_service.rs");
        for dir in CACHE_DIRS {
            assert!(src.contains(&format!("=> \"{dir}\"")), "{dir} is a CacheType folder");
        }
    }
}
