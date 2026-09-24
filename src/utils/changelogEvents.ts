/**
 * The name of the window event that opens the changelog popup.
 *
 * The popup lives in `App`, which also decides when it opens on its own after an
 * update and records the version as seen. Settings, the command palette and the
 * update notification only ask for it, so there is one changelog surface rather
 * than a second copy of it inside Settings.
 */
export const OPEN_CHANGELOG_EVENT = 'streamnook:open-changelog';

export interface OpenChangelogDetail {
  /** Which release to open on. Omitted means the installed version. */
  version?: string;
}

/** Ask App to open the changelog popup. */
export function requestChangelog(version?: string): void {
  window.dispatchEvent(
    new CustomEvent<OpenChangelogDetail>(OPEN_CHANGELOG_EVENT, { detail: { version } }),
  );
}
