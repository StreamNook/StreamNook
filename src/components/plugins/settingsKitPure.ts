// The parts of the settings-page kit that are plain functions, kept apart from
// the React components so they can be tested without a DOM. settingsPageKit.tsx
// re-exports them.

/** How full a progress bar is, as a percent from 0 to 100. A total of zero or
 *  less, or a number that is not finite, reads as empty rather than as NaN. */
export function progressPercent(value: number, max: number): number {
  if (!Number.isFinite(value) || !Number.isFinite(max) || max <= 0) return 0;
  return Math.min(100, Math.max(0, (value / max) * 100));
}

/** Whether each fold is open or shut is remembered per page visit, in
 *  sessionStorage, under this key. */
export function disclosureKey(id: string): string {
  return `sn-disclosure:${id}`;
}

type ReadStore = Pick<Storage, 'getItem'>;
type WriteStore = Pick<Storage, 'setItem'>;

/** A fold's remembered state; shut when nothing is remembered or the browser
 *  will not say (private window, blocked site data). */
export function readDisclosureOpen(id: string, storage?: ReadStore): boolean {
  try {
    return (storage ?? globalThis.sessionStorage).getItem(disclosureKey(id)) === '1';
  } catch {
    return false;
  }
}

/** Remembers a fold's state; a browser that will not store it is ignored. */
export function writeDisclosureOpen(id: string, open: boolean, storage?: WriteStore): void {
  try {
    (storage ?? globalThis.sessionStorage).setItem(disclosureKey(id), open ? '1' : '0');
  } catch {
    // Nothing to remember it in; the fold still works.
  }
}
