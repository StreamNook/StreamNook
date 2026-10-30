//! Opening a song's link in a music app on this computer instead of the
//! browser. `apps()` lists the installed desktop apps that can play each
//! service, kept for a minute; `open()` turns a stored web link into the link
//! its app answers to and launches it. The app link is built only from ids
//! read out of an address whose host is the service's own (services.rs), and
//! the page names an app only by a `target` from this list, looked up in the
//! list again before anything launches. Every failure falls back to the web
//! link in the browser, as it opened before there were app links.
//!
//! Windows reads the registry (the apps registered for each scheme, and the
//! user's default for it) through `Registry`, so the walk is tested with a
//! fake. A packaged app (Cider, Apple Music) is launched by its
//! AppUserModelID, so the link reaches exactly that app even where two apps
//! claim its scheme. macOS looks for the app bundles; Linux asks xdg-mime for
//! each app's own scheme. Phones list nothing: their links open the way every
//! other link does.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use url::Url;

use super::links::apple_id_of;
use super::services::{is_search_page, service_of};

/// How long a listing of the installed apps is used before it is read again.
const FRESH_FOR: Duration = Duration::from_secs(60);

/// The schemes a music app is found by, in the order a default is looked for.
const SCHEMES: [&str; 6] = ["spotify", "cider", "musics", "music", "tidal", "deezer"];

/// The services in the order the list gives them.
const SERVICE_ORDER: [&str; 4] = ["Spotify", "Apple Music", "Tidal", "Deezer"];

/// The music apps a link can open in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppKind {
    Spotify,
    /// Cider, an Apple Music player.
    Cider,
    /// Apple's own Apple Music app (Music on macOS), or another app that
    /// claims Apple Music's schemes, by its own name.
    AppleMusic,
    Tidal,
    Deezer,
}

impl AppKind {
    /// The service it plays, as services.rs names it.
    fn service(self) -> &'static str {
        match self {
            AppKind::Spotify => "Spotify",
            AppKind::Cider | AppKind::AppleMusic => "Apple Music",
            AppKind::Tidal => "Tidal",
            AppKind::Deezer => "Deezer",
        }
    }

    /// The app's own name, when the system gives none that reads.
    fn name(self) -> &'static str {
        match self {
            AppKind::Spotify => "Spotify",
            AppKind::Cider => "Cider",
            AppKind::AppleMusic => "Apple Music",
            AppKind::Tidal => "Tidal",
            AppKind::Deezer => "Deezer",
        }
    }

    /// The scheme only this app answers to (Apple's app shares its schemes
    /// with Cider, so it has none of its own).
    fn own_scheme(self) -> Option<&'static str> {
        match self {
            AppKind::Spotify => Some("spotify"),
            AppKind::Cider => Some("cider"),
            AppKind::AppleMusic => None,
            AppKind::Tidal => Some("tidal"),
            AppKind::Deezer => Some("deezer"),
        }
    }
}

/// How an app is launched with its link.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Launch {
    /// A packaged Windows app, by its AppUserModelID: the link reaches
    /// exactly this app.
    Packaged(String),
    /// The system's opener: the link's scheme is this app's alone.
    Scheme(&'static str),
    /// A macOS app bundle, by its path.
    Bundle(String),
}

impl Launch {
    /// The id the page names this app by.
    fn target(&self) -> String {
        match self {
            Launch::Packaged(aumid) => aumid.clone(),
            Launch::Scheme(scheme) => (*scheme).to_string(),
            Launch::Bundle(path) => path.clone(),
        }
    }
}

/// An installed desktop app that can open one service's links.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SongApp {
    /// The service it plays, as the page names links ("Apple Music").
    pub service: &'static str,
    /// The app's own name ("Cider", "Apple Music", "Spotify").
    pub app: String,
    /// What the page hands back to open a link in this app: an
    /// AppUserModelID, a scheme or an app bundle's path. Opaque to the page.
    pub target: String,
    /// The app a plain open of this service's links uses.
    pub default: bool,
    #[serde(skip)]
    kind: AppKind,
    #[serde(skip)]
    launch: Launch,
}

/// One app found for a service, before the list is settled.
#[derive(Debug, Clone)]
struct Found {
    kind: AppKind,
    app: String,
    launch: Launch,
    /// The schemes it was found on, each with the ProgId it holds it by
    /// (Windows), to match against the user's default handler.
    claims: Vec<(String, String)>,
}

impl Found {
    fn new(kind: AppKind, app: String, launch: Launch) -> Self {
        Found {
            kind,
            app,
            launch,
            claims: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------- the list

static CACHE: Mutex<Option<(Instant, Vec<SongApp>)>> = Mutex::new(None);

/// The installed desktop apps that open each service's links, each service's
/// default first. Read again when the last reading is a minute old.
pub fn apps() -> Vec<SongApp> {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, list)) = cache.as_ref() {
        if at.elapsed() < FRESH_FOR {
            return list.clone();
        }
    }
    let list = installed();
    *cache = Some((Instant::now(), list.clone()));
    list
}

#[cfg(windows)]
fn installed() -> Vec<SongApp> {
    windows_apps(&win::Real)
}

#[cfg(target_os = "macos")]
fn installed() -> Vec<SongApp> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    settle(mac_found(home.as_deref(), &|p| p.is_dir()), &|_| None)
}

