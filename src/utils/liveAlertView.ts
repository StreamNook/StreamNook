// What a live-alert bell shows for one channel, as a pure reducer.
//
// Every ask and every press gets a number from one counter. An answer older
// than the newest press or answer is dropped, so a slow first read can never
// put back the state a press just changed. A press shows its new state at
// once and the answer replaces it; a failed press goes back to the last
// answer Rust gave.

export interface LiveAlertView {
  /** The channel the view describes (`provider|userId|userLogin`). */
  key: string;
  /** The newest request whose result the view reflects or waits for. */
  latest: number;
  /** Rust's last answer for `key`: true on, false off, null no identity or not known yet. */
  confirmed: boolean | null;
  /** What the bell shows: the pressed state while a press is in flight, else `confirmed`. */
  shown: boolean | null;
  busy: boolean;
}

export type LiveAlertEvent =
  | { kind: 'press'; key: string; id: number; on: boolean }
  | { kind: 'answer'; key: string; id: number; on: boolean | null }
  | { kind: 'failed'; key: string; id: number };

export const LIVE_ALERT_INITIAL: LiveAlertView = {
  key: '',
  latest: 0,
  confirmed: null,
  shown: null,
  busy: false,
};

export function liveAlertReduce(view: LiveAlertView, event: LiveAlertEvent): LiveAlertView {
  switch (event.kind) {
    case 'press': {
      const sameChannel = view.key === event.key;
      return {
        key: event.key,
        latest: event.id,
        confirmed: sameChannel ? view.confirmed : null,
        shown: event.on,
        busy: true,
      };
    }
    case 'answer':
      if (event.id < view.latest) return view;
      return { key: event.key, latest: event.id, confirmed: event.on, shown: event.on, busy: false };
    case 'failed':
      if (event.id !== view.latest || event.key !== view.key) return view;
      return { ...view, shown: view.confirmed, busy: false };
  }
}

/** What the bell shows for `key`: nothing until an answer for that channel. */
export function shownFor(view: LiveAlertView, key: string): boolean | null {
  return view.key === key ? view.shown : null;
}
