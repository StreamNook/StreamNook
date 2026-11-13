//! The Log action's file: one writer thread appends to
//! `logs/automations/YYYY-MM-DD.txt` under the file logger's folder. 5 MB a
//! day (then one Activity line and no more writes that day), files older than
//! 14 days pruned at start and each midnight. The support zip skips subfolders
//! already (`commands/logs.rs:157-161`), so the folder is never sent.

use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::Mutex;

pub const DAY_CAP_BYTES: u64 = 5 * 1024 * 1024;
pub const KEEP_DAYS: i64 = 14;

pub struct LogFile {
    tx: Mutex<Option<Sender<String>>>,
    dir: Option<PathBuf>,
    /// True once today's cap stopped writes (one Activity line, said by the caller).
    capped: std::sync::atomic::AtomicBool,
}

/// The folder: next to the file logger's, never inside the zip's listing.
pub fn real_dir() -> Option<PathBuf> {
    let file = crate::services::file_log::log_file_path().ok()?;
    Some(file.parent()?.join("automations"))
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Delete day files older than `KEEP_DAYS` (by their date name). Returns how many.
pub fn prune(dir: &std::path::Path, today: chrono::NaiveDate) -> usize {
    let Ok(read) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(date) = name
            .strip_suffix(".txt")
            .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        else {
            continue;
        };
        if (today - date).num_days() > KEEP_DAYS && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

impl LogFile {
    pub fn real() -> LogFile {
        LogFile::to_dir_opt(real_dir())
    }

    pub fn to_dir(dir: PathBuf) -> LogFile {
        LogFile::to_dir_opt(Some(dir))
    }

    /// No folder: `append` answers false and writes nothing (an engine with no files, every test kit).
    pub fn none() -> LogFile {
        LogFile::to_dir_opt(None)
    }

    fn to_dir_opt(dir: Option<PathBuf>) -> LogFile {
        LogFile {
            tx: Mutex::new(None),
            dir,
            capped: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn sender(&self) -> Option<Sender<String>> {
        let dir = self.dir.clone()?;
        let mut guard = self.tx.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            let (tx, rx) = channel::<String>();
            let _ = std::fs::create_dir_all(&dir);
            prune(&dir, chrono::Local::now().date_naive());
            std::thread::spawn(move || {
                for line in rx {
                    let path = dir.join(format!("{}.txt", today()));
                    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    if size + line.len() as u64 + 1 > DAY_CAP_BYTES {
                        continue;
                    }
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                    {
                        let _ = writeln!(f, "{line}");
                    }
                }
            });
            *guard = Some(tx);
        }
        guard.clone()
    }

    /// Queue one line. True while today's file is under its cap.
    pub fn append(&self, line: &str) -> bool {
        let Some(dir) = &self.dir else { return false };
        let path = dir.join(format!("{}.txt", today()));
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if size + line.len() as u64 + 1 > DAY_CAP_BYTES {
            self.capped
                .store(true, std::sync::atomic::Ordering::Relaxed);
            return false;
        }
        if let Some(tx) = self.sender() {
            let _ = tx.send(line.to_string());
        }
        true
    }

    /// Whether the cap has stopped writes (for the one Activity line).
    pub fn capped(&self) -> bool {
        self.capped.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn flush(&self) {
        // Lines are appended as they arrive; dropping the sender at exit ends the thread.
    }

    pub fn dir(&self) -> Option<PathBuf> {
        self.dir.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_older_than_14_days_are_pruned_and_others_kept() {
        let dir = std::env::temp_dir().join(format!("sn-logfile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in [
            "2026-10-01.txt",
            "2026-11-01.txt",
            "2026-11-12.txt",
            "notes.md",
        ] {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 13).unwrap();
        assert_eq!(prune(&dir, today), 2);
        assert!(dir.join("2026-11-12.txt").exists() && dir.join("notes.md").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_day_cap_stops_writes() {
        let dir = std::env::temp_dir().join(format!("sn-logcap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = LogFile::to_dir(dir.clone());
        std::fs::write(
            dir.join(format!("{}.txt", today())),
            vec![b'x'; DAY_CAP_BYTES as usize - 3],
        )
        .unwrap();
        assert!(!log.append("a line past the cap"));
        assert!(log.capped());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_support_zip_skips_the_folder() {
        // `zip_logs` reads only files directly in the logs folder (commands/logs.rs:157-161).
        let root = std::env::temp_dir().join(format!("sn-logzip-{}", std::process::id()));
        let logs = root.join("logs");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(logs.join("automations")).unwrap();
        std::fs::write(logs.join("automations").join("2026-11-13.txt"), "private").unwrap();
        std::fs::write(logs.join("streamnook.log"), "a").unwrap();
        let dest = root.join("out.zip");
        assert_eq!(crate::commands::logs::zip_logs(&logs, &dest).unwrap(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }
}
