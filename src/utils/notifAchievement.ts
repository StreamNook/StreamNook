// The "Restless" hidden achievement: earned by clicking the Notifications
// "Test" button a stubborn number of times (some hands just can't leave a
// button alone). The running count is kept LOCAL (per device) because progress
// toward an easter egg does not need to follow the user around; only the EARNED
// state is persisted (Rust saves it through grant_own_accolade, the same way
// seasonal / cake-day accolades persist), so the medallion and the Midnight
// Atmosphere it unlocks do follow the account.

// Stable id stored in user_accolades.accolade_id and matched by the secret
// accolade (ProfileOverview) and the Midnight Atmosphere's unlock gate. The
// stored value stays 'insomniac' (the original codename) so any already-earned
// grants remain valid; the player-facing label is "Restless".
export const RESTLESS_ACCOLADE_ID = 'insomniac';

// How many Test clicks earns it. "50+" floor.
export const NOTIF_CLICK_THRESHOLD = 50;

const STORAGE_KEY = 'sn:test-notif-clicks';

/** Increment the local Test-click counter and return the new total. Safe if
 *  localStorage is unavailable (returns 0 so callers never crash). */
export const bumpTestNotificationClicks = (): number => {
  try {
    const next = (parseInt(localStorage.getItem(STORAGE_KEY) || '0', 10) || 0) + 1;
    localStorage.setItem(STORAGE_KEY, String(next));
    return next;
  } catch {
    return 0;
  }
};
