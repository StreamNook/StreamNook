// Secret accolades for the offline room's little moments: the DVD drift landing
// dead in a corner, a rare golden peepoSad, and being there when the stream
// finally starts. Like Restless, these are client-attested (only the viewer's
// own app saw them happen): Rust saves them through grant_own_accolade, and the
// viewer is told once per account on this device (utils/clientAccolades.ts).

import { earnClientAccolade } from './earnAccolade';
import { PERFECT_CORNER_ACCOLADE_ID, SHINY_ACCOLADE_ID, WORTH_THE_WAIT_ACCOLADE_ID } from './offlineAccoladeIds';

export { PERFECT_CORNER_ACCOLADE_ID, SHINY_ACCOLADE_ID, WORTH_THE_WAIT_ACCOLADE_ID };

const OFFLINE_IDS: ReadonlySet<string> = new Set([
  PERFECT_CORNER_ACCOLADE_ID,
  SHINY_ACCOLADE_ID,
  WORTH_THE_WAIT_ACCOLADE_ID,
]);

/** One of the offline room's secret accolades happened. Signed out, or an id that is
 *  not one of the three, nothing happens. */
export function earnOfflineAccolade(id: string): void {
  if (!OFFLINE_IDS.has(id)) return;
  void earnClientAccolade(id);
}
