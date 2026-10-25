//! Serves the universal cache's image files to the page over loopback HTTP.
//! Tauri's `asset` protocol reads each file inside WebView2's
//! WebResourceRequested handler, which runs on the UI thread, so a cached image
//! waited for whatever that thread was doing while a CDN image did not. Here
//! requests go through WebView2's network service and files are read on the
//! blocking pool, a few at a time. Windows only: the page keeps the asset
//! protocol everywhere else (utils/cacheFileUrl.ts).
//!
//! URL: `http://127.0.0.1:{port}/cache/{token}/{folder}/{file}`. The token is
//! kept per install and the port is reused when it is free, so a cached
//! image's URL is the same from one launch to the next and the WebView can keep
//! its copy. Only the cache's image folders are served, never the manifest
//! beside them.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Serialize;
use tokio::sync::Semaphore;
use warp::http::{Response, StatusCode};
use warp::{Filter, Rejection, Reply};

/// File in the app data folder that holds the install's URL token.
const TOKEN_FILE: &str = "cache_server_token";

/// File in the app data folder that holds the last bound port.
const PORT_FILE: &str = "cache_server_port";

/// The universal cache folders the page draws from (`CacheType::dir_name`).
const CACHE_DIRS: [&str; 5] = ["badges", "emotes", "third-party-badges", "cosmetics", "streamnook-art"];

/// Ports derived from the token and tried after the saved one.
const FALLBACK_PORTS: usize = 8;

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

/// Bind once, on the loopback port used last time when it is free and on one
/// the OS picks otherwise. A failure leaves the page on the asset protocol.
pub fn start() {
    if !cfg!(windows) || SERVER.get().is_some() {
        return;
    }
    tauri::async_runtime::spawn(async {
        let root = match crate::services::universal_cache_service::get_universal_cache_dir() {
            Ok(r) => r,
            Err(e) => {
                log::warn!("[CacheFiles] no cache folder ({e}); cached images stay on the asset protocol");
                return;
            }
        };
        let state_dir = crate::services::cache_service::get_app_data_dir().ok();
        let token = install_token(state_dir.as_deref());
        let listener = match bind_listener(state_dir.as_deref(), &token).await {
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
        if SERVER.set(CacheFileServer { port, token: token.clone() }).is_err() {
            return;
        }
        log::info!("[CacheFiles] serving cached images on 127.0.0.1:{port}");
        warp::serve(routes(token, root)).incoming(listener).run().await;
    });
}

/// The install's URL token: read from the app data folder, or made and saved
/// when there is none yet.
fn install_token(dir: Option<&Path>) -> String {
    let path = dir.map(|d| d.join(TOKEN_FILE));
    if let Some(p) = &path {
        if let Ok(saved) = std::fs::read_to_string(p) {
            let saved = saved.trim();
            if saved.len() == 32 && saved.bytes().all(|c| c.is_ascii_hexdigit()) {
                return saved.to_string();
            }
        }
    }
    let token = uuid::Uuid::new_v4().simple().to_string();
    if let Some(p) = &path {
        if let Err(e) = write_synced(p, &token) {
            log::warn!("[CacheFiles] could not save the URL token ({e}); cached image URLs change next launch");
        }
    }
    token
}

/// Ports tried after the saved one: a fixed sequence for a token, so a launch
/// that finds the saved port taken still lands on a port it can land on again.
fn fallback_ports(token: &str) -> [u16; FALLBACK_PORTS] {
    // FNV-1a over the token, then a linear congruential step per port.
    let mut state = token.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    let mut ports = [0_u16; FALLBACK_PORTS];
    for port in &mut ports {
        state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        *port = 20000 + ((state >> 33) % 20000) as u16;
    }
    ports
}

/// A loopback listener on the port saved last time, else on one of the ports
/// derived from the token, else on a free port the OS picks. The bound port is
/// saved for the next launch.
async fn bind_listener(dir: Option<&Path>, token: &str) -> std::io::Result<tokio::net::TcpListener> {
    let path = dir.map(|d| d.join(PORT_FILE));
    let saved = path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<u16>().ok())
        .filter(|p| *p != 0);
    let mut bound = None;
    for port in saved.into_iter().chain(fallback_ports(token)) {
        match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => {
                bound = Some(l);
                break;
            }
            Err(e) => log::info!("[CacheFiles] port {port} is taken ({e}); trying another"),
        }
    }
    let listener = match bound {
        Some(l) => l,
        None => tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?,
    };
    if let Some(p) = &path {
        let port = listener.local_addr()?.port();
        if saved != Some(port) {
            if let Err(e) = write_synced(p, &port.to_string()) {
                log::warn!("[CacheFiles] could not save the port ({e}); cached image URLs change next launch");
            }
        }
    }
    Ok(listener)
}

