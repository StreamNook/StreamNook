// The return from the background, for chat. One owner decides it in order:
// first the saved chats join again (a list Android parked after a while away;
// Rust decides, from its own state), then every join any tab change started
// settles, and only then does a long absence rebuild the Twitch connection, so
// the rebuild cycles every room instead of finding none and leaving the joins
// on a socket that died in the background.
import { useChatTabsStore, settleJoins } from './chatTabsStore';
import { hardCycleChat } from './chatRecovery';
import { Logger } from '../../utils/logger';

// How long away before chat is assumed dead on return.
//
// Android freezes a backgrounded process, Twitch then drops the connection for
// a missed keepalive, and the reconnect that follows runs before the network is
// back, which ends the connection for good. A brief switch away to read a
// notification does none of that, and rebuilding costs a reconnect and a
// history load for every open room, so short absences are left alone and the
// staleness watchdog picks up the rare miss. Anything longer is worth rebuilding on sight rather
// than making someone stare at a dead room for the two minutes the watchdog
// needs to be sure.
export const CHAT_REBUILD_AFTER_HIDDEN_MS = 90_000;

/** How long the return waits for joins before it rebuilds anyway: a join on a
 *  dead socket can hang, and the rebuild is what replaces that socket. */
export const RESUME_JOIN_WAIT_MS = 5_000;

export async function resumeChats(awayForMs: number): Promise<void> {
  // One deadline over the join and the wait: a tab command that never
  // answers must not cost the rebuild that follows.
  const end = Date.now() + RESUME_JOIN_WAIT_MS;
  let timer: ReturnType<typeof setTimeout> | undefined;
  await Promise.race([
    useChatTabsStore.getState().connect({ resume: true }),
    new Promise<void>((done) => {
      timer = setTimeout(done, RESUME_JOIN_WAIT_MS);
    }),
  ]);
  clearTimeout(timer);
  await settleJoins(Math.max(0, end - Date.now()));
  if (awayForMs < CHAT_REBUILD_AFTER_HIDDEN_MS) return;
  try {
    await hardCycleChat(`away for ${Math.round(awayForMs / 1000)}s`);
  } catch (err) {
    Logger.warn('[Lifecycle] chat resume failed:', err);
  }
}
