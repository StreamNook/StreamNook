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
        let listener = match bind_listener(state_dir.as_deref()).await {
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

/// A loopback listener on the port saved last time, or on a free port when that
/// one is taken or none was saved. The bound port is saved for the next launch.
async fn bind_listener(dir: Option<&Path>) -> std::io::Result<tokio::net::TcpListener> {
    let path = dir.map(|d| d.join(PORT_FILE));
    let saved = path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<u16>().ok())
        .filter(|p| *p != 0);
    let listener = match saved {
        Some(port) => match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(l) => l,
            Err(e) => {
                log::info!("[CacheFiles] port {port} is taken ({e}); picking another");
                tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?
            }
        },
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

/// Writes `text` to `path` and flushes it to disk before returning.
fn write_synced(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

/// The server's routes: `/cache/{token}/ping` and
/// `/cache/{token}/{folder}/{file}`, reading files under `root`.
fn routes(token: String, root: PathBuf) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    let file_token = token.clone();
    let ping_token = token;
    let files = warp::get()
        .and(warp::path("cache"))
        .and(warp::path::param::<String>())
        .and(warp::path::param::<String>())
        .and(warp::path::param::<String>())
        .and(warp::path::end())
        .and_then(move |tok: String, dir: String, file: String| {
            let allowed = tok == file_token;
            let root = root.clone();
            async move { Ok::<_, Rejection>(if allowed { serve(root, dir, file).await } else { status(StatusCode::NOT_FOUND) }) }
        });
    let ping = warp::get()
        .and(warp::path("cache"))
        .and(warp::path::param::<String>())
        .and(warp::path("ping"))
        .and(warp::path::end())
        .map(move |tok: String| status(if tok == ping_token { StatusCode::NO_CONTENT } else { StatusCode::NOT_FOUND }));
    ping.or(files)
}

fn status(code: StatusCode) -> Response<bytes::Bytes> {
    Response::builder()
        .status(code)
        .header("Access-Control-Allow-Origin", "*")
        .body(bytes::Bytes::new())
        .unwrap_or_else(|_| Response::new(bytes::Bytes::new()))
}

async fn serve(root: PathBuf, dir: String, file: String) -> Response<bytes::Bytes> {
    let Ok(permit) = READS.acquire().await else {
        return status(StatusCode::SERVICE_UNAVAILABLE);
    };
    // The permit moves into the read, so a request the page cancels still
    // holds it until the file read ends.
    let read = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let path = resolve(&root, &dir, &file)?;
        std::fs::read(path).ok()
    })
    .await;
    match read {
        Ok(Some(bytes)) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", sniff(&bytes))
            // The file behind a URL never changes (new art is cached under a
            // new key), so the WebView can keep its copy for good.
            .header("Cache-Control", "private, max-age=31536000, immutable")
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
    let canon_dir = std::fs::canonicalize(root.join(dir)).ok()?;
    if !canon_dir.starts_with(&canon_root) {
        return None;
    }
    let canon = std::fs::canonicalize(canon_dir.join(file.as_ref())).ok()?;
    canon.starts_with(&canon_dir).then_some(canon)
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

/// SVG text: after an optional BOM and whitespace it opens with `<svg`, a
/// comment, a `<!DOCTYPE svg`, or an XML declaration followed by `<svg` within
/// the first 1024 bytes.
fn is_svg(b: &[u8]) -> bool {
    let b = b.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(b);
    let start = b.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(b.len());
    let head = &b[start..b.len().min(start + 1024)];
    head.starts_with(b"<svg")
        || head.starts_with(b"<!--")
        || head.starts_with(b"<!DOCTYPE svg")
        || (head.starts_with(b"<?xml") && head.windows(4).any(|w| w == b"<svg"))
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
        assert_eq!(sniff(b"\xEF\xBB\xBF<svg/>"), "image/svg+xml");
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

    #[cfg(windows)]
    fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }

    #[cfg(windows)]
    fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(target, link)
    }

    #[cfg(unix)]
    fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(unix)]
    fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    const NO_SYMLINK: &str = "this platform could not create a symlink here (on Windows turn on Developer Mode or run elevated); the symlink checks cannot run";

    #[test]
    fn a_symlink_out_of_the_image_folder_does_not_resolve() {
        let root = temp_root("symlink");
        symlink_file(&root.join("manifest.json"), &root.join("emotes").join("evil.webp")).expect(NO_SYMLINK);
        assert!(resolve(&root, "emotes", "evil.webp").is_none(), "a file link to the manifest");
        let outside = std::env::temp_dir().join(format!("sn-cachefiles-outside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("x.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        symlink_dir(&outside, &root.join("badges")).expect(NO_SYMLINK);
        assert!(resolve(&root, "badges", "x.png").is_none(), "an image folder that is a link out of the cache");
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
        symlink_file(&root.join("manifest.json"), &root.join("emotes").join("evil.webp")).expect(NO_SYMLINK);
        let (base, token) = spawn_server(root.clone()).await;
        assert_eq!(status_of(&client(), &format!("{base}/cache/{token}/emotes/evil.webp")).await, 404);
        let _ = std::fs::remove_dir_all(&root);
    }
}
