// Secret accolades for the offline room's little moments: the DVD drift landing
// dead in a corner, a rare golden peepoSad, and being there when the stream
// finally starts. Like Restless, these are client-attested (only the viewer's
// own app saw them happen), granted with grantAccolade and announced once.
// A future lockdown of user_accolades must keep these ids writable, the same
// exception the 'insomniac' id needs.

import { useAppStore } from '../stores/AppStore';
import { grantAccolade } from '../services/supabaseService';
import { PERFECT_CORNER_ACCOLADE_ID, SHINY_ACCOLADE_ID, WORTH_THE_WAIT_ACCOLADE_ID } from './offlineAccoladeIds';

export { PERFECT_CORNER_ACCOLADE_ID, SHINY_ACCOLADE_ID, WORTH_THE_WAIT_ACCOLADE_ID };

const LABELS: Record<string, string> = {
  [PERFECT_CORNER_ACCOLADE_ID]: 'Perfect Corner',
  [SHINY_ACCOLADE_ID]: 'Shiny',
  [WORTH_THE_WAIT_ACCOLADE_ID]: 'Worth the Wait',
};

const STORAGE_KEY = 'sn:offline-accolades';

function announced(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(STORAGE_KEY) || '[]') as string[]);
  } catch {
    return new Set();
  }
}

function remember(ids: Set<string>): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify([...ids]));
  } catch {
    /* the grant itself is what persists; the toast may repeat on this device */
  }
}

/** Grant one of the offline room's secret accolades to the signed-in viewer
 *  and say so, once per device. The grant is idempotent server-side, so a
 *  repeat costs one ignored upsert. Signed out, nothing happens. */
export function earnOfflineAccolade(id: string): void {
  const { isAuthenticated, currentUser, addToast } = useAppStore.getState();
  if (!isAuthenticated || !currentUser?.user_id) return;
  const seen = announced();
  if (seen.has(id)) return;
  seen.add(id);
  remember(seen);
  void grantAccolade(currentUser.user_id, id);
  addToast(`Achievement unlocked: ${LABELS[id] ?? id}.`, 'success', undefined, { alwaysShow: true });
}
