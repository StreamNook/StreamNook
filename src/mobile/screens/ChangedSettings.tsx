// Settings > Changed settings: every row you set differently from a fresh
// install, grouped by page, each with an arrow that puts it back and a tap that
// opens its page. Rust decides what counts as changed (`get_changed_settings`
// compares against `Settings::default()` over the shared catalog) and does the
// reset (`reset_settings`), so the iPhone and this page always agree.
import React, { createElement, useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { RotateCcw } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { SettingsSection } from '../../components/settings/_primitives';
import { EmptyNote } from '../ui/EmptyNote';
import { GROUP_ICON, GROUP_ICON_STROKE, GroupIcon } from '../ui/GroupedList';
import { useGroupRowFrame } from '../ui/groupSurface';
import { CatalogLogo } from './SettingsSearch';
import { settingIcon } from './settingsIcons';
import { CATALOG_PAGES, catalogRowKey, knownRows, openCatalogRow, pageViewOf, type CatalogRow } from './settingsCatalog';
import { Logger } from '../../utils/logger';

/** One changed setting: its glyph and name (a tap opens its page), where it
 *  lives, and the arrow that puts it back. A nested line wears the glyph of
 *  the row it sits under, as it does on its page. */
const ChangedRow: React.FC<{
  row: CatalogRow;
  busy: boolean;
  onReset: (row: CatalogRow) => void;
}> = ({ row, busy, onReset }) => {
  const frame = useGroupRowFrame();
  return (
    <div className={`settings-row ${frame} flex items-center pr-1`}>
      <button
        onClick={() => openCatalogRow(row)}
        className="min-w-0 flex-1 flex items-start gap-3 py-2.5 text-left active:opacity-70"
      >
        <GroupIcon className="text-textSecondary">
          {createElement(settingIcon(row.section, row.under ?? row.title), {
            size: GROUP_ICON,
            strokeWidth: GROUP_ICON_STROKE,
          })}
        </GroupIcon>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[15px] leading-6 text-textPrimary">
            <CatalogLogo logo={row.logo} size={14} />
            {row.title}
          </span>
          <span className="block truncate text-[13px] leading-[18px] text-textSecondary">{row.section}</span>
        </span>
      </button>
      {row.paths.length > 0 && (
        <button
          onClick={() => onReset(row)}
          disabled={busy}
          className="sn-touch flex shrink-0 items-center justify-center text-textSecondary active:text-textPrimary disabled:opacity-50"
          aria-label={`Reset ${row.title} to default`}
        >
          <RotateCcw size={15} />
        </button>
      )}
    </div>
  );
};

const ChangedSettings: React.FC = () => {
  const settings = useAppStore((s) => s.settings);
  const addToast = useAppStore((s) => s.addToast);
  const [rows, setRows] = useState<CatalogRow[] | null>(null);
  // The row whose reset is in flight, so a second tap cannot send it twice.
  const [resetting, setResetting] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setRows(knownRows(await invoke<CatalogRow[]>('get_changed_settings'), 'ChangedSettings'));
    } catch (e) {
      Logger.warn('[ChangedSettings] get_changed_settings failed:', e);
      setRows((prev) => prev ?? []);
    }
  }, []);

  // Re-read whenever the settings change: a reset here, or a change made on
  // another page, comes back through the settings broadcast into the store.
  useEffect(() => {
    void load();
  }, [load, settings]);

  const reset = async (row: CatalogRow) => {
    const key = catalogRowKey(row);
    if (resetting) return;
    setResetting(key);
    try {
      // No source: this window re-reads the settings too, so every open page
      // draws the default straight away.
      await invoke('reset_settings', { paths: row.paths });
      await load();
    } catch (e) {
      Logger.warn('[ChangedSettings] reset_settings failed:', e);
      addToast(`Could not reset ${row.title}: ${String(e)}`, 'error');
    } finally {
      setResetting(null);
    }
  };

  if (rows === null) {
    return <div className="py-10 text-center text-sm text-textMuted">Loading…</div>;
  }

  const pages = CATALOG_PAGES.filter((page) => rows.some((r) => r.page === page));

  return (
    <div className="space-y-6">
      <p className="px-4 text-[13px] leading-[18px] text-textMuted">
        Everything you set differently from a fresh install. The arrow puts one back; tap a row to open its page.
      </p>
      {rows.length === 0 && <EmptyNote text="Every setting is at its default." hint={null} />}
      {pages.map((page) => (
        <SettingsSection key={page} label={pageViewOf(page) ?? page}>
          {rows
            .filter((r) => r.page === page)
            .map((row) => (
              <ChangedRow
                key={catalogRowKey(row)}
                row={row}
                busy={resetting !== null}
                onReset={(r) => void reset(r)}
              />
            ))}
        </SettingsSection>
      ))}
    </div>
  );
};

export default ChangedSettings;
