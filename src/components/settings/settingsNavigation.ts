// Landing a settings search result on the setting itself. Shared by the full
// Settings dialog and the MultiChat chat-settings modal, which render the same
// rows and search the same index.

import { sectionIdFromLabel } from './sectionId';
import type { SettingsIndexEntry } from './searchIndex';

const rowsIn = (root: ParentNode): HTMLElement[] =>
  Array.from(root.querySelectorAll<HTMLElement>('[data-setting-row]'));

/** Exact title first, then a title that leads with it (a row whose title also
 *  shows its value, like "Text size: 14px"). */
const matchRow = (rows: HTMLElement[], title: string): HTMLElement | null =>
  rows.find((r) => r.dataset.settingRow === title) ??
  rows.find((r) => r.dataset.settingRow?.startsWith(`${title}:`)) ??
  rows.find((r) => r.dataset.settingRow?.startsWith(title)) ??
  null;

/** The settings row a search result names, matched on its title. Short row
 *  names repeat ("Color", "Volume"), so a top-level row wins over a line of the
 *  same name nested under another row. */
export function findSettingRow(root: ParentNode | null, title: string): HTMLElement | null {
  if (!root) return null;
  const all = rowsIn(root);
  const top = all.filter((r) => !r.parentElement?.closest('[data-setting-row], [data-setting-subgroup]'));
  return matchRow(top, title) ?? matchRow(all, title);
}

/** Where a result should land: its row inside its own section (inside its
 *  parent row, for a nested setting), else its section. Every section has a
 *  DOM id, declared or derived from its label, so an entry without an explicit
 *  sectionId still has somewhere to go. */
export function findSettingTarget(root: ParentNode | null, entry: SettingsIndexEntry): HTMLElement | null {
  const section =
    root?.querySelector<HTMLElement>(`[data-settings-section="${CSS.escape(entry.section)}"]`) ?? null;
  const scope = section ?? root;
  if (entry.parent) {
    const parent = findSettingRow(scope, entry.parent);
    if (parent) {
      // Nested inside the row (SubControl), or in the SettingsSubGroup that
      // follows it (the overlay builder's whole nested rows).
      const group = parent.nextElementSibling;
      const nested = [
        ...rowsIn(parent),
        ...(group instanceof HTMLElement && group.hasAttribute('data-setting-subgroup') ? rowsIn(group) : []),
      ];
      return matchRow(nested, entry.title) ?? parent;
    }
  }
  return (
    findSettingRow(scope, entry.title) ??
    section ??
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