#[cfg(target_os = "linux")]
fn installed() -> Vec<SongApp> {
    settle(linux_found(&xdg_handler), &|_| None)
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn installed() -> Vec<SongApp> {
    Vec::new()
}

/// One entry per app and service (a packaged copy wins over one found by its
/// scheme alone), each service's default first. `choice` gives the user's
/// default handler (its ProgId) for a scheme where the system keeps one.
fn settle(found: Vec<Found>, choice: &dyn Fn(&str) -> Option<String>) -> Vec<SongApp> {
    let mut merged: Vec<Found> = Vec::new();
    for f in found {
        let same = merged
            .iter_mut()
            .find(|m| m.kind.service() == f.kind.service() && m.app.eq_ignore_ascii_case(&f.app));
        match same {
            Some(m) => {
                if matches!(m.launch, Launch::Scheme(_)) && matches!(f.launch, Launch::Packaged(_))
                {
                    m.launch = f.launch;
                }
                for claim in f.claims {
                    if !m.claims.contains(&claim) {
                        m.claims.push(claim);
                    }
                }
            }
            None => merged.push(f),
        }
    }

    let mut out = Vec::new();
    for service in SERVICE_ORDER {
        let mut group: Vec<Found> = merged
            .iter()
            .filter(|f| f.kind.service() == service)
            .cloned()
            .collect();
        if group.is_empty() {
            continue;
        }
        let chosen = group.remove(default_of(&group, choice));
        group.insert(0, chosen);
        for (i, f) in group.into_iter().enumerate() {
            out.push(SongApp {
                service,
                target: f.launch.target(),
                app: f.app,
                default: i == 0,
                kind: f.kind,
                launch: f.launch,
            });
        }
    }
    out
}

/// Which of one service's apps a plain open uses: the one holding the user's
/// default for a scheme they both claim, else Cider before Apple's app, else
/// the first found.
fn default_of(group: &[Found], choice: &dyn Fn(&str) -> Option<String>) -> usize {
    if group.len() < 2 {
        return 0;
    }
    let claimed_by = |scheme: &str| {
        group
            .iter()
            .filter(|f| f.claims.iter().any(|(s, _)| s == scheme))
            .count()
    };
    for scheme in SCHEMES.into_iter().filter(|s| claimed_by(*s) > 1) {
        let Some(prog_id) = choice(scheme) else {
            continue;
        };
        let holder = group.iter().position(|f| {
            f.claims
                .iter()
                .any(|(s, p)| s == scheme && p.eq_ignore_ascii_case(prog_id.trim()))
        });
        if let Some(i) = holder {
            return i;
        }
    }
    group
        .iter()
        .position(|f| f.kind == AppKind::Cider)
        .unwrap_or(0)
}

// ---------------------------------------------------------------- Windows

/// Where Windows keeps a registry key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Hive {
    LocalMachine,
    CurrentUser,
    ClassesRoot,
}

/// The registry reads the Windows walk makes.
trait Registry {
    /// The string values directly under `key`, as (name, data).
    fn values(&self, hive: Hive, key: &str) -> Vec<(String, String)>;
    /// One string value of `key`; `""` names its default value.
    fn string(&self, hive: Hive, key: &str, name: &str) -> Option<String>;
    /// Whether `key` holds a value called `name`, of any type.
    fn has_value(&self, hive: Hive, key: &str, name: &str) -> bool;
    /// An indirect string (`@{Package?ms-resource://...}`) as Windows shows it.
    fn indirect(&self, raw: &str) -> Option<String>;
}

/// The music apps Windows has, with the user's defaults deciding which one
/// each service opens in.
fn windows_apps(reg: &dyn Registry) -> Vec<SongApp> {
    settle(windows_found(reg), &|scheme| {
        reg.string(
            Hive::CurrentUser,
            &format!(
                r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\{scheme}\UserChoice"
            ),
            "ProgId",
        )
    })
}

/// The apps Windows has registered for the music schemes: every registered
/// application's URL associations, in both hives (a packaged app such as
/// Cider files its own there), then the classic way, a scheme's own key
/// marked as a URL protocol with a command to run. A classic key with no
/// command (left behind by an uninstalled app) opens nothing and is passed
/// over.
fn windows_found(reg: &dyn Registry) -> Vec<Found> {
    let mut found = Vec::new();
    for hive in [Hive::CurrentUser, Hive::LocalMachine] {
        for (_, capabilities) in reg.values(hive, r"SOFTWARE\RegisteredApplications") {
            let capabilities = capabilities.trim().trim_matches('\\');
            if capabilities.is_empty() {
                continue;
            }
            for (scheme, prog_id) in reg.values(hive, &format!(r"{capabilities}\URLAssociations")) {
                let lower = scheme.to_ascii_lowercase();
                let Some(scheme) = SCHEMES.into_iter().find(|s| *s == lower) else {
                    continue;
                };
                let prog_id = prog_id.trim().to_string();
                if prog_id.is_empty() || prog_id.contains('\\') {
                    continue;
                }
                let application = format!(r"{prog_id}\Application");
                let aumid = reg
                    .string(Hive::ClassesRoot, &application, "AppUserModelID")
                    .map(|s| s.trim().to_string())
                    .filter(|s| is_aumid(s));
                let name = reg
                    .string(Hive::ClassesRoot, &application, "ApplicationName")
                    .and_then(|raw| readable(reg, &raw));
                if let Some(mut app) = claimant(scheme, name.as_deref(), aumid) {
                    app.claims.push((scheme.to_string(), prog_id));
                    found.push(app);
                }
            }
        }
    }
    for kind in [
        AppKind::Spotify,
        AppKind::Cider,
        AppKind::Tidal,
        AppKind::Deezer,
    ] {
        let Some(scheme) = kind.own_scheme() else {
            continue;
        };
        let command = reg.string(
            Hive::ClassesRoot,
            &format!(r"{scheme}\shell\open\command"),
            "",
        );
        if reg.has_value(Hive::ClassesRoot, scheme, "URL Protocol")
            && command.is_some_and(|c| !c.trim().is_empty())
        {
            let mut app = Found::new(kind, kind.name().to_string(), Launch::Scheme(scheme));
            app.claims.push((scheme.to_string(), scheme.to_string()));
            found.push(app);
        }
    }
    found
}

/// The app a registered claim on `scheme` is. Spotify's, Tidal's and Deezer's
/// schemes and `cider` are their own app's; `musics` and `music` are Apple
/// Music's, claimed by Cider or Apple's app (or another player, by its own
/// name), which must be packaged to be launched as itself, since every one of
/// them answers to those schemes.
fn claimant(scheme: &'static str, name: Option<&str>, aumid: Option<String>) -> Option<Found> {
    let cider = scheme == "cider" || name.is_some_and(|n| n.eq_ignore_ascii_case("cider"));
    let kind = match scheme {
        "spotify" => AppKind::Spotify,
        "tidal" => AppKind::Tidal,
        "deezer" => AppKind::Deezer,
        "cider" | "musics" | "music" if cider => AppKind::Cider,
        "musics" | "music" => AppKind::AppleMusic,
        _ => return None,
    };
    let launch = match (aumid, matches!(scheme, "musics" | "music")) {
        (Some(aumid), _) => Launch::Packaged(aumid),
        (None, false) => Launch::Scheme(kind.own_scheme()?),
        (None, true) => return None,
    };
    let app = match (kind, &launch) {
        (AppKind::AppleMusic, Launch::Packaged(aumid))
            if aumid.starts_with("AppleInc.AppleMusic") =>
        {
            "Apple Music".to_string()
        }
        (AppKind::AppleMusic, _) => tidy(Some(name?), kind.name()),
        _ => tidy(name, kind.name()),
    };
    Some(Found::new(kind, app, launch))
}

