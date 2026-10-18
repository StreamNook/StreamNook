// Recovery for a Twitch IRC connection that has died for good.
//
// The IRC socket lives in Rust; the frontend reads it through a local WebSocket
// bridge. Those two are independent, and that is what hides the failure: the
// bridge stays up and the UI keeps reporting a healthy chat long after the
// upstream connection is gone. It is also why reloading a room never helped,
// since a reload only PARTs and re-JOINs on a connection that no longer exists.
//
// Rust's IRC task can end permanently in two ways and both leave the same
// state. A failed connect attempt ends it outright, which is what a phone hits
// when the process thaws before the network is back or during a Wi-Fi to mobile
// handoff. An expired token ends it too: the token is read once when the task
// starts, so every later reconnect re-sends that same one and eventually fails
// to authenticate. Only a stop clears that state, and the fresh start reads the
// token again.
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
