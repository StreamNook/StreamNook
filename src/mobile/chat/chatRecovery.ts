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
// to authenticate. Afterwards `start_chat` still sees a handle that looks
// present, takes its "already running" path, and hands back the stale port, so
// every soft reconnect reports success while nothing arrives.
//
// `stop_chat` is the only call that clears that handle, which is the real
// reason restarting the stream was the one thing that ever worked. Doing it
// directly is the whole fix, and it covers the expired-token case for free
// because the fresh start reads the token again.
import { invoke } from '@tauri-apps/api/core';
import { acquireChannel, releaseChannel } from '../../stores/chatConnectionStore';
import { useChatTabsStore } from './chatTabsStore';
import { Logger } from '../../utils/logger';

// Automatic triggers can overlap: resuming the app and the staleness watchdog
// will often both fire for the same dead connection, and with several rooms
// open every pane sees the same failure. One cycle settles all of them, so the
// rest are noise. A manual reload passes `force` and is never throttled.
const COOLDOWN_MS = 60_000;

let cycling = false;
let lastCycleAt = 0;

/**
 * Tear the chat service down to nothing and bring it back with every open room.
 *
 * Returns true when a cycle actually ran. Callers use that only for logging;
 * a false return means the connection was already being rebuilt, was rebuilt
 * moments ago, or there was nothing open to rebuild.
 */
export async function hardCycleChat(reason: string, force = false): Promise<boolean> {
  if (cycling) return false;
  if (!force && Date.now() - lastCycleAt < COOLDOWN_MS) return false;

  const { tabs } = useChatTabsStore.getState();
  if (tabs.length === 0) return false;

  cycling = true;
  lastCycleAt = Date.now();
  // Deliberately warn, not debug: debug logging is off by default on device and
  // there is no way to turn it on from a phone.
  Logger.warn(`[ChatRecovery] cycling ${tabs.length} room(s): ${reason}`);

  try {
    // Release first, stop second, re-acquire third. The order is load-bearing.
    // Stopping first kills the bridge underneath a live socket, which fires the
    // connection store's own reconnect and races this one. Releasing lets the
    // store close that socket deliberately and forget its port, so the
    // re-acquire below takes the full cold bring-up path rather than a JOIN.
    //
    // Each tab holds exactly one reference, because this store is the only
    // thing on mobile that acquires, so one release per tab empties it exactly.
    for (const tab of tabs) {
      try {
        await releaseChannel(tab.channel);
      } catch (err) {
        Logger.warn(`[ChatRecovery] release ${tab.channel} failed:`, err);
      }
    }

    try {
      await invoke('stop_chat');
    } catch (err) {
      // Worth continuing anyway. If the service was already down the
      // re-acquire below still rebuilds it.
      Logger.warn('[ChatRecovery] stop_chat failed:', err);
    }

    for (const tab of tabs) {
      try {
        await acquireChannel(tab.channel, tab.channelId);
      } catch (err) {
        Logger.warn(`[ChatRecovery] re-acquire ${tab.channel} failed:`, err);
      }
    }

    // Panes keyed on the nonce rebuild their view of the room, matching what a
    // per-room reload used to do.
    useChatTabsStore.setState((s) => {
      const next = { ...s.reloadNonce };
      for (const tab of tabs) next[tab.channel] = (next[tab.channel] ?? 0) + 1;
      return { reloadNonce: next };
    });

    return true;
  } finally {
    cycling = false;
  }
}