/// The service's own app name for a name that carries it ("Spotify - Music
/// and Podcasts" is Spotify), else the name itself, else the own name.
fn tidy(name: Option<&str>, own: &str) -> String {
    match name {
        Some(n) if !n.to_ascii_lowercase().contains(&own.to_ascii_lowercase()) => n.to_string(),
        _ => own.to_string(),
    }
}

/// An app name fit to show: an indirect string resolved, plain text of a
/// sensible length.
fn readable(reg: &dyn Registry, raw: &str) -> Option<String> {
    let raw = raw.trim();
    let text = if raw.starts_with('@') {
        reg.indirect(raw)?
    } else {
        raw.to_string()
    };
    let text = text.trim();
    (!text.is_empty()
        && !text.starts_with('@')
        && text.chars().count() <= 64
        && !text.chars().any(char::is_control))
    .then(|| text.to_string())
}

/// An AppUserModelID: `Family!App`, with no spaces or control characters.
fn is_aumid(s: &str) -> bool {
    s.len() <= 256
        && s.contains('!')
        && !s.starts_with('!')
        && !s.chars().any(|c| c.is_whitespace() || c.is_control())
}

// ---------------------------------------------------------------- macOS and Linux

/// The app bundles macOS has: Apple's Music in the system's folder, the
/// others in /Applications or the user's own Applications.
fn mac_found(home: Option<&Path>, is_app: &dyn Fn(&Path) -> bool) -> Vec<Found> {
    const BUNDLES: [(AppKind, &str); 5] = [
        (AppKind::Spotify, "Spotify.app"),
        (AppKind::Cider, "Cider.app"),
        (AppKind::AppleMusic, "Music.app"),
        (AppKind::Tidal, "TIDAL.app"),
        (AppKind::Deezer, "Deezer.app"),
    ];
    let mut folders = vec![PathBuf::from("/Applications")];
    if let Some(home) = home {
        folders.push(home.join("Applications"));
    }
    BUNDLES
        .into_iter()
        .filter_map(|(kind, bundle)| {
            let places: Vec<PathBuf> = if kind == AppKind::AppleMusic {
                vec![PathBuf::from("/System/Applications").join(bundle)]
            } else {
                folders.iter().map(|f| f.join(bundle)).collect()
            };
            let place = places.into_iter().find(|p| is_app(p.as_path()))?;
            Some(Found::new(
                kind,
                kind.name().to_string(),
                Launch::Bundle(place.to_string_lossy().into_owned()),
            ))
        })
        .collect()
}

/// The apps Linux has a handler for, by each app's own scheme. `handler`
/// names the desktop entry that opens a scheme, if any.
fn linux_found(handler: &dyn Fn(&str) -> Option<String>) -> Vec<Found> {
    [
        AppKind::Spotify,
        AppKind::Cider,
        AppKind::Tidal,
        AppKind::Deezer,
    ]
    .into_iter()
    .filter_map(|kind| {
        let scheme = kind.own_scheme()?;
        handler(scheme)
            .filter(|entry| !entry.trim().is_empty())
            .map(|_| Found::new(kind, kind.name().to_string(), Launch::Scheme(scheme)))
    })
    .collect()
}

