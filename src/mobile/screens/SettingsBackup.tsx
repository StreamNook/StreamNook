// Settings > Backup: the daily copies Rust keeps of your settings (and puts
// back on its own when the settings file will not load), each one restorable.
// Rust lists and restores them (`list_settings_backups`,
// `restore_settings_backup`); a restore comes back to this window through the
// settings broadcast like any other write.
import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { CalendarDays } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { SettingsRow, SettingsSection } from '../../components/settings/_primitives';
import { Logger } from '../../utils/logger';

/** Mirrors Rust `settings_backups::BackupInfo`. */
interface BackupInfo {
  /** The day it was saved, `YYYY-MM-DD`; also how it is restored. */
  id: string;
  bytes: number;
}

/** How long an armed Restore waits for the second tap. */
const ARM_MS = 3000;

/** "Today", "Yesterday", else "Wednesday, Oct 15". */
function dayLabel(id: string): string {
  const [y, m, d] = id.split('-').map(Number);
  if (!y || !m || !d) return id;
  const day = new Date(y, m - 1, d);
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const diff = Math.round((today.getTime() - day.getTime()) / 86_400_000);
  if (diff === 0) return 'Today';
  if (diff === 1) return 'Yesterday';
  return day.toLocaleDateString('en-US', { weekday: 'long', month: 'short', day: 'numeric' });
}

const sizeLabel = (bytes: number): string =>
  bytes < 1024 ? `${bytes} bytes` : `${Math.max(1, Math.floor(bytes / 1024))} KB`;

const SettingsBackup: React.FC = () => {
  const addToast = useAppStore((s) => s.addToast);
  const [backups, setBackups] = useState<BackupInfo[] | null>(null);
  // Two taps to restore: the first arms one copy, the second replaces your
  // settings with it. Arming another copy disarms the first.
  const [armed, setArmed] = useState<string | null>(null);
  const [restoring, setRestoring] = useState<string | null>(null);
  // Success toasts are hidden on the phone, so the restored copy's own row
  // says it happened.
  const [restored, setRestored] = useState<string | null>(null);
  const armTimer = useRef<number | undefined>(undefined);

  useEffect(() => {
    let cancelled = false;
    invoke<BackupInfo[]>('list_settings_backups')
      .then((list) => {
        if (!cancelled) setBackups(list);
      })
      .catch((e) => {
        Logger.warn('[SettingsBackup] list_settings_backups failed:', e);
        if (!cancelled) setBackups([]);
      });
    return () => {
      cancelled = true;
      window.clearTimeout(armTimer.current);
    };
  }, []);

  const tapRestore = async (id: string) => {
    if (restoring) return;
    if (armed !== id) {
      setArmed(id);
      window.clearTimeout(armTimer.current);
      armTimer.current = window.setTimeout(() => setArmed(null), ARM_MS);
      return;
    }
    window.clearTimeout(armTimer.current);
    setArmed(null);
    setRestoring(id);
    try {
      await invoke('restore_settings_backup', { id });
      setRestored(id);
    } catch (e) {
      Logger.warn('[SettingsBackup] restore_settings_backup failed:', e);
      addToast(String(e), 'error');
    } finally {
      setRestoring(null);
    }
  };

  return (
    <div className="space-y-7">
      <SettingsSection
        label="Daily copies"
        description="StreamNook keeps a copy of your settings each day you use it, for the last 7 days. If your settings ever fail to load, the newest copy is put back on its own."
      >
        {backups === null ? (
          <p className="py-3 text-[13px] text-textMuted">Loading…</p>
        ) : backups.length === 0 ? (
          <p className="py-3 text-[13px] text-textMuted">The first copy is saved the next time a setting changes.</p>
        ) : (
          backups.map((backup) => {
            const isArmed = armed === backup.id;
            return (
              <SettingsRow
                key={backup.id}
                // A day's name is no setting the glyph table could know.
                icon={CalendarDays}
                title={dayLabel(backup.id)}
                description={
                  restored === backup.id ? 'Restored. Your settings now match this day.' : sizeLabel(backup.bytes)
                }
                control={
                  <button
                    onClick={() => void tapRestore(backup.id)}
                    disabled={restoring !== null}
                    className="sn-touch flex shrink-0 items-center disabled:opacity-50"
                    aria-label={isArmed ? `Replace current settings with ${dayLabel(backup.id)}` : `Restore ${dayLabel(backup.id)}`}
                  >
                    {/* The tap target stays full size; the pill inside is what you see. */}
                    <span
                      className={`whitespace-nowrap rounded-full px-3 py-1.5 text-[12.5px] font-semibold transition-colors ${
                        isArmed ? 'bg-error/15 text-error' : 'glass-button text-textSecondary'
                      }`}
                    >
                      {restoring === backup.id ? 'Restoring…' : isArmed ? 'Replace current' : 'Restore'}
                    </span>
                  </button>
                }
              />
            );
          })
        )}
      </SettingsSection>
    </div>
  );
};

export default SettingsBackup;