/// Writes `text` to `<path>.tmp`, flushes it to disk, then renames it onto
/// `path`, so a crash leaves the old file or the new one, never a partial one.
/// A failed write deletes the temp file.
fn write_synced(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut tmp_name = path.as_os_str().to_owned();
    tmp_name.push(".tmp");
    let tmp = PathBuf::from(tmp_name);
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// The server's routes: `/cache/{token}/ping` and
/// `/cache/{token}/{folder}/{file}`, reading files under `root`. Every request
/// that matches no route is answered with `status`, so each reply carries the
/// same cache and CORS headers.
fn routes(token: String, root: PathBuf) -> impl Filter<Extract = (impl Reply,), Error = std::convert::Infallible> + Clone {
    let file_token = token.clone();
    let ping_token = token;
    let files = warp::get()
        .and(warp::path("cache"))
        .and(warp::path::param::<String>())
        .and(warp::path::param::<String>())
        .and(warp::path::param::<String>())
        .and(warp::path::end())
        .and(warp::header::optional::<String>("if-none-match"))
        .and_then(move |tok: String, dir: String, file: String, if_none_match: Option<String>| {
            let allowed = tok == file_token;
            let root = root.clone();
            async move {
                Ok::<_, Rejection>(if allowed { serve(root, dir, file, if_none_match).await } else { status(StatusCode::NOT_FOUND) })
            }
        });
    let ping = warp::get()
        .and(warp::path("cache"))
        .and(warp::path::param::<String>())
        .and(warp::path("ping"))
        .and(warp::path::end())
        .map(move |tok: String| status(if tok == ping_token { StatusCode::NO_CONTENT } else { StatusCode::NOT_FOUND }));
    ping.or(files).recover(rejection_status)
}

/// The reply for a request no route took: 405 for a method other than GET, 404
/// for anything else.
async fn rejection_status(rejection: Rejection) -> Result<Response<bytes::Bytes>, std::convert::Infallible> {
    let not_get = rejection.find::<warp::reject::MethodNotAllowed>().is_some();
    Ok(status(if not_get { StatusCode::METHOD_NOT_ALLOWED } else { StatusCode::NOT_FOUND }))
}

/// A reply with no body. Never cached, so a transient miss is not remembered.
fn status(code: StatusCode) -> Response<bytes::Bytes> {
    Response::builder()
        .status(code)
        .header("Cache-Control", "no-store")
        .header("Access-Control-Allow-Origin", "*")
        .body(bytes::Bytes::new())
        .unwrap_or_else(|_| Response::new(bytes::Bytes::new()))
}

/// What a file request found.
enum Found {
    /// The caller's validator still matches the file.
    NotModified(String),
    /// The file's bytes, with a validator when the folder is revalidated.
    File { bytes: Vec<u8>, etag: Option<String>, ext: String },
}

/// Folders whose files can change under the same name, so the WebView asks
/// again before reusing its copy. The other folders are content addressed: new
/// art is cached under a new key.
fn revalidates(dir: &str) -> bool {
    matches!(dir, "badges" | "third-party-badges" | "cosmetics")
}

/// A validator from the file's length and modification time.
fn etag_of(meta: &std::fs::Metadata) -> String {
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    format!("\"{:x}-{:x}\"", meta.len(), mtime_ns)
}

/// Whether an `If-None-Match` value names `etag` (a list of validators, weak
/// ones included, or `*`).
fn etag_matches(header: &str, etag: &str) -> bool {
    header.split(',').map(str::trim).any(|v| v == "*" || v.strip_prefix("W/").unwrap_or(v) == etag)
}

async fn serve(root: PathBuf, dir: String, file: String, if_none_match: Option<String>) -> Response<bytes::Bytes> {
    let Ok(permit) = READS.acquire().await else {
        return status(StatusCode::SERVICE_UNAVAILABLE);
    };
    let revalidate = revalidates(&dir);
    // The permit moves into the read, so a request the page cancels still
    // holds it until the file read ends.
    let read = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let path = resolve(&root, &dir, &file)?;
        let etag = if revalidate { Some(etag_of(&std::fs::metadata(&path).ok()?)) } else { None };
        if let (Some(tag), Some(header)) = (&etag, &if_none_match) {
            if etag_matches(header, tag) {
                return Some(Found::NotModified(tag.clone()));
            }
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        let bytes = std::fs::read(path).ok()?;
        Some(Found::File { bytes, etag, ext })
    })
    .await;
    match read {
        Ok(Some(Found::NotModified(etag))) => Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header("Cache-Control", "no-cache")
            .header("ETag", etag)
            .header("Access-Control-Allow-Origin", "*")
            .body(bytes::Bytes::new())
            .unwrap_or_else(|_| status(StatusCode::INTERNAL_SERVER_ERROR)),
        Ok(Some(Found::File { bytes, etag, ext })) => {
            let builder = Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", sniff(&bytes, &ext))
                .header("Access-Control-Allow-Origin", "*");
            let builder = match etag {
                // The file can be replaced under the same name, so the WebView
                // revalidates instead of keeping its copy.
                Some(tag) => builder.header("Cache-Control", "no-cache").header("ETag", tag),
                // The file behind a URL never changes (new art is cached under
                // a new key), so the WebView can keep its copy for good.
                None => builder.header("Cache-Control", "private, max-age=31536000, immutable"),
            };
            builder.body(bytes::Bytes::from(bytes)).unwrap_or_else(|_| status(StatusCode::INTERNAL_SERVER_ERROR))
        }
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
    let canon_dir = std::fs::canonicalize(root.join(dir)).ok()?;
    if !canon_dir.starts_with(&canon_root) {
        return None;
    }
    let canon = std::fs::canonicalize(canon_dir.join(file.as_ref())).ok()?;
    canon.starts_with(&canon_dir).then_some(canon)
}

/// The image type from its first bytes (many cached badges are `.bin` files
/// that hold PNG; some cosmetics `.bin` files hold WebP). Bytes that give no
/// known type in a file named `.svg` are still served as SVG, because the cache
/// only saves that extension for SVG content. An HTML page is served as opaque
/// bytes whatever the extension says.
fn sniff(b: &[u8], ext: &str) -> &'static str {
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
    } else if crate::services::universal_cache_service::looks_like_html(b) {
        // A page is never an image, whatever the file is named.
        "application/octet-stream"
    } else if crate::services::universal_cache_service::looks_like_svg(b) || ext.eq_ignore_ascii_case("svg") {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::universal_cache_service::looks_like_svg as is_svg;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("sn-cachefiles-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("emotes")).unwrap();
        std::fs::write(root.join("emotes").join("a@1x.webp"), b"RIFF0000WEBPVP8 ").unwrap();
        std::fs::write(root.join("manifest.json"), b"{}").unwrap();
        std::fs::create_dir_all(root.join("other")).unwrap();
        std::fs::write(root.join("other").join("secret.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        root
    }

    #[test]
    fn only_plain_files_in_the_image_folders_resolve() {
        let root = temp_root("scope");
        assert!(resolve(&root, "emotes", "a%401x.webp").is_some(), "encoded @ decodes");
        assert!(resolve(&root, "emotes", "a@1x.webp").is_some());
        assert!(resolve(&root, "emotes", "missing.webp").is_none());
        assert!(resolve(&root, "manifest.json", "x").is_none(), "not an image folder");
        assert!(resolve(&root, "other", "secret.png").is_none(), "a real folder beside the image folders is not served");
        assert!(resolve(&root, "emotes", "..%5Cmanifest.json").is_none());
        assert!(resolve(&root, "emotes", "..%2Fmanifest.json").is_none());
        assert!(resolve(&root, "emotes", "C%3A%5Cx").is_none());
        assert!(resolve(&root, "emotes", "").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn types_come_from_the_bytes() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest", "bin"), "image/png");
        assert_eq!(sniff(b"GIF89a..", "bin"), "image/gif");
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0], "bin"), "image/jpeg");
        assert_eq!(sniff(b"RIFF0000WEBPVP8 ", "bin"), "image/webp");
        assert_eq!(sniff(b"\0\0\0\x1cftypavif0000", "bin"), "image/avif");
        assert_eq!(sniff(b"  <svg xmlns=''/>", "bin"), "image/svg+xml");
        assert_eq!(sniff(b"<?xml version='1.0'?><svg/>", "bin"), "image/svg+xml");
        assert_eq!(sniff(b"hello", "bin"), "application/octet-stream");
    }

    #[test]
    fn an_svg_file_is_served_as_svg_even_when_its_bytes_fail_the_check() {
        assert_eq!(sniff(b"not markup", "svg"), "image/svg+xml");
        assert_eq!(sniff(b"not markup", "SVG"), "image/svg+xml");
        assert_eq!(sniff(b"not markup", "png"), "application/octet-stream");
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest", "svg"), "image/png", "known bytes win over the extension");
    }

    #[test]
    fn an_html_page_is_never_served_as_an_image() {
        let page = b"<!DOCTYPE html><html><body>Sign in to the network</body></html>";
        assert_eq!(sniff(page, "svg"), "application/octet-stream");
        assert_eq!(sniff(page, "png"), "application/octet-stream");
        assert_eq!(sniff(page, "bin"), "application/octet-stream");
        assert_eq!(sniff(b"<svg/>", "svg"), "image/svg+xml");
    }

    #[test]
    fn svg_files_are_recognized_in_their_real_world_shapes() {
        assert!(is_svg(b"<svg xmlns='http://www.w3.org/2000/svg'/>"));
        assert!(is_svg(b"\xEF\xBB\xBF<svg/>"), "BOM first");
        assert!(is_svg(b"\xEF\xBB\xBF \r\n\t<svg/>"), "BOM, then whitespace");
        assert!(is_svg(b"<?xml version='1.0'?>\n<svg/>"), "XML declaration");
        let mut late = b"<?xml version='1.0'?>".to_vec();
        late.extend(std::iter::repeat(b' ').take(900));
        late.extend_from_slice(b"<svg/>");
        assert!(is_svg(&late), "<svg within the first 1024 bytes");
        let mut too_late = b"<?xml version='1.0'?>".to_vec();
        too_late.extend(std::iter::repeat(b' ').take(1100));
        too_late.extend_from_slice(b"<svg/>");
        assert!(!is_svg(&too_late), "<svg past the first 1024 bytes");
        assert!(is_svg(b"<!-- made by a tool -->\n<svg/>"), "leading comment");
        assert!(is_svg(b"<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"\"><svg/>"), "doctype");
        assert!(!is_svg(b"\x89PNG\r\n\x1a\nrest"), "a PNG is not an SVG");
        assert!(!is_svg(b"<html><body></body></html>"));
        assert!(!is_svg(b""));
        assert_eq!(sniff(b"\xEF\xBB\xBF<svg/>", "bin"), "image/svg+xml");
    }

    #[test]
    fn the_folders_match_the_cache_types() {
        let src = include_str!("universal_cache_service.rs");
        for dir in CACHE_DIRS {
            assert!(src.contains(&format!("=> \"{dir}\"")), "{dir} is a CacheType folder");
        }
        // `CacheType::dir_name` is private, so its arms are counted in the source.
        let arms = src.lines().filter(|l| l.trim_start().starts_with("CacheType::") && l.contains("=> \"")).count();
        assert_eq!(arms, CACHE_DIRS.len(), "every CacheType folder is in CACHE_DIRS");
    }

    /// Links the folder `link` to `target`. Windows uses a directory junction,
    /// which needs no privilege; other platforms use a symlink.
    #[cfg(windows)]
    fn link_dir(target: &Path, link: &Path) -> std::io::Result<()> {
        let status = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(link).arg(target).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::new(std::io::ErrorKind::Other, format!("mklink /J failed with {status}")))
        }
    }

    #[cfg(not(windows))]
    fn link_dir(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    /// Makes `emotes/evil.webp` a way out of the cache. Windows junctions are
    /// folder-only, so there the `emotes` folder becomes a junction to a folder
    /// outside the cache that holds `evil.webp`; elsewhere `evil.webp` is a
    /// symlink to the manifest. Returns the outside folder on Windows.
    fn link_evil_image_out(root: &Path, name: &str) -> Option<PathBuf> {
        #[cfg(windows)]
        {
            let outside = std::env::temp_dir().join(format!("sn-cachefiles-evil-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&outside);
            std::fs::create_dir_all(&outside).unwrap();
            std::fs::write(outside.join("evil.webp"), b"RIFF0000WEBPVP8 ").unwrap();
            std::fs::remove_dir_all(root.join("emotes")).unwrap();
            link_dir(&outside, &root.join("emotes")).expect(NO_SYMLINK);
            Some(outside)
        }
        #[cfg(not(windows))]
        {
            let _ = name;
            std::os::unix::fs::symlink(root.join("manifest.json"), root.join("emotes").join("evil.webp")).expect(NO_SYMLINK);
            None
        }
    }

    const NO_SYMLINK: &str = "this platform could not create a link out of the cache folder here; the link checks cannot run";

    #[test]
    fn a_symlink_out_of_the_image_folder_does_not_resolve() {
        let root = temp_root("symlink");
        let evil_outside = link_evil_image_out(&root, "symlink");
        assert!(resolve(&root, "emotes", "evil.webp").is_none(), "a link out of the cache to an image");
        let outside = std::env::temp_dir().join(format!("sn-cachefiles-outside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("x.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        link_dir(&outside, &root.join("badges")).expect(NO_SYMLINK);
        assert!(resolve(&root, "badges", "x.png").is_none(), "an image folder that is a link out of the cache");
        let _ = std::fs::remove_dir_all(&outside);
        let _ = std::fs::remove_dir_all(&root);
        if let Some(evil) = evil_outside {
            let _ = std::fs::remove_dir_all(&evil);
        }
    }

    /// A junction named like an image, inside an image folder, leads out of the
    /// cache; the per-file containment check refuses it.
    #[cfg(windows)]
    #[test]
    fn a_junction_named_like_an_image_does_not_resolve() {
        let root = temp_root("junction-file");
        let outside = std::env::temp_dir().join(format!("sn-cachefiles-junction-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("x.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        link_dir(&outside, &root.join("emotes").join("evil.webp")).expect(NO_SYMLINK);
        assert!(resolve(&root, "emotes", "evil.webp").is_none(), "a junction inside an image folder that leads out of the cache");
        let _ = std::fs::remove_dir_all(&outside);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Serves `routes` on a loopback port the OS picks. Returns the base URL and
    /// the token it accepts.
    async fn spawn_server(root: PathBuf) -> (String, String) {
        let token = "0123456789abcdef0123456789abcdef".to_string();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(warp::serve(routes(token.clone(), root)).incoming(listener).run());
        (format!("http://127.0.0.1:{port}"), token)
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().unwrap()
    }

    async fn status_of(client: &reqwest::Client, url: &str) -> u16 {
        client.get(url).send().await.unwrap().status().as_u16()
    }

    #[tokio::test]
    async fn a_wrong_or_missing_token_is_rejected() {
        let root = temp_root("token");
        let (base, token) = spawn_server(root.clone()).await;
        let c = client();
        assert_eq!(status_of(&c, &format!("{base}/cache/{token}/emotes/a@1x.webp")).await, 200);
        assert_eq!(status_of(&c, &format!("{base}/cache/wrong/emotes/a@1x.webp")).await, 404);
        assert_eq!(status_of(&c, &format!("{base}/cache/{token}x/emotes/a@1x.webp")).await, 404);
        assert_eq!(status_of(&c, &format!("{base}/cache/emotes/a@1x.webp")).await, 404, "no token segment");
        assert_eq!(status_of(&c, &format!("{base}/cache/wrong/ping")).await, 404);
        assert_eq!(status_of(&c, &format!("{base}/emotes/a@1x.webp")).await, 404, "no cache prefix");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn ping_answers_no_content() {
        let root = temp_root("ping");
        let (base, token) = spawn_server(root.clone()).await;
        assert_eq!(status_of(&client(), &format!("{base}/cache/{token}/ping")).await, 204);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn an_unknown_folder_or_missing_file_is_not_found() {
        let root = temp_root("missing");
        let (base, token) = spawn_server(root.clone()).await;
        let c = client();
        assert_eq!(status_of(&c, &format!("{base}/cache/{token}/nope/a@1x.webp")).await, 404);
        assert_eq!(status_of(&c, &format!("{base}/cache/{token}/emotes/missing.webp")).await, 404);
        assert_eq!(status_of(&c, &format!("{base}/cache/{token}/manifest.json/x")).await, 404, "the manifest is no folder");
        assert_eq!(status_of(&c, &format!("{base}/cache/{token}/emotes/manifest.json")).await, 404);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn every_error_reply_carries_the_loopback_headers() {
        let root = temp_root("error-headers");
        let (base, token) = spawn_server(root.clone()).await;
        let c = client();
        let urls = [
            format!("{base}/cache/{token}/emotes"),
            format!("{base}/cache/{token}/emotes/a/b"),
            format!("{base}/cache/{token}/nope/a@1x.webp"),
            format!("{base}/cache/wrong/emotes/a@1x.webp"),
            format!("{base}/nothing"),
        ];
        for url in urls {
            let res = c.get(&url).send().await.unwrap();
            assert_eq!(res.status().as_u16(), 404, "{url}");
            let header = |name: &str| res.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
            assert_eq!(header("cache-control").as_deref(), Some("no-store"), "{url}");
            assert_eq!(header("access-control-allow-origin").as_deref(), Some("*"), "{url}");
        }
        let post = c.post(format!("{base}/cache/{token}/emotes/a@1x.webp")).send().await.unwrap();
        assert_eq!(post.status().as_u16(), 405);
        assert_eq!(post.headers().get("cache-control").and_then(|v| v.to_str().ok()), Some("no-store"));
        assert_eq!(post.headers().get("access-control-allow-origin").and_then(|v| v.to_str().ok()), Some("*"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn only_get_is_answered() {
        let root = temp_root("method");
        let (base, token) = spawn_server(root.clone()).await;
        let c = client();
        let image = format!("{base}/cache/{token}/emotes/a@1x.webp");
        assert_eq!(c.post(&image).send().await.unwrap().status().as_u16(), 405);
        assert_eq!(c.put(&image).send().await.unwrap().status().as_u16(), 405);
        assert_eq!(c.delete(&image).send().await.unwrap().status().as_u16(), 405);
        assert_eq!(c.post(format!("{base}/cache/{token}/ping")).send().await.unwrap().status().as_u16(), 405);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_valid_image_carries_its_type_cache_rule_and_cors_header() {
        let root = temp_root("image");
        let (base, token) = spawn_server(root.clone()).await;
        let res = client().get(format!("{base}/cache/{token}/emotes/a%401x.webp")).send().await.unwrap();
        assert_eq!(res.status().as_u16(), 200);
        let header = |name: &str| res.headers().get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
        assert_eq!(header("cache-control").as_deref(), Some("private, max-age=31536000, immutable"));
        assert_eq!(header("content-type").as_deref(), Some("image/webp"));
        assert_eq!(header("access-control-allow-origin").as_deref(), Some("*"));
        assert_eq!(res.bytes().await.unwrap().as_ref(), b"RIFF0000WEBPVP8 ");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn content_addressed_art_is_immutable_and_has_no_etag() {
        let root = temp_root("immutable");
        let (base, token) = spawn_server(root.clone()).await;
        let res = client().get(format!("{base}/cache/{token}/emotes/a@1x.webp")).send().await.unwrap();
        assert_eq!(res.status().as_u16(), 200);
        assert_eq!(res.headers().get("cache-control").and_then(|v| v.to_str().ok()), Some("private, max-age=31536000, immutable"));
        assert!(res.headers().get("etag").is_none(), "an immutable file has no validator");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn changeable_art_revalidates_with_an_etag() {
        let root = temp_root("revalidate");
        let badges = root.join("badges");
        std::fs::create_dir_all(&badges).unwrap();
        std::fs::write(badges.join("b.png"), b"\x89PNG\r\n\x1a\nold").unwrap();
        let (base, token) = spawn_server(root.clone()).await;
        let c = client();
        let url = format!("{base}/cache/{token}/badges/b.png");

        let first = c.get(&url).send().await.unwrap();
        assert_eq!(first.status().as_u16(), 200);
        assert_eq!(first.headers().get("cache-control").and_then(|v| v.to_str().ok()), Some("no-cache"));
        let etag = first.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_owned).expect("a validator");
        assert!(etag.starts_with('"') && etag.ends_with('"'), "{etag}");
        assert_eq!(first.bytes().await.unwrap().as_ref(), b"\x89PNG\r\n\x1a\nold");

        let same = c.get(&url).header("if-none-match", &etag).send().await.unwrap();
        assert_eq!(same.status().as_u16(), 304);
        assert_eq!(same.headers().get("access-control-allow-origin").and_then(|v| v.to_str().ok()), Some("*"));
        assert_eq!(same.headers().get("etag").and_then(|v| v.to_str().ok()), Some(etag.as_str()));
        assert!(same.bytes().await.unwrap().is_empty());

        std::fs::write(badges.join("b.png"), b"\x89PNG\r\n\x1a\nreplaced with more bytes").unwrap();
        let changed = c.get(&url).header("if-none-match", &etag).send().await.unwrap();
        assert_eq!(changed.status().as_u16(), 200);
        assert_ne!(changed.headers().get("etag").and_then(|v| v.to_str().ok()), Some(etag.as_str()));
        assert_eq!(changed.bytes().await.unwrap().as_ref(), b"\x89PNG\r\n\x1a\nreplaced with more bytes");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn only_an_svg_named_file_with_unknown_bytes_is_served_as_svg() {
        let root = temp_root("extension-fallback");
        let badges = root.join("badges");
        std::fs::create_dir_all(&badges).unwrap();
        std::fs::write(badges.join("x.svg"), b"not markup").unwrap();
        std::fs::write(badges.join("y.png"), b"not markup").unwrap();
        let (base, token) = spawn_server(root.clone()).await;
        let c = client();
        let content_type = |res: &reqwest::Response| res.headers().get("content-type").and_then(|v| v.to_str().ok()).map(str::to_owned);
        let svg = c.get(format!("{base}/cache/{token}/badges/x.svg")).send().await.unwrap();
        assert_eq!(svg.status().as_u16(), 200);
        assert_eq!(content_type(&svg).as_deref(), Some("image/svg+xml"));
        let png = c.get(format!("{base}/cache/{token}/badges/y.png")).send().await.unwrap();
        assert_eq!(png.status().as_u16(), 200);
        assert_eq!(content_type(&png).as_deref(), Some("application/octet-stream"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_synced_write_leaves_no_temp_file() {
        let root = temp_root("write-synced");
        let target = root.join("cache_server_port");
        std::fs::create_dir_all(&target).unwrap();
        assert!(write_synced(&target, "1234").is_err(), "a folder cannot be replaced by a file");
        assert!(!root.join("cache_server_port.tmp").exists(), "the temp file is deleted");
        let file = root.join("saved_port");
        write_synced(&file, "1234").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "1234");
        assert!(!root.join("saved_port.tmp").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_folder_outside_the_allowlist_is_not_served() {
        let root = temp_root("allowlist-wire");
        let (base, token) = spawn_server(root.clone()).await;
        assert_eq!(status_of(&client(), &format!("{base}/cache/{token}/other/secret.png")).await, 404);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn fallback_ports_are_stable_for_a_token() {
        let token = "0123456789abcdef0123456789abcdef";
        let ports = fallback_ports(token);
        assert_eq!(ports, fallback_ports(token), "the same token gives the same ports");
        assert_eq!(ports, [35854, 29290, 33345, 33257, 34893, 39078, 20522, 31556], "the ports are fixed for a token");
        assert!(ports.iter().all(|p| (20000..40000).contains(p)), "{ports:?}");
        assert_ne!(ports, fallback_ports("fedcba9876543210fedcba9876543210"), "another token gives other ports");
    }

    #[tokio::test]
    async fn encoded_traversal_is_rejected_over_the_wire() {
        let root = temp_root("wire-traversal");
        let (base, token) = spawn_server(root.clone()).await;
        let c = client();
        for file in ["..%2Fmanifest.json", "..%5Cmanifest.json", "..%2F..%5Cmanifest.json", "%2E%2E%2Fmanifest.json", "C%3A%5Cx", "C%3Ax", "..%252Fmanifest.json"] {
            assert_eq!(status_of(&c, &format!("{base}/cache/{token}/emotes/{file}")).await, 404, "{file}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_symlink_out_of_the_folder_is_not_served() {
        let root = temp_root("wire-symlink");
        let evil_outside = link_evil_image_out(&root, "wire-symlink");
        let (base, token) = spawn_server(root.clone()).await;
        assert_eq!(status_of(&client(), &format!("{base}/cache/{token}/emotes/evil.webp")).await, 404);
        let _ = std::fs::remove_dir_all(&root);
        if let Some(evil) = evil_outside {
            let _ = std::fs::remove_dir_all(&evil);
        }
    }
}