#[cfg(target_os = "linux")]
fn xdg_handler(scheme: &str) -> Option<String> {
    let out = std::process::Command::new("xdg-mime")
        .args(["query", "default", &format!("x-scheme-handler/{scheme}")])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

// ---------------------------------------------------------------- the app links

/// What a song's web link names, read only from an address on the
/// service's own host.
#[derive(Debug, PartialEq, Eq)]
enum Item {
    SpotifyTrack(String),
    SpotifySearch(String),
    AppleSong { country: String, id: String },
    TidalTrack(String),
    DeezerTrack(String),
}

fn item_of(url: &str) -> Option<Item> {
    let service = service_of(url)?;
    let parsed = Url::parse(url).ok()?;
    let parts: Vec<&str> = parsed.path_segments()?.filter(|p| !p.is_empty()).collect();
    match service.name {
        "Spotify" => {
            // A localized address starts with its language (`/intl-de/track/...`).
            let parts = match parts.split_first() {
                Some((first, rest)) if first.starts_with("intl-") => rest,
                _ => &parts[..],
            };
            match parts {
                ["track", id] if is_spotify_id(id) => Some(Item::SpotifyTrack(id.to_string())),
                ["search", words] => search_words(words).map(Item::SpotifySearch),
                _ => None,
            }
        }
        "Apple Music" => {
            if is_search_page(url) {
                return None;
            }
            let id = apple_id_of(url)?;
            let country = parts
                .first()
                .filter(|p| is_country(p))
                .map_or_else(|| "us".to_string(), |p| p.to_ascii_lowercase());
            Some(Item::AppleSong { country, id })
        }
        "Tidal" => {
            let parts = match parts.split_first() {
                Some((&"browse", rest)) => rest,
                _ => &parts[..],
            };
            match parts {
                ["track", id] | ["track", id, "u"] if is_digits(id) => {
                    Some(Item::TidalTrack(id.to_string()))
                }
                _ => None,
            }
        }
        "Deezer" => {
            let parts = match parts.split_first() {
                Some((first, rest)) if is_country(first) => rest,
                _ => &parts[..],
            };
            match parts {
                ["track", id] if is_digits(id) => Some(Item::DeezerTrack(id.to_string())),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A Spotify id: 22 of letters and digits.
fn is_spotify_id(s: &str) -> bool {
    s.len() == 22 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// A catalog id: 1 to 20 digits.
fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit())
}

/// A storefront's country (`us`, `gb`).
fn is_country(s: &str) -> bool {
    s.len() == 2 && s.bytes().all(|b| b.is_ascii_alphabetic())
}

/// The words of a search page's address, decoded: plain text of a sensible
/// length.
fn search_words(raw: &str) -> Option<String> {
    let words = urlencoding::decode(raw).ok()?;
    let words = words.trim();
    (!words.is_empty() && words.chars().count() <= 200 && !words.chars().any(char::is_control))
        .then(|| words.to_string())
}

/// The link `kind`'s app answers to for `item`, or None when that app does
/// not open it. Built only from the validated ids. A search's words are
/// encoded again, so nothing in them can reach past the link.
fn app_link(item: &Item, kind: AppKind, mac: bool) -> Option<String> {
    match (item, kind) {
        (Item::SpotifyTrack(id), AppKind::Spotify) => Some(format!("spotify:track:{id}")),
        (Item::SpotifySearch(words), AppKind::Spotify) => {
            Some(format!("spotify:search:{}", urlencoding::encode(words)))
        }
        (Item::AppleSong { id, .. }, AppKind::Cider) => Some(format!("cider://play/s/{id}")),
        (Item::AppleSong { country, id }, AppKind::AppleMusic) => {
            let scheme = if mac { "music" } else { "musics" };
            Some(format!("{scheme}://music.apple.com/{country}/song/{id}"))
        }
        (Item::TidalTrack(id), AppKind::Tidal) => Some(format!("tidal://track/{id}")),
        (Item::DeezerTrack(id), AppKind::Deezer) => {
            Some(format!("deezer://www.deezer.com/track/{id}"))
        }
        _ => None,
    }
}

/// What a press on a song link does.
#[derive(Debug, PartialEq, Eq)]
enum Plan {
    /// Open `link` in the app, launched as `launch`.
    App {
        link: String,
        launch: Launch,
        kind: AppKind,
    },
    /// Open the web link in the browser.
    Browser,
}

/// The plan for `url` opened in `app` (a target from `apps`), or in its
/// service's default app. The browser when the service has no app here, the
/// target is not one of its apps, or the app does not open this link (an
/// album, a search on a service whose app has no search link). An address
/// that is no song link is refused.
fn plan(url: &str, app: Option<&str>, apps: &[SongApp], mac: bool) -> Result<Plan, String> {
    let service = service_of(url).ok_or_else(|| "not a song link".to_string())?;
    let chosen = match app {
        Some(target) => apps
            .iter()
            .find(|a| a.service == service.name && a.target == target),
        None => apps.iter().find(|a| a.service == service.name && a.default),
    };
    let planned = chosen.zip(item_of(url)).and_then(|(a, item)| {
        app_link(&item, a.kind, mac).map(|link| Plan::App {
            link,
            launch: a.launch.clone(),
            kind: a.kind,
        })
    });
    Ok(planned.unwrap_or(Plan::Browser))
}

/// Opens the song at `url` in `app` (a `target` from `apps()`), or in its
/// service's default app when None; in the browser (through `system_open`)
/// when no app here opens it or the launch fails. True when an app took it.
/// An address that is no song link is refused.
pub fn open(
    url: &str,
    app: Option<&str>,
    system_open: &dyn Fn(&str) -> Result<(), String>,
) -> Result<bool, String> {
    if let Plan::App { link, launch, kind } = plan(url, app, &apps(), cfg!(target_os = "macos"))? {
        match launch_app(&link, &launch, kind, system_open) {
            Ok(()) => return Ok(true),
            Err(e) => log::warn!("[SongLinks] {} did not open in its app: {e}", kind.name()),
        }
    }
    system_open(url).map(|()| false)
}

/// Launches `link` in its app. A packaged app that will not take it is tried
/// once more through the system's opener when the link's scheme is that
/// app's alone.
fn launch_app(
    link: &str,
    launch: &Launch,
    kind: AppKind,
    system_open: &dyn Fn(&str) -> Result<(), String>,
) -> Result<(), String> {
    match launch {
        Launch::Scheme(_) => system_open(link),
        #[cfg(windows)]
        Launch::Packaged(aumid) => {
            win::activate(aumid, link).or_else(|e| match kind.own_scheme() {
                Some(_) => system_open(link),
                None => Err(e),
            })
        }
        #[cfg(target_os = "macos")]
        Launch::Bundle(path) => {
            let status = std::process::Command::new("open")
                .arg("-a")
                .arg(path)
                .arg(link)
                .status()
                .map_err(|e| e.to_string())?;
            if status.success() {
                Ok(())
            } else {
                Err(format!("open exited with {status}"))
            }
        }
        #[allow(unreachable_patterns)]
        _ => Err("that app is not launched this way here".to_string()),
    }
}

// ---------------------------------------------------------------- the Windows registry and launch

#[cfg(windows)]
mod win {
    use super::{Hive, Registry};
    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegEnumValueW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CLASSES_ROOT,
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_EXPAND_SZ, REG_SZ, RRF_RT_ANY,
        RRF_RT_REG_SZ,
    };

    /// The registry as it is.
    pub(super) struct Real;

    fn root(hive: Hive) -> HKEY {
        match hive {
            Hive::LocalMachine => HKEY_LOCAL_MACHINE,
            Hive::CurrentUser => HKEY_CURRENT_USER,
            Hive::ClassesRoot => HKEY_CLASSES_ROOT,
        }
    }

    /// An open key, closed when dropped.
    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the key was opened by RegOpenKeyExW and is closed once.
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }

    fn open(hive: Hive, path: &str) -> Option<Key> {
        let mut key = HKEY::default();
        // SAFETY: `key` is a valid out pointer; the path HSTRING outlives the call.
        let status =
            unsafe { RegOpenKeyExW(root(hive), &HSTRING::from(path), None, KEY_READ, &mut key) };
        (status == ERROR_SUCCESS).then(|| Key(key))
    }

    /// UTF-16 text from a registry buffer, up to its first NUL.
    fn text(units: &[u16]) -> String {
        let end = units.iter().position(|u| *u == 0).unwrap_or(units.len());
        String::from_utf16_lossy(&units[..end])
    }

    impl Registry for Real {
        fn values(&self, hive: Hive, key: &str) -> Vec<(String, String)> {
            let Some(key) = open(hive, key) else {
                return Vec::new();
            };
            let mut out = Vec::new();
            let mut name = vec![0u16; 16_384];
            let mut data = vec![0u16; 4_096];
            // A bound on a key's values; these keys hold a few dozen.
            for index in 0..4_096u32 {
                let mut name_len = name.len() as u32;
                let mut data_bytes = (data.len() * 2) as u32;
                let mut kind = 0u32;
                // SAFETY: each buffer and its length describe the same
                // allocation; the call writes no more than those lengths.
                let status = unsafe {
                    RegEnumValueW(
                        key.0,
                        index,
                        Some(PWSTR(name.as_mut_ptr())),
                        &mut name_len,
                        None,
                        Some(&mut kind),
                        Some(data.as_mut_ptr().cast()),
                        Some(&mut data_bytes),
                    )
                };
                if status == ERROR_NO_MORE_ITEMS {
                    break;
                }
                // A value too long for the buffer is no app's name or path.
                if status != ERROR_SUCCESS || (kind != REG_SZ.0 && kind != REG_EXPAND_SZ.0) {
                    continue;
                }
                let name_len = (name_len as usize).min(name.len());
                let data_len = (data_bytes as usize / 2).min(data.len());
                out.push((text(&name[..name_len]), text(&data[..data_len])));
            }
            out
        }

        fn string(&self, hive: Hive, key: &str, name: &str) -> Option<String> {
            let mut buf = vec![0u16; 2_048];
            let mut bytes = (buf.len() * 2) as u32;
            // SAFETY: the buffer and its byte length describe the same
            // allocation; RegGetValueW writes at most `bytes` bytes and
            // NUL-terminates a REG_SZ.
            let status = unsafe {
                RegGetValueW(
                    root(hive),
                    &HSTRING::from(key),
                    &HSTRING::from(name),
                    RRF_RT_REG_SZ,
                    None,
                    Some(buf.as_mut_ptr().cast()),
                    Some(&mut bytes),
                )
            };
            if status != ERROR_SUCCESS {
                return None;
            }
            let len = (bytes as usize / 2).min(buf.len());
            Some(text(&buf[..len]))
        }

        fn has_value(&self, hive: Hive, key: &str, name: &str) -> bool {
            // SAFETY: no data is read; only whether the value exists.
            let status = unsafe {
                RegGetValueW(
                    root(hive),
                    &HSTRING::from(key),
                    &HSTRING::from(name),
                    RRF_RT_ANY,
                    None,
                    None,
                    None,
                )
            };
            status == ERROR_SUCCESS
        }

        fn indirect(&self, raw: &str) -> Option<String> {
            use windows::Win32::UI::Shell::SHLoadIndirectString;
            let mut buf = vec![0u16; 512];
            // SAFETY: the output slice carries its own length.
            unsafe { SHLoadIndirectString(&HSTRING::from(raw), &mut buf, None) }.ok()?;
            Some(text(&buf))
        }
    }

    /// Hands `link` to the packaged app `aumid` alone, the way the shell
    /// opens a link in a chosen app.
    pub(super) fn activate(aumid: &str, link: &str) -> Result<(), String> {
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_LOCAL_SERVER,
            COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
        };
        use windows::Win32::UI::Shell::{
            ApplicationActivationManager, IApplicationActivationManager, IShellItem,
            IShellItemArray, SHCreateItemFromParsingName, SHCreateShellItemArrayFromShellItem,
        };

        /// COM for this thread, released when the launch is done (only when
        /// this call started it; a thread already in another mode keeps it).
        struct Com(bool);
        impl Drop for Com {
            fn drop(&mut self) {
                if self.0 {
                    // SAFETY: pairs the successful CoInitializeEx below.
                    unsafe { CoUninitialize() };
                }
            }
        }
        // SAFETY: plain COM start-up on a blocking worker thread.
        let _com =
            Com(
                unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }
                    .is_ok(),
            );

        // SAFETY: every interface is used while COM is up (`_com` drops last)
        // and the HSTRINGs outlive the calls that read them.
        unsafe {
            let manager: IApplicationActivationManager =
                CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER)
                    .map_err(|e| format!("activation manager: {e}"))?;
            let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(link), None)
                .map_err(|e| format!("link item: {e}"))?;
            let items: IShellItemArray = SHCreateShellItemArrayFromShellItem(&item)
                .map_err(|e| format!("link items: {e}"))?;
            manager
                .ActivateForProtocol(&HSTRING::from(aumid), &items)
                .map_err(|e| format!("activate: {e}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // ------------------------------------------------ the app links

    fn link(url: &str, kind: AppKind, mac: bool) -> Option<String> {
        app_link(&item_of(url)?, kind, mac)
    }

    #[test]
    fn spotify_tracks_and_searches_become_spotify_links() {
        let id = "4cOdK2wGLETKBW3PvgPWqT";
        assert_eq!(
            link(
                &format!("https://open.spotify.com/track/{id}"),
                AppKind::Spotify,
                false
            ),
            Some(format!("spotify:track:{id}"))
        );
        assert_eq!(
            link(
                &format!("https://open.spotify.com/intl-de/track/{id}?si=abc"),
                AppKind::Spotify,
                false
            ),
            Some(format!("spotify:track:{id}"))
        );
        // Decoded, then encoded again: nothing in the words reaches past the link.
        assert_eq!(
            link(
                "https://open.spotify.com/search/Bad%20Bunny%20NUEVAYoL",
                AppKind::Spotify,
                false
            ),
            Some("spotify:search:Bad%20Bunny%20NUEVAYoL".to_string())
        );
        assert_eq!(
            link(
                "https://open.spotify.com/search/a%22%20--flag",
                AppKind::Spotify,
                false
            ),
            Some("spotify:search:a%22%20--flag".to_string())
        );
    }

    #[test]
    fn spotify_refuses_what_is_no_track() {
        for url in [
            "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPW",
            "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT1",
            "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPW-T",
            "https://open.spotify.com/album/4cOdK2wGLETKBW3PvgPWqT",
            "https://open.spotify.com/playlist/4cOdK2wGLETKBW3PvgPWqT",
            "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT/extra",
            "https://open.spotify.com/search/",
            "https://open.spotify.com/search/a%0Aplay",
            "https://open.spotify.com/",
        ] {
            assert_eq!(link(url, AppKind::Spotify, false), None, "{url}");
        }
    }

    #[test]
    fn apple_songs_open_in_cider_and_in_apples_app() {
        for (url, country, id) in [
            (
                "https://music.apple.com/us/song/kiss-of-life/950742158",
                "us",
                "950742158",
            ),
            (
                "https://music.apple.com/gb/song/950742158",
                "gb",
                "950742158",
            ),
            (
                "https://music.apple.com/us/album/nuevayol/1787022393?i=1787022572",
                "us",
                "1787022572",
            ),
            (
                "https://geo.music.apple.com/us/album/_/1787022393?i=1787022572&mt=1&app=music",
                "us",
                "1787022572",
            ),
        ] {
            assert_eq!(
                link(url, AppKind::Cider, false),
                Some(format!("cider://play/s/{id}")),
                "{url}"
            );
            assert_eq!(
                link(url, AppKind::AppleMusic, false),
                Some(format!("musics://music.apple.com/{country}/song/{id}")),
                "{url}"
            );
            assert_eq!(
                link(url, AppKind::AppleMusic, true),
                Some(format!("music://music.apple.com/{country}/song/{id}")),
                "{url}"
            );
        }
    }

    #[test]
    fn apple_refuses_albums_searches_and_bad_ids() {
        for url in [
            "https://music.apple.com/us/album/nuevayol/1787022393",
            "https://music.apple.com/us/search?term=Sade",
            "https://music.apple.com/us/song/kiss-of-life/95074215x",
            "https://music.apple.com/us/artist/sade/1",
            "https://music.apple.com/us/album/x/1?i=12345678901234",
        ] {
            assert_eq!(link(url, AppKind::Cider, false), None, "{url}");
            assert_eq!(link(url, AppKind::AppleMusic, false), None, "{url}");
        }
    }

    #[test]
    fn tidal_and_deezer_tracks_become_their_app_links() {
        for url in [
            "https://tidal.com/browse/track/412345678",
            "https://listen.tidal.com/track/412345678",
            "https://tidal.com/track/412345678/u",
        ] {
            assert_eq!(
                link(url, AppKind::Tidal, false),
                Some("tidal://track/412345678".to_string()),
                "{url}"
            );
        }
        for url in [
            "https://www.deezer.com/track/3112345678",
            "https://www.deezer.com/us/track/3112345678",
        ] {
            assert_eq!(
                link(url, AppKind::Deezer, false),
                Some("deezer://www.deezer.com/track/3112345678".to_string()),
                "{url}"
            );
        }
        for url in [
            "https://tidal.com/browse/album/412345678",
            "https://tidal.com/search?q=Sade",
            "https://tidal.com/browse/track/41x",
            "https://www.deezer.com/album/1",
            "https://www.deezer.com/search/Sade",
            "https://www.deezer.com/track/",
        ] {
            assert_eq!(link(url, AppKind::Tidal, false), None, "{url}");
            assert_eq!(link(url, AppKind::Deezer, false), None, "{url}");
        }
    }

    #[test]
    fn a_link_opens_only_in_its_own_services_app() {
        let spotify = "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT";
        let apple = "https://music.apple.com/us/song/950742158";
        assert_eq!(link(spotify, AppKind::Cider, false), None);
        assert_eq!(link(spotify, AppKind::Tidal, false), None);
        assert_eq!(link(apple, AppKind::Spotify, false), None);
        assert_eq!(
            link("https://www.deezer.com/track/1", AppKind::Tidal, false),
            None
        );
        // The service is the host's own, never a name somewhere in the address.
        for url in [
            "https://open.spotify.com.x.example/track/4cOdK2wGLETKBW3PvgPWqT",
            "https://x.example/?u=https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT",
            "spotify:track:4cOdK2wGLETKBW3PvgPWqT",
            "https://open.spotify.com:8443/track/4cOdK2wGLETKBW3PvgPWqT",
        ] {
            assert_eq!(item_of(url), None, "{url}");
        }
    }

    // ------------------------------------------------ the plan

    fn app(kind: AppKind, app: &str, launch: Launch, default: bool) -> SongApp {
        SongApp {
            service: kind.service(),
            app: app.to_string(),
            target: launch.target(),
            default,
            kind,
            launch,
        }
    }

    const CIDER: &str = "CiderCollective.Cider_a6qxe093bx5xj!App";
    const APPLE: &str = "AppleInc.AppleMusicWin_nzyj5cx40ttqa!App";

    fn both_apple_apps() -> Vec<SongApp> {
        vec![
            app(AppKind::Spotify, "Spotify", Launch::Scheme("spotify"), true),
            app(
                AppKind::Cider,
                "Cider",
                Launch::Packaged(CIDER.into()),
                true,
            ),
            app(
                AppKind::AppleMusic,
                "Apple Music",
                Launch::Packaged(APPLE.into()),
                false,
            ),
        ]
    }

    #[test]
    fn a_plain_open_takes_the_services_default_app() {
        let apps = both_apple_apps();
        assert_eq!(
            plan(
                "https://music.apple.com/us/song/950742158",
                None,
                &apps,
                false
            ),
            Ok(Plan::App {
                link: "cider://play/s/950742158".into(),
                launch: Launch::Packaged(CIDER.into()),
                kind: AppKind::Cider
            })
        );
        assert_eq!(
            plan(
                "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT",
                None,
                &apps,
                false
            ),
            Ok(Plan::App {
                link: "spotify:track:4cOdK2wGLETKBW3PvgPWqT".into(),
                launch: Launch::Scheme("spotify"),
                kind: AppKind::Spotify
            })
        );
    }

    #[test]
    fn a_named_app_is_used_only_when_it_is_on_the_list_for_that_service() {
        let apps = both_apple_apps();
        let apple = "https://music.apple.com/us/song/950742158";
        assert_eq!(
            plan(apple, Some(APPLE), &apps, false),
            Ok(Plan::App {
                link: "musics://music.apple.com/us/song/950742158".into(),
                launch: Launch::Packaged(APPLE.into()),
                kind: AppKind::AppleMusic
            })
        );
        // Not on the list, or another service's app: the browser.
        assert_eq!(
            plan(apple, Some("Evil.App_x!App"), &apps, false),
            Ok(Plan::Browser)
        );
        assert_eq!(
            plan(apple, Some("spotify"), &apps, false),
            Ok(Plan::Browser)
        );
        assert_eq!(plan(apple, Some(""), &apps, false), Ok(Plan::Browser));
    }

    #[test]
    fn the_browser_when_no_app_opens_the_link() {
        let apps = both_apple_apps();
        for url in [
            "https://music.apple.com/us/album/nuevayol/1787022393",
            "https://music.apple.com/us/search?term=Sade",
            "https://www.deezer.com/track/1",
            "https://music.youtube.com/watch?v=6U4n4bkOGs4",
            "https://song.link/s/1",
            "https://www.shazam.com/track/1/x",
        ] {
            assert_eq!(plan(url, None, &apps, false), Ok(Plan::Browser), "{url}");
        }
        assert_eq!(
            plan(
                "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT",
                None,
                &[],
                false
            ),
            Ok(Plan::Browser)
        );
    }

    #[test]
    fn what_is_no_song_link_is_refused() {
        for url in [
            "https://x.example/track/1",
            "javascript:alert(1)",
            "file:///C:/Windows/System32/calc.exe",
            "spotify:track:4cOdK2wGLETKBW3PvgPWqT",
            "https://open.spotify.com.x.example/track/4cOdK2wGLETKBW3PvgPWqT",
            "",
        ] {
            assert!(plan(url, None, &both_apple_apps(), false).is_err(), "{url}");
        }
    }

    #[test]
    fn the_page_sees_only_service_app_target_and_default() {
        let json = serde_json::to_value(&both_apple_apps()[1]).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "service": "Apple Music", "app": "Cider", "target": CIDER, "default": true })
        );
    }

    // ------------------------------------------------ the Windows walk

    #[derive(Default)]
    struct FakeRegistry {
        values: HashMap<(Hive, String), Vec<(String, String)>>,
        strings: HashMap<(Hive, String, String), String>,
        indirect: HashMap<String, String>,
    }

    impl FakeRegistry {
        fn value(mut self, hive: Hive, key: &str, name: &str, data: &str) -> Self {
            self.values
                .entry((hive, key.to_ascii_lowercase()))
                .or_default()
                .push((name.to_string(), data.to_string()));
            self.strings.insert(
                (hive, key.to_ascii_lowercase(), name.to_ascii_lowercase()),
                data.to_string(),
            );
            self
        }

        /// A packaged app as the Store files one: its registration, its URL
        /// associations and each ProgId's Application key.
        fn packaged(
            mut self,
            hive: Hive,
            reg: &str,
            name: &str,
            aumid: &str,
            claims: &[(&str, &str)],
        ) -> Self {
            let caps = format!(r"Software\Classes\Local Settings\Packages\{reg}\App\Capabilities");
            self = self.value(hive, r"SOFTWARE\RegisteredApplications", reg, &caps);
            for (scheme, prog) in claims {
                self = self
                    .value(hive, &format!(r"{caps}\URLAssociations"), scheme, prog)
                    .value(
                        Hive::ClassesRoot,
                        &format!(r"{prog}\Application"),
                        "ApplicationName",
                        name,
                    )
                    .value(
                        Hive::ClassesRoot,
                        &format!(r"{prog}\Application"),
                        "AppUserModelID",
                        aumid,
                    );
            }
            self
        }

        fn user_choice(self, scheme: &str, prog: &str) -> Self {
            self.value(
                Hive::CurrentUser,
                &format!(r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\{scheme}\UserChoice"),
                "ProgId",
                prog,
            )
        }
    }

    impl Registry for FakeRegistry {
        fn values(&self, hive: Hive, key: &str) -> Vec<(String, String)> {
            self.values
                .get(&(hive, key.to_ascii_lowercase()))
                .cloned()
                .unwrap_or_default()
        }
        fn string(&self, hive: Hive, key: &str, name: &str) -> Option<String> {
            self.strings
                .get(&(hive, key.to_ascii_lowercase(), name.to_ascii_lowercase()))
                .cloned()
        }
        fn has_value(&self, hive: Hive, key: &str, name: &str) -> bool {
            self.string(hive, key, name).is_some()
        }
        fn indirect(&self, raw: &str) -> Option<String> {
            self.indirect.get(raw).cloned()
        }
    }

    /// Cider from the Store, as it is filed on Brandon's PC.
    fn cider(reg: FakeRegistry) -> FakeRegistry {
        reg.packaged(
            Hive::CurrentUser,
            "AppXvevbnyqvc5jvhg4w4jftznp79j6s432e",
            "Cider",
            CIDER,
            &[
                ("cider", "AppX3mvs9cdd73g45jhdnfnfgbvsjrcgh4y6"),
                ("itunes", "AppXb9104sc411r6br687nzmjz9q3dpfd13k"),
                ("itms", "AppX59wt6sdgeg3m4tyhe4rbt1h9d0966he6"),
                ("music", "AppXp02hp4qgsksjsqrq69af1zsg405rhz7v"),
                ("musics", "AppXk9fvzdfrkzz7skzabzr6h1sygas32shp"),
            ],
        )
    }

    /// Apple's app from the Store, named by an indirect string.
    fn apple(mut reg: FakeRegistry) -> FakeRegistry {
        let raw = "@{AppleInc.AppleMusicWin_1.0.0.0_x64__nzyj5cx40ttqa?ms-resource://AppleInc.AppleMusicWin/Resources/AppName}";
        reg.indirect
            .insert(raw.to_string(), "Apple Music".to_string());
        reg.packaged(
            Hive::CurrentUser,
            "AppXapplemusic",
            raw,
            APPLE,
            &[
                ("music", "AppXapplemusicmusic"),
                ("musics", "AppXapplemusicmusics"),
            ],
        )
    }

    fn listed(apps: &[SongApp]) -> Vec<(&str, &str, &str, bool)> {
        apps.iter()
            .map(|a| (a.service, a.app.as_str(), a.target.as_str(), a.default))
            .collect()
    }

    #[test]
    fn cider_from_the_store_is_one_app_for_apple_music() {
        let apps = windows_apps(&cider(FakeRegistry::default()));
        assert_eq!(listed(&apps), vec![("Apple Music", "Cider", CIDER, true)]);
        assert_eq!(apps[0].launch, Launch::Packaged(CIDER.into()));
        assert_eq!(apps[0].kind, AppKind::Cider);
    }

    #[test]
    fn with_both_apple_apps_cider_is_the_default_unless_the_user_chose_apples() {
        let apps = windows_apps(&apple(cider(FakeRegistry::default())));
        assert_eq!(
            listed(&apps),
            vec![
                ("Apple Music", "Cider", CIDER, true),
                ("Apple Music", "Apple Music", APPLE, false)
            ]
        );
        assert_eq!(apps[1].kind, AppKind::AppleMusic);

        let chose_apple = windows_apps(
            &apple(cider(FakeRegistry::default())).user_choice("musics", "AppXapplemusicmusics"),
        );
        assert_eq!(
            listed(&chose_apple),
            vec![
                ("Apple Music", "Apple Music", APPLE, true),
                ("Apple Music", "Cider", CIDER, false)
            ]
        );

        let chose_cider = windows_apps(
            &apple(cider(FakeRegistry::default()))
                .user_choice("musics", "AppXk9fvzdfrkzz7skzabzr6h1sygas32shp"),
        );
        assert_eq!(
            listed(&chose_cider)[0],
            ("Apple Music", "Cider", CIDER, true)
        );
    }

    #[test]
    fn a_classic_scheme_counts_only_with_a_command_to_run() {
        // The key an uninstalled Spotify left behind: a URL protocol with no command.
        let left_behind =
            FakeRegistry::default().value(Hive::ClassesRoot, "spotify", "URL Protocol", "");
        assert!(windows_apps(&left_behind).is_empty());

        let installed = left_behind.value(
            Hive::ClassesRoot,
            r"spotify\shell\open\command",
            "",
            r#""C:\Users\x\AppData\Roaming\Spotify\Spotify.exe" --protocol-uri="%1""#,
        );
        let apps = windows_apps(&installed);
        assert_eq!(listed(&apps), vec![("Spotify", "Spotify", "spotify", true)]);
        assert_eq!(apps[0].launch, Launch::Scheme("spotify"));
    }

    #[test]
    fn a_packaged_copy_wins_over_the_same_app_found_by_its_scheme() {
        let reg = cider(FakeRegistry::default())
            .value(Hive::ClassesRoot, "cider", "URL Protocol", "")
            .value(
                Hive::ClassesRoot,
                r"cider\shell\open\command",
                "",
                r#""C:\Cider\Cider.exe" "%1""#,
            );
        let apps = windows_apps(&reg);
        assert_eq!(listed(&apps), vec![("Apple Music", "Cider", CIDER, true)]);
    }

    #[test]
    fn apple_musics_shared_schemes_need_a_packaged_app() {
        // A classic claim on `musics` could land in any app that claims it.
        let classic = FakeRegistry::default()
            .value(Hive::ClassesRoot, "musics", "URL Protocol", "")
            .value(
                Hive::ClassesRoot,
                r"musics\shell\open\command",
                "",
                r#""C:\x.exe" "%1""#,
            );
        assert!(windows_apps(&classic).is_empty());

        let caps = r"Software\Clients\Media\Player\Capabilities";
        let unpackaged = FakeRegistry::default()
            .value(
                Hive::LocalMachine,
                r"SOFTWARE\RegisteredApplications",
                "Player",
                caps,
            )
            .value(
                Hive::LocalMachine,
                &format!(r"{caps}\URLAssociations"),
                "musics",
                "Player.musics",
            )
            .value(
                Hive::ClassesRoot,
                r"Player.musics\Application",
                "ApplicationName",
                "Player",
            );
        assert!(windows_apps(&unpackaged).is_empty());
    }

    #[test]
    fn another_packaged_player_of_apples_schemes_shows_by_its_own_name() {
        let reg = FakeRegistry::default().packaged(
            Hive::LocalMachine,
            "AppXother",
            "iTunes",
            "AppleInc.iTunes_nzyj5cx40ttqa!iTunes",
            &[("musics", "AppXotheritunes")],
        );
        assert_eq!(
            listed(&windows_apps(&reg)),
            vec![(
                "Apple Music",
                "iTunes",
                "AppleInc.iTunes_nzyj5cx40ttqa!iTunes",
                true
            )]
        );
        // One with no name that reads is passed over.
        let nameless = FakeRegistry::default().packaged(
            Hive::LocalMachine,
            "AppXnameless",
            "@{Unresolvable?ms-resource://x}",
            "Some.Player_x!App",
            &[("musics", "AppXnameless1")],
        );
        assert!(windows_apps(&nameless).is_empty());
    }

    #[test]
    fn stale_registrations_and_other_schemes_are_passed_over() {
        let reg = FakeRegistry::default()
            // A registration whose package folder is gone.
            .value(
                Hive::CurrentUser,
                r"SOFTWARE\RegisteredApplications",
                "AppXv2k8pxjxvq02m41xa8f0xz9bsb6tpkj4",
                r"Software\Classes\Local Settings\Packages\CiderCollective.Cider_4.0.17.0\App\Capabilities",
            )
            // A browser's URL associations.
            .packaged(
                Hive::LocalMachine,
                "Browser",
                "Browser",
                "Browser.App_x!App",
                &[("http", "BrowserURL"), ("https", "BrowserURL")],
            );
        assert!(windows_apps(&reg).is_empty());
    }

    #[test]
    fn store_names_that_carry_the_service_are_tidied() {
        let reg = FakeRegistry::default().packaged(
            Hive::CurrentUser,
            "AppXspotify",
            "Spotify - Music and Podcasts",
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
            &[("spotify", "AppXspotifyprog")],
        );
        assert_eq!(
            listed(&windows_apps(&reg)),
            vec![(
                "Spotify",
                "Spotify",
                "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
                true
            )]
        );
    }

    #[test]
    fn the_list_runs_spotify_apple_music_tidal_deezer() {
        let reg = cider(FakeRegistry::default())
            .packaged(
                Hive::CurrentUser,
                "AppXdeezer",
                "Deezer",
                "Deezer.62021_x!Deezer",
                &[("deezer", "AppXdz")],
            )
            .packaged(
                Hive::CurrentUser,
                "AppXtidal",
                "TIDAL",
                "TIDAL.TIDAL_x!App",
                &[("tidal", "AppXtd")],
            )
            .value(Hive::ClassesRoot, "spotify", "URL Protocol", "")
            .value(
                Hive::ClassesRoot,
                r"spotify\shell\open\command",
                "",
                r#""Spotify.exe" "%1""#,
            );
        let services: Vec<&str> = windows_apps(&reg).iter().map(|a| a.service).collect();
        assert_eq!(services, vec!["Spotify", "Apple Music", "Tidal", "Deezer"]);
    }

    // ------------------------------------------------ macOS and Linux

    #[test]
    fn macos_finds_the_bundles_cider_first_for_apple_music() {
        let home = PathBuf::from("/Users/x");
        let there = |p: &Path| {
            [
                "/System/Applications/Music.app",
                "/Users/x/Applications/Cider.app",
                "/Applications/Spotify.app",
            ]
            .iter()
            .any(|q| Path::new(q) == p)
        };
        let apps = settle(mac_found(Some(home.as_path()), &there), &|_| None);
        let shown: Vec<(&str, &str, bool)> = apps
            .iter()
            .map(|a| (a.service, a.app.as_str(), a.default))
            .collect();
        assert_eq!(
            shown,
            vec![
                ("Spotify", "Spotify", true),
                ("Apple Music", "Cider", true),
                ("Apple Music", "Apple Music", false)
            ]
        );
        let Launch::Bundle(music) = &apps[2].launch else {
            panic!("Music is a bundle");
        };
        assert_eq!(
            Path::new(music),
            Path::new("/System/Applications/Music.app")
        );
        assert!(settle(mac_found(None, &|_| false), &|_| None).is_empty());
    }

    #[test]
    fn linux_finds_the_apps_with_a_scheme_handler() {
        let handler = |scheme: &str| match scheme {
            "spotify" => Some("spotify.desktop".to_string()),
            "cider" => Some("cider.desktop\n".to_string()),
            "tidal" => Some(String::new()),
            _ => None,
        };
        let apps = settle(linux_found(&handler), &|_| None);
        assert_eq!(
            listed(&apps),
            vec![
                ("Spotify", "Spotify", "spotify", true),
                ("Apple Music", "Cider", "cider", true)
            ]
        );
    }
}
