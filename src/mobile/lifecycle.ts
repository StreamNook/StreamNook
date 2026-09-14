// Background/foreground lifecycle for the mobile shell.
//
// A phone spends most of its time with the app backgrounded, and Android
// throttles WebView timers there rather than stopping them: Supabase realtime
// holds its socket, and hls.js keeps pulling segments for a stream nobody is
// watching. That is battery and data spent on nothing, and on resume the
// backlogged timers all fire at once.
//
// So: stand the background chatter down when hidden and bring it back on
// resume. The one deliberate exception is picture-in-picture, where the app is
// reported hidden but is still very much on screen.
//
// Going hidden is also the last reliable moment to get persistent state to
// disk: Android kills a backgrounded process outright, with none of the exit
// events the desktop flushes on, so the debounced settings / cache / log
// stores are flushed here.
//
// The badge-drop feed used to be paused here too. It is owned by Rust now
// (services::badge_feed) and keeps its socket up while backgrounded; a
// backgrounded pause for it is a follow-up on the Rust side.
//
// Playback is deliberately NOT touched. Backgrounding a stream to keep
// listening is a normal thing to do with a Twitch client, and pausing the video
// (or calling hls stopLoad, which stalls it once the buffer drains) would take
// that away. The sockets are pure background chatter with no user-visible
// value while hidden; the audio is not.
import { invoke } from '@tauri-apps/api/core';
import { refreshEntitlementRegistries } from '../services/supabaseService';
import { refreshFollowingIfStale } from './followRefresh';
import { isInPip, runNotifyCheckNow } from './nativeBridge';
import { setBackgrounded } from './backgroundGate';
import { Logger } from '../utils/logger';

let installed = false;

// When the app went into the background, so returning knows how long it was
// gone. See the chat rebuild in onVisible for why the duration matters.
let hiddenSince = 0;

// Foreground returns also kick one immediate notification poll (the periodic
// slot can be most of its interval away, and the user is looking at the phone
// right now), throttled so rapid app-switching and PiP exits do not each cost
// a network round trip.
const NOTIFY_KICK_MIN_GAP_MS = 5 * 60 * 1000;
let lastNotifyKick = 0;

// How long away before chat is assumed dead on return.
//
// Android freezes a backgrounded process, Twitch then drops the connection for
// a missed keepalive, and the reconnect that follows runs before the network is
// back, which ends the connection for good. A brief switch away to read a
// notification does none of that, and rebuilding costs a visible reload of
// every open room, so short absences are left alone and the staleness watchdog
// picks up the rare miss. Anything longer is worth rebuilding on sight rather
// than making someone stare at a dead room for the two minutes the watchdog
// needs to be sure.
const CHAT_REBUILD_AFTER_HIDDEN_MS = 90_000;

// Ask the activity directly. The dataset mirror is written by an async
// evaluateJavascript from onPictureInPictureModeChanged, and `visibilitychange`
// fires independently as the activity pauses, so the flag frequently has not
// landed yet when this is read: the gate lost the race and tore the badge
// socket down while the viewer was watching in PiP. A direct field read cannot.
// The mirror stays as the fallback for anything without the bridge.
function inPictureInPicture(): boolean {
  const native = isInPip();
  if (native !== null) return native;
  return document.documentElement.dataset.snPip === 'true';
}

async function onHidden(): Promise<void> {
  if (inPictureInPicture()) return;
  hiddenSince = Date.now();
  // The polls read this and skip their tick. Set before the await so a poll
  // firing in the same turn already sees it.
  setBackgrounded(true);
  // Fire-and-forget on purpose: the IPC message is posted synchronously, so
  // it reaches Rust even if the WebView is frozen a moment later, and every
  // flush is a no-op when nothing is dirty.
  void invoke('flush_persistent_stores').catch((err) => {
    Logger.warn('[Lifecycle] flush on hide failed:', err);
  });
}

async function onVisible(): Promise<void> {
  setBackgrounded(false);
  const now = Date.now();
  if (now - lastNotifyKick >= NOTIFY_KICK_MIN_GAP_MS) {
    lastNotifyKick = now;
    runNotifyCheckNow();
  }
  // Entitlements may have changed while away (a purchase completed in a
  // browser, a badge granted). The registries themselves self-heal, this just
  // pulls once on return.
  try {
    refreshEntitlementRegistries();
  } catch {
    /* not configured */
  }
  // Who is live moves while the app is away, and the activity survives
  // backgrounding, so nothing else would ever invalidate the list. Throttled
  // internally; see followRefresh.ts.
  void refreshFollowingIfStale();

  // Chat is the one thing that does not survive a long absence, and it cannot
  // report that itself: the frontend reads a local bridge that stays up whether
  // or not the connection behind it is alive, so a dead room looks connected
  // forever. Rebuild it rather than trust it. Re-joining refetches recent
  // history, so the room comes back populated instead of blank.
  const awayFor = hiddenSince ? Date.now() - hiddenSince : 0;
  hiddenSince = 0;
  if (awayFor >= CHAT_REBUILD_AFTER_HIDDEN_MS) {
    try {
      const { hardCycleChat } = await import('./chat/chatRecovery');
      await hardCycleChat(`away for ${Math.round(awayFor / 1000)}s`);
    } catch (err) {
      Logger.warn('[Lifecycle] chat resume failed:', err);
    }
  }
}

/** Idempotent; returns a teardown. */
export function installLifecycle(): () => void {
  if (installed) return () => {};
  installed = true;

  const handler = () => {
    if (document.visibilityState === 'hidden') void onHidden();
    else void onVisible();
  };
  document.addEventListener('visibilitychange', handler);
  return () => {
    document.removeEventListener('visibilitychange', handler);
    installed = false;
  };
}
