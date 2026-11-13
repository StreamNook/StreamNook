//! "Start with Windows" (macOS "Open at login", Linux "Start at login"): the
//! login entry that starts StreamNook straight into the tray.
//!
//! One worker (`sync`) reads the LIVE settings and the provider state and
//! brings the entry in line, then reports what it found as `AutostartState`
//! for the Autopilot page. It wants the entry exactly when Missing badges, Keep
//! running when closed and Start at login are on and the provider is ready.
//! It never removes an entry while the provider state is unknown (the registry
//! may simply be unreadable), and a missing Linux tray host never removes one.
//!
//! - Windows: `HKCU\...\CurrentVersion\Run` value `StreamNook` = `"<exe>"
//!   --background` (quoted). A `StartupApproved\Run` value whose first byte has
//!   the low bit set is Task Manager's "Disabled": the worker never re-enables
//!   it on its own and reports `Blocked`. A stored command is rewritten only
//!   when the exe it names no longer exists; another existing exe (a second
//!   copy) is left and reported `OtherCopy`. Turning Start at login on in
//!   StreamNook is an explicit takeover (`request_takeover`): the next run
//!   writes the entry for this exe and clears the Task Manager mark.
//! - macOS: `~/Library/LaunchAgents/app.streamnook.background.plist` running
//!   `/usr/bin/open -b <bundle id> --args --background`, so a moved or trashed
//!   app never leaves a stale path.
//! - Linux: `~/.config/autostart/streamnook.desktop`, `Exec` quoted, the
//!   AppImage path when `APPIMAGE` is set.
//!
//! Only a release build writes; a debug build reports the state and never
//! writes, so a dev copy and the installed app do not fight over the entry.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::Manager;

use crate::models::settings::AppState;
use crate::rt::AppHandle;
use crate::services::background_mode::{self, Provider};

/// The value name under every key.
pub const ENTRY_NAME: &str = "StreamNook";
const BACKGROUND_ARG: &str = "--background";
#[allow(dead_code)]
const LAUNCH_AGENT_LABEL: &str = "app.streamnook.background";
/// `StartupApproved` bytes for "enabled" (first byte low bit clear).
#[allow(dead_code)]
const ENABLED_BYTES: [u8; 12] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// What the worker found or did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutostartState {
    On,
    Off,
    /// Task Manager (or the OS) has the entry disabled.
    Blocked,
    /// Another StreamNook copy owns the entry.
    OtherCopy,
    Error(String),
    #[allow(dead_code)]
    Unsupported,
}

impl AutostartState {
    /// The words the page reads.
    pub fn code(&self) -> &'static str {
        match self {
            AutostartState::On => "on",
            AutostartState::Off => "off",
            AutostartState::Blocked => "blocked",
            AutostartState::OtherCopy => "other_copy",
            AutostartState::Error(_) => "error",
            AutostartState::Unsupported => "unsupported",
        }
    }

    /// The problem in the viewer's words; the technical reason stays in the log
    /// (`run_sync` logs it).
    pub fn error(&self) -> Option<String> {
        match self {
            AutostartState::Error(reason) => Some(viewer_reason(reason).to_string()),
            _ => None,
        }
    }
}

/// Pure: a stored failure as one plain sentence for the page, which shows it
/// after "Could not set this up: ". Windows reasons name the registry call;
/// macOS and Linux reasons carry a file path and an io error.
pub fn viewer_reason(raw: &str) -> &'static str {
    let windows = raw.starts_with("Reg") || raw.contains("WIN32_ERROR");
    let denied = raw.contains("WIN32_ERROR(5)")
        || raw.contains("(os error 5)")
        || raw.contains("Access is denied");
    if windows && denied {
        "Windows refused to change its startup list."
    } else if windows {
        "Windows could not save the startup entry."
    } else {
        "the login entry in your home folder could not be saved."
    }
}

#[cfg(test)]
mod viewer_reason_tests {
    use super::*;

