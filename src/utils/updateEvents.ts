/**
 * The name of the window event that starts an update install.
 *
 * The install flow (download, verify, stage, restart) lives in `TitleBar`, which
 * owns the progress overlay and the restart handoff. The changelog popup needs to
 * start it too: the update notification opens it on the new version's notes,
 * and a changelog with no install action next to it is a dead end that leaves
 * people on old versions.
 *
 * A window event rather than lifting the flow into the store, because the flow is
 * genuinely coupled to the overlay and the restart snapshot; duplicating it in a
 * second place is how the two would drift.
 */
export const START_UPDATE_EVENT = 'streamnook:start-update';

/** Ask whoever owns the install flow to begin. No-op if no window owns it. */
export function requestUpdateInstall(): void {
  window.dispatchEvent(new Event(START_UPDATE_EVENT));
}
