// Landing a settings search result on the setting itself. Shared by the full
// Settings dialog and the MultiChat chat-settings modal, which render the same
// rows and search the same index.

import { sectionIdFromLabel } from './sectionId';
import type { SettingsIndexEntry } from './searchIndex';

/** The settings row a search result names, matched on its title. Exact first,
 *  then a title that leads with it (a row whose title also shows its value,
 *  like "Text size: 14px"). */
export function findSettingRow(root: ParentNode | null, title: string): HTMLElement | null {
  if (!root) return null;
  const rows = Array.from(root.querySelectorAll<HTMLElement>('[data-setting-row]'));
  return (
    rows.find((r) => r.dataset.settingRow === title) ??
    rows.find((r) => r.dataset.settingRow?.startsWith(`${title}:`)) ??
    rows.find((r) => r.dataset.settingRow?.startsWith(title)) ??
    null
  );
}

/** Where a result should land: its row when one matches, else its section.
 *  Every section has a DOM id, declared or derived from its label, so an entry
 *  without an explicit sectionId still has somewhere to go. */
export function findSettingTarget(root: ParentNode | null, entry: SettingsIndexEntry): HTMLElement | null {
  return (
    findSettingRow(root, entry.title) ??
    document.getElementById(entry.sectionId ?? sectionIdFromLabel(entry.section))
  );
}

/** Wash the row once with the accent, so the eye finds it in a long section. */
export function flashSettingRow(el: HTMLElement): void {
  if (!el.hasAttribute('data-setting-row')) return;
  el.classList.remove('settings-row-found');
  // Restart the animation when the same row is found twice in a row.
  void el.offsetWidth;
  el.classList.add('settings-row-found');
  window.setTimeout(() => el.classList.remove('settings-row-found'), 2000);
}
