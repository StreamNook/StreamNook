// The accolades only the viewer's own app can attest: Restless (fifty clicks on
// the Notifications Test button) and the offline room's three (the DVD logo
// landing in a corner, a rare golden frog, being there when the stream starts).
// Rust saves them (`grant_own_accolade`, for the signed-in account only); this
// decides when to ask and when to say so. Pure apart from what it is handed, so
// the rules are tested without a screen:
//  - an id already announced for this account is not asked again;
//  - while a request for an id is running, a second one is not started;
//  - the announce waits until Rust has said yes, and a failed request leaves it
//    undone, so the next moment asks again;
//  - the toast says so only when Rust newly saved the accolade: one it already
//    held was earned (and told) somewhere else, so it joins the announced set
//    without a word.
import { NOTIF_CLICK_THRESHOLD, RESTLESS_ACCOLADE_ID } from './notifAchievement';
import { PERFECT_CORNER_ACCOLADE_ID, SHINY_ACCOLADE_ID, WORTH_THE_WAIT_ACCOLADE_ID } from './offlineAccoladeIds';

/** Exactly the ids Rust's `grant_own_accolade` allows, with the line each one announces. */
export const CLIENT_ACCOLADE_TOASTS: Readonly<Record<string, string>> = {
  [RESTLESS_ACCOLADE_ID]: 'Achievement unlocked: Restless. The Midnight Atmosphere is yours.',
  [PERFECT_CORNER_ACCOLADE_ID]: 'Achievement unlocked: Perfect Corner.',
  [SHINY_ACCOLADE_ID]: 'Achievement unlocked: Shiny.',
  [WORTH_THE_WAIT_ACCOLADE_ID]: 'Achievement unlocked: Worth the Wait.',
};

/** Restless is earned from the threshold click on, so a missed grant is retried on every click after it. */
export const restlessReached = (clicks: number): boolean => clicks >= NOTIF_CLICK_THRESHOLD;

/** The ids announced on this device, per account. */
export interface AnnouncedStore {
  read(userId: string): ReadonlySet<string>;
  add(userId: string, id: string): void;
}

const announcedKey = (userId: string) => `sn:client-accolades:${userId}`;

/** The device-wide set the offline room kept before announced ids were per account. */
const LEGACY_OFFLINE_KEY = 'sn:offline-accolades';

const idsIn = (raw: string | null | undefined): string[] => {
  try {
    const parsed: unknown = JSON.parse(raw || '[]');
    return Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === 'string') : [];
  } catch {
    return [];
  }
};

/** An announced set kept in a Storage (localStorage); a missing or broken one reads as empty.
 *  The first account to read after the old device-wide key exists takes its ids
 *  into its own set, and the old key is removed. */
export function storageAnnouncedStore(
  storage: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> | null,
): AnnouncedStore {
  return {
    read(userId) {
      try {
        const own = new Set(idsIn(storage?.getItem(announcedKey(userId))));
        const legacy = storage?.getItem(LEGACY_OFFLINE_KEY) ?? null;
        if (legacy === null) return own;
        idsIn(legacy).forEach((id) => own.add(id));
        storage?.setItem(announcedKey(userId), JSON.stringify([...own]));
        storage?.removeItem(LEGACY_OFFLINE_KEY);
        return own;
      } catch {
        return new Set();
      }
    },
    add(userId, id) {
      try {
        const next = new Set(this.read(userId)).add(id);
        storage?.setItem(announcedKey(userId), JSON.stringify([...next]));
      } catch {
        /* the accolade itself is what persists; the toast may repeat on this device */
      }
    },
  };
}

export interface EarnDeps {
  /** The signed-in account's id, or null when signed out. */
  userId: () => string | null;
  /** Asks Rust to save the accolade. Resolves when Rust answered Ok (true: newly saved, false: already held, so already told); rejects when it did not. */
  grant: (id: string) => Promise<boolean>;
  announced: AnnouncedStore;
  toast: (text: string) => void;
}

/** Builds the function a moment calls: `earn('perfect_corner')`. One earner keeps one in-flight set. */
export function createEarner(deps: EarnDeps): (id: string) => Promise<void> {
  const inFlight = new Set<string>();
  return async (id) => {
    const text = CLIENT_ACCOLADE_TOASTS[id];
    const userId = deps.userId();
    if (!text || !userId) return;
    if (deps.announced.read(userId).has(id)) return;
    const key = `${userId}:${id}`;
    if (inFlight.has(key)) return;
    inFlight.add(key);
    try {
      const inserted = await deps.grant(id);
      deps.announced.add(userId, id);
      // The account may have changed while Rust was answering; say it only to the one it was for.
      if (inserted && deps.userId() === userId) deps.toast(text);
    } catch {
      /* not announced, so the next moment asks again */
    } finally {
      inFlight.delete(key);
    }
  };
}