    #[test]
    fn autostart_errors_reach_the_page_in_plain_words() {
        assert_eq!(
            viewer_reason("RegSetValueExW StreamNook: WIN32_ERROR(5)"),
            "Windows refused to change its startup list."
        );
        assert_eq!(
            viewer_reason("RegOpenKeyExW Run: WIN32_ERROR(2)"),
            "Windows could not save the startup entry."
        );
        let mac = viewer_reason(
            "/Users/someone/Library/LaunchAgents/app.plist: Permission denied (os error 13)",
        );
        assert_eq!(
            mac,
            "the login entry in your home folder could not be saved."
        );
        assert!(!mac.contains("/Users/"));
        assert_eq!(
            AutostartState::Error("RegSetValueExW StreamNook: WIN32_ERROR(5)".into()).error(),
            Some("Windows refused to change its startup list.".to_string())
        );
        assert_eq!(AutostartState::On.error(), None);
    }
}

/// One whole sync at a time.
static WORKER: Mutex<()> = Mutex::new(());
static STATE: Mutex<AutostartState> = Mutex::new(AutostartState::Off);
/// A viewer's off-to-on of Start at login: the next run that wants the entry
/// writes it for this exe and clears a Task Manager mark.
static TAKEOVER_PENDING: AtomicBool = AtomicBool::new(false);

pub fn state() -> AutostartState {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn request_takeover() {
    TAKEOVER_PENDING.store(true, Ordering::Release);
}

pub fn clear_takeover() {
    TAKEOVER_PENDING.store(false, Ordering::Release);
}

// ---------------------------------------------------------------- pure rules

/// Pure: the entry exists exactly when all four hold.
pub fn autostart_wanted(
    collect: bool,
    keep_running: bool,
    start_at_login: bool,
    provider_ready: bool,
) -> bool {
    collect && keep_running && start_at_login && provider_ready
}

/// Pure: an unknown provider (unreadable registry) never removes an entry.
pub fn removal_allowed(provider: Provider) -> bool {
    provider != Provider::Unknown
}

/// Pure: only a release build writes.
pub fn writes_allowed(debug_assertions: bool) -> bool {
    !debug_assertions
}

fn writes_allowed_now() -> bool {
    writes_allowed(cfg!(debug_assertions))
}

/// Pure: the settings change that is an explicit takeover.
pub fn takeover_requested(previous_start_at_login: bool, next_start_at_login: bool) -> bool {
    !previous_start_at_login && next_start_at_login
}

/// Pure: `"<exe>" --background`.
pub fn windows_run_command(exe: &str) -> String {
    format!("\"{exe}\" {BACKGROUND_ARG}")
}

/// Pure: Task Manager's disable mark is the low bit of the first byte.
pub fn task_manager_disabled(bytes: &[u8]) -> bool {
    bytes.first().is_some_and(|b| b & 1 == 1)
}

/// Pure: the bytes to write to `StartupApproved\Run`: the enabled bytes on a
/// takeover, nothing otherwise (no other toggle touches the mark).
#[allow(dead_code)]
pub fn approved_bytes_to_write(takeover: bool) -> Option<[u8; 12]> {
    takeover.then_some(ENABLED_BYTES)
}

/// Pure: the exe a stored Run command names.
pub fn command_exe(command: &str) -> &str {
    let trimmed = command.trim();
    match trimmed.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(""),
        None => trimmed
            .strip_suffix(BACKGROUND_ARG)
            .unwrap_or(trimmed)
            .trim(),
    }
}

fn same_path(a: &str, b: &str) -> bool {
    a.replace('/', "\\")
        .eq_ignore_ascii_case(&b.replace('/', "\\"))
}

/// Who a stored Run command belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    This,
    /// The exe it names no longer exists.
    Missing,
    /// Another existing exe (a second copy).
    Other,
}

