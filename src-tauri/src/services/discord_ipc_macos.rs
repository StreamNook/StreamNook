//! macOS-only Discord IPC transport.
//!
//! Why this file exists at all: `discord-rich-presence` ships one Unix socket
//! finder for both Linux and macOS, and on macOS it is both too narrow and
//! impossible to diagnose.
//!
//! Too narrow, in three ways:
//!
//!   1. It searches `XDG_RUNTIME_DIR`, `TMPDIR`, `TMP` and `TEMP` and then
//!      gives up. **There is no `/tmp` fallback**, which every reference
//!      implementation carries (discord-rpc, discord.js RPC, pypresence all end
//!      their search at `/tmp`). A Mac whose `TMPDIR` is unset or pointed
//!      somewhere else by the launching environment finds nothing.
//!   2. Discord is reported to place `discord-ipc-N` in the **parent** of the
//!      per-user temp dir on some machines: `/var/folders/<hash>/` rather than
//!      `/var/folders/<hash>/T/`. The crate only ever descends into
//!      subdirectories (its Linux flatpak/snap paths), never up.
//!   3. Its own subdirectory list is entirely Linux packaging (`flatpak`,
//!      `snap`), so on macOS it multiplies the search by six for nothing.
//!
//! Impossible to diagnose, in one way that cost a release: every miss is logged
//! at `debug`, and `streamnook.log` is Info at best (Warn with diagnostics
//! off), so a Mac that cannot find the socket produces **zero** evidence. The
//! service layer then converts the error to `Ok(())` so the stream is never
//! blocked, which is right for the user and leaves nothing at all to read.
//!
//! What this is NOT: a reimplementation of the IPC protocol. `DiscordIpc`
//! provides `send`, `recv`, `set_activity` and `clear_activity` on top of four
//! required transport methods, and only those four are here. Windows and Linux
//! keep the crate's own client untouched, so the platform with all the users
//! cannot regress from this file.

use discord_rich_presence::{error::Error, DiscordIpc};
use serde_json::json;
use std::env::var;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

type Result<T> = std::result::Result<T, Error>;

/// Discord numbers its sockets when several clients (stable, PTB, Canary) run
/// at once. Nine is the documented ceiling.
const MAX_SOCKET_INDEX: u8 = 10;

/// The full "we looked everywhere" report is worth a warning exactly once per
/// run. After that it is one line of noise per stream open, because
/// `update_presence` retries the connection every time it has no client.
static REPORTED_MISSING: AtomicBool = AtomicBool::new(false);

/// Minimal Discord IPC client for macOS.
pub struct MacDiscordIpcClient {
    client_id: String,
    socket: Option<UnixStream>,
}

impl MacDiscordIpcClient {
    /// Same signature as the crate's own client so the call site is identical
    /// on every platform.
    pub fn new<T: AsRef<str>>(client_id: T) -> Self {
        Self {
            client_id: client_id.as_ref().to_string(),
            socket: None,
        }
    }

    /// Every directory Discord is known to put its socket in on macOS, most
    /// likely first, de-duplicated.
    ///
    /// `/tmp` is a symlink to `/private/tmp` and `/var` one to `/private/var`,
    /// so the short spellings cover both; `exists()` follows symlinks.
    fn candidate_dirs() -> Vec<PathBuf> {
        fn push(dirs: &mut Vec<PathBuf>, dir: PathBuf) {
            if dir.is_dir() && !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }

        let mut dirs: Vec<PathBuf> = Vec::new();

        // The normal location: Electron's os.tmpdir(), which is $TMPDIR.
        if let Ok(tmpdir) = var("TMPDIR") {
            let tmpdir = PathBuf::from(tmpdir);
            // The parent-directory variant. Checked second so a machine with
            // the socket in both places still prefers the documented spot.
            // A TMPDIR with the usual trailing slash still yields the right
            // parent: path components ignore it.
            let parent = tmpdir.parent().map(|p| p.to_path_buf());
            push(&mut dirs, tmpdir);
            if let Some(parent) = parent {
                push(&mut dirs, parent);
            }
        }

        // Set by some launchers and by people forwarding the socket over ssh.
        for key in ["XDG_RUNTIME_DIR", "TMP", "TEMP"] {
            if let Ok(dir) = var(key) {
                push(&mut dirs, PathBuf::from(dir));
            }
        }

        // The fallback the crate is missing.
        push(&mut dirs, PathBuf::from("/tmp"));

        dirs
    }

    /// Locate a live `discord-ipc-N` socket, or report every place we looked.
    fn find_socket() -> Option<PathBuf> {
        let dirs = Self::candidate_dirs();

        for dir in &dirs {
            for index in 0..MAX_SOCKET_INDEX {
                let path = dir.join(format!("discord-ipc-{index}"));
                if path.exists() {
                    log::info!("[Discord] IPC socket found at {}", path.display());
                    // A later run that finds the socket should be able to warn
                    // again if it ever disappears.
                    REPORTED_MISSING.store(false, Ordering::Relaxed);
                    return Some(path);
                }
            }
        }

        let searched: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
        let report = format!(
            "[Discord] no discord-ipc-0..{} socket in any of: {}. \
             Discord is probably not running; if it is, this is the macOS socket-path gap.",
            MAX_SOCKET_INDEX - 1,
            if searched.is_empty() {
                "(no readable candidate directories)".to_string()
            } else {
                searched.join(", ")
            }
        );
        if REPORTED_MISSING.swap(true, Ordering::Relaxed) {
            log::debug!("{report}");
        } else {
            log::warn!("{report}");
        }
        None
    }
}

impl DiscordIpc for MacDiscordIpcClient {
    fn get_client_id(&self) -> &str {
        &self.client_id
    }

    fn connect_ipc(&mut self) -> Result<()> {
        let path = Self::find_socket().ok_or(Error::IPCNotFound)?;

        match UnixStream::connect(&path) {
            Ok(socket) => {
                self.socket = Some(socket);
                Ok(())
            }
            Err(e) => {
                // Distinct from IPCNotFound on purpose: the socket exists and
                // Discord still refused us, which is a different problem
                // (permissions, a dead socket left by a crashed client).
                log::warn!(
                    "[Discord] IPC socket at {} exists but would not connect: {e}",
                    path.display()
                );
                Err(Error::IPCConnectionFailed)
            }
        }
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        let socket = self.socket.as_mut().ok_or(Error::NotConnected)?;
        socket.write_all(data).map_err(Error::WriteError)
    }

    fn read(&mut self, buffer: &mut [u8]) -> Result<()> {
        let socket = self.socket.as_mut().ok_or(Error::NotConnected)?;
        socket.read_exact(buffer).map_err(Error::ReadError)
    }

    fn close(&mut self) -> Result<()> {
        // Opcode 2 is CLOSE. Best effort: a Discord that already went away
        // makes this fail, and there is nothing useful to do about it.
        let _ = self.send(json!({}), 2);

        let socket = self.socket.as_mut().ok_or(Error::NotConnected)?;
        socket.flush().map_err(Error::FlushError)?;
        let _ = socket.shutdown(Shutdown::Both);
        Ok(())
    }
}
