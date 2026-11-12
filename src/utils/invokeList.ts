// A fixed list read from Rust once per run (alert types, notice types), that
// never settles on nothing while Rust can still answer.
//
// Right after an update or a reboot the first call can fail, or sit
// unanswered while the app comes up. A page that took that as an empty list
// drew its section blank for the whole run (zoox's empty "Notify me about").
// So each try gets a time limit, a failed or empty answer is tried again with
// a growing gap, and only after every try does the caller get an empty list,
// with the next caller starting over.

import { invoke } from '@tauri-apps/api/core';
import { Logger } from './logger';

/** How long one call may take before the next try starts beside it. */
const TRY_MS = 4000;
/** Gaps between tries: about a minute in all. */
const GAPS_MS = [300, 700, 1500, 3000, 5000, 8000, 12000, 15000];

const wait = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** Calls `command` until it answers with a non-empty list, or every try is spent. */
export async function invokeList<T>(command: string): Promise<T[]> {
  for (let attempt = 0; attempt <= GAPS_MS.length; attempt++) {
    try {
      const list = await Promise.race([
        invoke<T[]>(command),
        wait(TRY_MS).then(() => {
          throw new Error(`no answer in ${TRY_MS} ms`);
        }),
      ]);
      if (Array.isArray(list) && list.length > 0) {
        Logger.info(`[invokeList] ${command}: ${list.length} on try ${attempt + 1}`);
        return list;
      }
      Logger.warn(`[invokeList] ${command} answered an empty list (try ${attempt + 1})`);
    } catch (err) {
      Logger.warn(`[invokeList] ${command} failed (try ${attempt + 1}):`, err);
    }
    if (attempt < GAPS_MS.length) await wait(GAPS_MS[attempt]);
  }
  return [];
}
