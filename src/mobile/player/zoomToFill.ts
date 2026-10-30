// The one writer of `video_player.zoom_to_fill` on Android (SN-30): a pinch on
// the landscape picture, the Player switch and its reset arrow all come here.
// The page shows its choice at once and holds it while its writes are out;
// once every write has settled the store leads again, so a change made
// elsewhere (a reset, a restore) shows from then on. Ordering every settings
// read and write is its own plan; this keeps to one field.
import { useSyncExternalStore } from 'react';
import { useAppStore } from '../../stores/AppStore';
import { setSettingsFields } from '../../utils/settingsFields';
import { Logger } from '../../utils/logger';

const PATH = 'video_player.zoom_to_fill';
// Rust remembers the last number per stream for the life of the process, which
// outlives a page reload, so each page load numbers a stream of its own.
const STREAM = `zoom-to-fill:${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;

let chosen: boolean | null = null;
let inFlight = 0;
let seq = 0;
const listeners = new Set<() => void>();
const notify = () => listeners.forEach((l) => l());

const storedNow = (): boolean => useAppStore.getState().settings?.video_player?.zoom_to_fill === true;

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** The choice this page holds while its writes are out, or null when the
 *  store leads. */
export function currentZoomChoice(): boolean | null {
  return chosen;
}

/** Whether Zoom to fill is on, as this page shows it. */
export function useZoomToFill(): boolean {
  const mine = useSyncExternalStore(subscribe, currentZoomChoice, currentZoomChoice);
  const stored = useAppStore((s) => s.settings?.video_player?.zoom_to_fill === true);
  return mine ?? stored;
}

/** Turn Zoom to fill on or off: shown at once, saved as one field. */
export async function writeZoomToFill(want: boolean): Promise<void> {
  const n = ++seq;
  inFlight += 1;
  chosen = want;
  notify();
  // Every whole-group writer (the other Player rows) spreads the store's copy,
  // so the copy carries the choice before any of them can run.
  const group = useAppStore.getState().settings?.video_player;
  const patched = !!group && typeof group === 'object';
  const before = group?.zoom_to_fill === true;
  const setStored = (value: boolean) =>
    useAppStore.setState((s) => ({
      settings: { ...s.settings, video_player: { ...s.settings.video_player, zoom_to_fill: value } },
    }));
  if (patched) setStored(want);
  try {
    await setSettingsFields({ [PATH]: want }, { stream: STREAM, n });
  } catch (err) {
    if (n === seq) {
      if (patched && storedNow() === want) setStored(before);
      Logger.warn('[ZoomToFill] save failed:', err);
      useAppStore.getState().addToast('Could not save that change.', 'error');
    }
  } finally {
    inFlight -= 1;
    if (inFlight === 0) chosen = null;
    notify();
  }
}