pub fn entry_owner(command: &str, this_exe: &str, exists: impl Fn(&str) -> bool) -> Owner {
    let exe = command_exe(command);
    if same_path(exe, this_exe) {
        Owner::This
    } else if !exists(exe) {
        Owner::Missing
    } else {
        Owner::Other
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPlan {
    Write,
    Keep,
    OtherCopy,
}

/// Pure: what the worker does with the Run value when it wants the entry.
/// A takeover always writes; otherwise only a missing entry or a stored exe
/// that no longer exists is written, and another existing copy is left alone.
pub fn plan_run(
    stored: Option<&str>,
    this_exe: &str,
    exists: impl Fn(&str) -> bool,
    takeover: bool,
) -> RunPlan {
    if takeover {
        return RunPlan::Write;
    }
    match stored {
        None => RunPlan::Write,
        Some(command) => match entry_owner(command, this_exe, exists) {
            Owner::This => RunPlan::Keep,
            Owner::Missing => RunPlan::Write,
            Owner::Other => RunPlan::OtherCopy,
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FilePlan {
    Write,
    Remove,
    Keep,
}

/// Pure (macOS and Linux files): write when the content differs, remove only
/// when not wanted and removal is allowed.
#[allow(dead_code)]
pub fn plan_file(existing: Option<&str>, wanted: Option<&str>, removal_allowed: bool) -> FilePlan {
    match (existing, wanted) {
        (Some(have), Some(want)) if have == want => FilePlan::Keep,
        (_, Some(_)) => FilePlan::Write,
        (Some(_), None) if removal_allowed => FilePlan::Remove,
        _ => FilePlan::Keep,
    }
}

#[allow(dead_code)]
fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Pure: the macOS LaunchAgent. `open -b` starts the app by bundle id.
#[allow(dead_code)]
pub fn launch_agent_plist(bundle_id: &str) -> String {
    let id = xml_escape(bundle_id);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
\t<key>Label</key>\n\
\t<string>{LAUNCH_AGENT_LABEL}</string>\n\
\t<key>ProgramArguments</key>\n\
\t<array>\n\
\t\t<string>/usr/bin/open</string>\n\
\t\t<string>-b</string>\n\
\t\t<string>{id}</string>\n\
\t\t<string>--args</string>\n\
\t\t<string>{BACKGROUND_ARG}</string>\n\
\t</array>\n\
\t<key>RunAtLoad</key>\n\
\t<true/>\n\
</dict>\n\
</plist>\n"
    )
}

/// Pure: a freedesktop `Exec` argument in double quotes. Inside quotes `"`,
/// a backtick and `$` take a backslash, and a backslash takes four because
/// the file's own string escaping runs first; `%` doubles.
#[allow(dead_code)]
pub fn desktop_exec_quote(path: &str) -> String {
    let mut out = String::from("\"");
    for c in path.chars() {
        match c {
            '"' | '`' | '$' => {
                out.push_str("\\\\");
                out.push(c);
            }
            '\\' => out.push_str("\\\\\\\\"),
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Pure: the Linux autostart entry.
#[allow(dead_code)]
pub fn desktop_entry(exe: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=StreamNook\nExec={} {BACKGROUND_ARG}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        desktop_exec_quote(exe)
    )
}

// ---------------------------------------------------------------- the worker

struct Wants {
    wanted: bool,
    removal_allowed: bool,
    takeover: bool,
    writes: bool,
    #[allow(dead_code)]
    bundle_id: String,
}

struct Outcome {
    state: AutostartState,
    takeover_done: bool,
}

/// Brings the entry in line on a blocking thread, then tells the page.
pub fn sync(app: &AppHandle) {
    let app = app.clone();
    let bundle_id = app.config().identifier.clone();
    tauri::async_runtime::spawn_blocking(move || run_sync(&app, bundle_id));
}

fn run_sync(app: &AppHandle, bundle_id: String) {
    let _guard = WORKER.lock().unwrap_or_else(|e| e.into_inner());
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let (collect, keep_running, start_at_login) = {
        let s = state.settings.lock().unwrap_or_else(|e| e.into_inner());
        (
            s.drops.collect_missing_badges,
            s.drops.keep_running_when_closed,
            s.drops.start_at_login,
        )
    };
    let provider = background_mode::provider();
    if !start_at_login {
        clear_takeover();
    }
    let wants = Wants {
        wanted: autostart_wanted(
            collect,
            keep_running,
            start_at_login,
            provider == Provider::Ready,
        ),
        removal_allowed: removal_allowed(provider),
        takeover: TAKEOVER_PENDING.load(Ordering::Acquire),
        writes: writes_allowed_now(),
        bundle_id,
    };
    let outcome = platform::sync(&wants);
    if outcome.takeover_done {
        clear_takeover();
    }
    if let AutostartState::Error(reason) = &outcome.state {
        log::warn!("[Autostart] {reason}");
    }
    *STATE.lock().unwrap_or_else(|e| e.into_inner()) = outcome.state;
    background_mode::emit_state_changed(app);
}

/// Removes the login entry now (a `--background` launch with Missing badges
/// off). A debug build writes nothing.
pub fn remove() {
    if !writes_allowed_now() {
        return;
    }
    let _guard = WORKER.lock().unwrap_or_else(|e| e.into_inner());
    match platform::remove() {
        Ok(()) => *STATE.lock().unwrap_or_else(|e| e.into_inner()) = AutostartState::Off,
        Err(reason) => log::warn!("[Autostart] could not remove the login entry: {reason}"),
    }
}

/// Shared by macOS and Linux: one file, written when it differs, removed when
/// not wanted.
#[allow(dead_code)]
fn file_sync(path: Option<std::path::PathBuf>, content: Option<String>, w: &Wants) -> Outcome {
    let Some(path) = path else {
        return Outcome {
            state: AutostartState::Error("no home folder for the login entry".into()),
            takeover_done: false,
        };
    };
    let existing = std::fs::read_to_string(&path).ok();
    let plan = plan_file(existing.as_deref(), content.as_deref(), w.removal_allowed);
    let result: Result<(), String> = if !w.writes {
        Ok(())
    } else {
        match plan {
            FilePlan::Write => path
                .parent()
                .map_or(Ok(()), |dir| std::fs::create_dir_all(dir))
                .and_then(|()| std::fs::write(&path, content.as_deref().unwrap_or("")))
                .map_err(|e| format!("{}: {e}", path.display())),
            FilePlan::Remove => {
                std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))
            }
            FilePlan::Keep => Ok(()),
        }
    };
    match result {
        Err(reason) => Outcome {
            state: AutostartState::Error(reason),
            takeover_done: false,
        },
        Ok(()) => {
            let present = match (w.writes, plan) {
                (true, FilePlan::Write) => true,
                (true, FilePlan::Remove) => false,
                _ => existing.is_some(),
            };
            Outcome {
                state: if present {
                    AutostartState::On
                } else {
                    AutostartState::Off
                },
                takeover_done: w.takeover && present,
            }
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows::core::HSTRING;
    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
        RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, KEY_WRITE, REG_BINARY,
        REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE,
    };

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const APPROVED_KEY: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

    /// An open key, closed when dropped.
    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the key was opened by a Reg call and is closed once.
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }

    fn open(path: &str, access: REG_SAM_FLAGS) -> Result<Option<Key>, String> {
        let mut key = HKEY::default();
        // SAFETY: `key` is a valid out pointer; the path outlives the call.
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                &HSTRING::from(path),
                None,
                access,
                &mut key,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status.is_err() {
            return Err(format!("RegOpenKeyExW {path}: {status:?}"));
        }
        Ok(Some(Key(key)))
    }

    fn create(path: &str) -> Result<Key, String> {
        let mut key = HKEY::default();
        // SAFETY: as in `open`; the key is closed by `Key`.
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                &HSTRING::from(path),
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut key,
                None,
            )
        };
        if status.is_err() {
            return Err(format!("RegCreateKeyExW {path}: {status:?}"));
        }
        Ok(Key(key))
    }

    fn read(path: &str, name: &str) -> Result<Option<Vec<u8>>, String> {
        let Some(key) = open(path, KEY_READ)? else {
            return Ok(None);
        };
        let name = HSTRING::from(name);
        let mut kind = REG_VALUE_TYPE::default();
        let mut size = 0u32;
        // SAFETY: the first call asks only for the value's size.
        let status =
            unsafe { RegQueryValueExW(key.0, &name, None, Some(&mut kind), None, Some(&mut size)) };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status.is_err() {
            return Err(format!("RegQueryValueExW {path}: {status:?}"));
        }
        let mut data = vec![0u8; size as usize];
        let mut read_size = size;
        // SAFETY: `data` holds `size` bytes, which `read_size` states.
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                &name,
                None,
                Some(&mut kind),
                Some(data.as_mut_ptr()),
                Some(&mut read_size),
            )
        };
        if status.is_err() {
            return Err(format!("RegQueryValueExW {path}: {status:?}"));
        }
        data.truncate(read_size as usize);
        Ok(Some(data))
    }

    fn sz_text(bytes: &[u8]) -> String {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let end = units.iter().position(|u| *u == 0).unwrap_or(units.len());
        String::from_utf16_lossy(&units[..end])
    }

    fn write_sz(path: &str, name: &str, value: &str) -> Result<(), String> {
        let key = create(path)?;
        let wide: Vec<u8> = value
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        // SAFETY: plain call with owned buffers; the key closes on drop.
        let status =
            unsafe { RegSetValueExW(key.0, &HSTRING::from(name), None, REG_SZ, Some(&wide)) };
        if status.is_err() {
            return Err(format!("RegSetValueExW {name}: {status:?}"));
        }
        Ok(())
    }

    fn write_binary(path: &str, name: &str, bytes: &[u8]) -> Result<(), String> {
        let key = create(path)?;
        // SAFETY: as in `write_sz`.
        let status =
            unsafe { RegSetValueExW(key.0, &HSTRING::from(name), None, REG_BINARY, Some(bytes)) };
        if status.is_err() {
            return Err(format!("RegSetValueExW {name}: {status:?}"));
        }
        Ok(())
    }

    fn delete(path: &str, name: &str) -> Result<(), String> {
        let Some(key) = open(path, KEY_SET_VALUE)? else {
            return Ok(());
        };
        // SAFETY: plain call; the key closes on drop.
        let status = unsafe { RegDeleteValueW(key.0, &HSTRING::from(name)) };
        if status == ERROR_FILE_NOT_FOUND || !status.is_err() {
            return Ok(());
        }
        Err(format!("RegDeleteValueW {name}: {status:?}"))
    }

    pub(super) fn sync(w: &Wants) -> Outcome {
        match run(w) {
            Ok(outcome) => outcome,
            Err(reason) => Outcome {
                state: AutostartState::Error(reason),
                takeover_done: false,
            },
        }
    }

    fn exists(path: &str) -> bool {
        std::path::Path::new(path).exists()
    }

    fn run(w: &Wants) -> Result<Outcome, String> {
        let exe = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .display()
            .to_string();
        let stored = read(RUN_KEY, ENTRY_NAME)?.map(|b| sz_text(&b));
        if w.wanted {
            let mut takeover_done = false;
            match plan_run(stored.as_deref(), &exe, exists, w.takeover) {
                RunPlan::OtherCopy => {
                    return Ok(Outcome {
                        state: AutostartState::OtherCopy,
                        takeover_done,
                    })
                }
                RunPlan::Write if w.writes => {
                    write_sz(RUN_KEY, ENTRY_NAME, &windows_run_command(&exe))?;
                    if let Some(bytes) = approved_bytes_to_write(w.takeover) {
                        write_binary(APPROVED_KEY, ENTRY_NAME, &bytes)?;
                    }
                    takeover_done = w.takeover;
                }
                RunPlan::Write | RunPlan::Keep => {}
            }
            let stored_now = read(RUN_KEY, ENTRY_NAME)?;
            let blocked =
                read(APPROVED_KEY, ENTRY_NAME)?.is_some_and(|b| task_manager_disabled(&b));
            let state = if blocked {
                AutostartState::Blocked
            } else if stored_now.is_some() {
                AutostartState::On
            } else {
                AutostartState::Off
            };
            return Ok(Outcome {
                state,
                takeover_done,
            });
        }
        // Not wanted: remove this copy's entry (or a stale one), never another copy's.
        let ours = stored
            .as_deref()
            .is_some_and(|c| entry_owner(c, &exe, exists) != Owner::Other);
        if ours && w.removal_allowed && w.writes {
            delete(RUN_KEY, ENTRY_NAME)?;
            return Ok(Outcome {
                state: AutostartState::Off,
                takeover_done: false,
            });
        }
        let state = if ours {
            AutostartState::On
        } else {
            AutostartState::Off
        };
        Ok(Outcome {
            state,
            takeover_done: false,
        })
    }

    /// Removes this copy's entry (or a stale one); another existing copy's stays.
    pub(super) fn remove() -> Result<(), String> {
        let exe = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .display()
            .to_string();
        let ours = read(RUN_KEY, ENTRY_NAME)?
            .map(|b| sz_text(&b))
            .is_some_and(|c| entry_owner(&c, &exe, exists) != Owner::Other);
        if ours {
            delete(RUN_KEY, ENTRY_NAME)
        } else {
            Ok(())
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    fn path() -> Option<std::path::PathBuf> {
        dirs::home_dir().map(|home| {
            home.join("Library")
                .join("LaunchAgents")
                .join(format!("{LAUNCH_AGENT_LABEL}.plist"))
        })
    }

    pub(super) fn sync(w: &Wants) -> Outcome {
        file_sync(
            path(),
            w.wanted.then(|| launch_agent_plist(&w.bundle_id)),
            w,
        )
    }

    pub(super) fn remove() -> Result<(), String> {
        match path() {
            Some(p) if p.exists() => std::fs::remove_file(&p).map_err(|e| e.to_string()),
            _ => Ok(()),
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;

    fn path() -> Option<std::path::PathBuf> {
        dirs::config_dir().map(|dir| dir.join("autostart").join("streamnook.desktop"))
    }

    /// The AppImage when there is one (the exe inside it is a temporary mount).
    fn exe() -> Option<String> {
        std::env::var_os("APPIMAGE")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::current_exe().ok())
            .map(|p| p.display().to_string())
    }

    pub(super) fn sync(w: &Wants) -> Outcome {
        let content = if w.wanted {
            exe().map(|e| desktop_entry(&e))
        } else {
            None
        };
        file_sync(path(), content, w)
    }

    pub(super) fn remove() -> Result<(), String> {
        match path() {
            Some(p) if p.exists() => std::fs::remove_file(&p).map_err(|e| e.to_string()),
            _ => Ok(()),
        }
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod platform {
    use super::*;

    pub(super) fn sync(_w: &Wants) -> Outcome {
        Outcome {
            state: AutostartState::Unsupported,
            takeover_done: false,
        }
    }

    pub(super) fn remove() -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Users\Viewer\AppData\Local\StreamNook\StreamNook.exe";

    // 26
    #[test]
    fn autostart_wanted_needs_all_four() {
        assert!(autostart_wanted(true, true, true, true));
        assert!(!autostart_wanted(false, true, true, true));
        assert!(!autostart_wanted(true, false, true, true));
        assert!(!autostart_wanted(true, true, false, true));
        assert!(!autostart_wanted(true, true, true, false));
    }

    #[test]
    fn windows_run_command_is_quoted() {
        assert_eq!(
            windows_run_command(r"C:\Program Files\StreamNook\StreamNook.exe"),
            "\"C:\\Program Files\\StreamNook\\StreamNook.exe\" --background"
        );
        // And reads back to the same exe.
        assert_eq!(command_exe(&windows_run_command(EXE)), EXE);
        assert_eq!(command_exe(&format!("{EXE} --background")), EXE);
    }

    #[test]
    fn a_task_manager_disable_is_blocked() {
        assert!(task_manager_disabled(&[3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert!(task_manager_disabled(&[7, 1, 2]));
        assert!(!task_manager_disabled(&[
            2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ]));
        assert!(!task_manager_disabled(&[6, 0, 0, 0]));
        assert!(!task_manager_disabled(&[]));
        assert!(!task_manager_disabled(&ENABLED_BYTES));
    }

    #[test]
    fn only_a_missing_exe_is_rewritten() {
        let exists = |p: &str| p.contains("Other");
        let mine = windows_run_command(EXE);
        // No entry: write. This exe: keep.
        assert_eq!(plan_run(None, EXE, exists, false), RunPlan::Write);
        assert_eq!(plan_run(Some(&mine), EXE, exists, false), RunPlan::Keep);
        assert_eq!(
            plan_run(Some(&mine.to_uppercase()), EXE, exists, false),
            RunPlan::Keep
        );
        // A stored exe that no longer exists is rewritten.
        let gone = windows_run_command(r"C:\Old\StreamNook.exe");
        assert_eq!(plan_run(Some(&gone), EXE, exists, false), RunPlan::Write);
        // Another existing copy is left alone.
        let other = windows_run_command(r"C:\Other\StreamNook.exe");
        assert_eq!(
            plan_run(Some(&other), EXE, exists, false),
            RunPlan::OtherCopy
        );
    }

    #[test]
    fn unknown_provider_never_removes() {
        assert!(!removal_allowed(Provider::Unknown));
        assert!(removal_allowed(Provider::Ready));
        assert!(removal_allowed(Provider::NotReady));
        // The same rule for the files: not wanted, entry present.
        assert_eq!(plan_file(Some("x"), None, false), FilePlan::Keep);
        assert_eq!(plan_file(Some("x"), None, true), FilePlan::Remove);
        assert_eq!(plan_file(None, None, true), FilePlan::Keep);
    }

    #[test]
    fn debug_builds_never_write() {
        assert!(!writes_allowed(true));
        assert!(writes_allowed(false));
    }

    // 26a
    #[test]
    fn turning_start_at_login_on_takes_over_another_copy_and_clears_the_mark() {
        assert!(takeover_requested(false, true));
        let exists = |_: &str| true;
        let other = windows_run_command(r"C:\Other\StreamNook.exe");
        assert_eq!(
            plan_run(Some(&other), EXE, exists, false),
            RunPlan::OtherCopy
        );
        assert_eq!(plan_run(Some(&other), EXE, exists, true), RunPlan::Write);
        // The mark is cleared by writing the enabled bytes.
        let bytes = approved_bytes_to_write(true).expect("a takeover writes the mark");
        assert!(!task_manager_disabled(&bytes));
    }

    #[test]
    fn other_toggles_leave_the_mark() {
        assert!(!takeover_requested(true, true));
        assert!(!takeover_requested(false, false));
        assert!(!takeover_requested(true, false));
        assert_eq!(approved_bytes_to_write(false), None);
    }

    #[test]
    fn the_macos_and_linux_entries_are_escaped_and_quoted() {
        let plist = launch_agent_plist("app.stream<nook>&co");
        assert!(plist.contains("<string>/usr/bin/open</string>"));
        assert!(plist.contains("<string>app.stream&lt;nook&gt;&amp;co</string>"));
        assert!(plist.contains("<string>--background</string>"));
        assert_eq!(
            desktop_exec_quote("/opt/Stream Nook/app"),
            "\"/opt/Stream Nook/app\""
        );
        assert_eq!(desktop_exec_quote("/a$b"), "\"/a\\\\$b\"");
        assert!(desktop_entry("/opt/s n/app").contains("Exec=\"/opt/s n/app\" --background\n"));
    }

    #[test]
    fn file_plans_write_only_what_differs() {
        assert_eq!(plan_file(None, Some("a"), false), FilePlan::Write);
        assert_eq!(plan_file(Some("a"), Some("a"), false), FilePlan::Keep);
        assert_eq!(plan_file(Some("b"), Some("a"), false), FilePlan::Write);
    }
}
