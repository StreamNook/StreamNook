// Recovery for a Twitch IRC connection that has died for good.
//
// The IRC socket lives in Rust; the frontend reads it through a local WebSocket
// bridge. Those two are independent, and that is what hides the failure: the
// bridge stays up and the UI keeps reporting a healthy chat long after the
// upstream connection is gone. It is also why reloading a room never helped,
// since a reload only PARTs and re-JOINs on a connection that no longer exists.
//
// Rust's IRC supervisor retries forever and reads the token again on every
// attempt, but a socket that died while the phone slept can still sit there
// until the read deadline notices it, and the retries back off up to a minute.
// A rebuild replaces the connection now instead.
//
// The rebuild itself is Rust's (`rebuild_chat`, services/chat_recovery.rs): it
// stops the Twitch side and brings it back with every room this window holds,
// one cycle at a time and automatic ones at most once a minute, the same core
// the iPhone uses. The page keeps every room and its rows through it, so the
// chat on screen stays put while the connection behind it is replaced.
import { rebuildTwitchChat } from '../../stores/chatConnectionStore';
import { Logger } from '../../utils/logger';

/**
 * Rebuild the chat connection under every open room.
 *
 * Returns true when a cycle actually ran. Callers use that only for logging;
 * a false return means the connection was already being rebuilt, was rebuilt
 * moments ago, or there was nothing open to rebuild. A manual reload passes
 * `force` and is never throttled.
 */
export async function hardCycleChat(reason: string, force = false): Promise<boolean> {
  const ran = await rebuildTwitchChat(reason, force);
  // Deliberately warn, not debug: debug logging is off by default on device and
  // there is no way to turn it on from a phone.
  if (ran) Logger.warn(`[ChatRecovery] rebuilt chat: ${reason}`);
  return ran;
}
