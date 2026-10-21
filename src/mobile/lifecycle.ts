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
// Playback is deliberately NOT touched here. Pausing on surface loss is
// lockScreenAudio's job and follows Background play: on, the audio keeps going
// behind the media notification; off, locking or leaving pauses the stream.
// The sockets are pure background chatter with no user-visible value while
// hidden; the audio, when wanted, is not.
//
// Rust learns the same thing (set_app_foreground; PiP counts as seen), so its
// pollers rest while the app is away. Going to the background also flushes
// every debounced store there.
import { invoke } from '@tauri-apps/api/core';
import { refreshEntitlementRegistries, resumePresence, suspendPresence } from '../services/supabaseService';
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
// notification does none of that, and rebuilding costs a reconnect and a
// history load for every open room, so short absences are left alone and the
// staleness watchdog picks up the rare miss. Anything longer is worth rebuilding on sight rather
// than making someone stare at a dead room for the two minutes the watchdog
// needs to be sure.
const CHAT_REBUILD_AFTER_HIDDEN_MS = 90_000;

// One run per transition. Two signals can report the same one: a lock with
// audio playing raises sn:screen-off while the page may never turn hidden
// (MainActivity resumes the WebView while the media service runs), and a lock
// without it raises both. Seeded by installLifecycle. pipAway marks a hide
// that only went into PiP, so the return from it still runs onVisible.
let hiddenNow = false;
let pipAway = false;

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

// Tell the core whether the app is seen (PiP counts as seen). The one presence
// signal its pollers read; going to the background also flushes the stores.
function reportForeground(foreground: boolean): void {
  void invoke('set_app_foreground', { foreground }).catch((err) => {
    Logger.debug('[Lifecycle] set_app_foreground failed:', err);
  });
}

async function onHidden(): Promise<void> {
  if (inPictureInPicture()) {
    pipAway = true;
    return;
  }
  if (hiddenNow) return;
  hiddenNow = true;
  hiddenSince = Date.now();
  // The polls read this and skip their tick. Set before the await so a poll
  // firing in the same turn already sees it.
  setBackgrounded(true);
  // Fire-and-forget on purpose: the IPC message is posted synchronously, so
  // it reaches Rust even if the WebView is frozen a moment later. Rust flushes
  // every store as part of it (each a no-op when nothing is dirty).
  reportForeground(false);
  suspendPresence();
}

async function onVisible(): Promise<void> {
  if (!hiddenNow && !pipAway) return;
  hiddenNow = false;
  pipAway = false;
  setBackgrounded(false);
  reportForeground(true);
  void resumePresence();
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
  // Closing the PiP window backgrounds the app with no visibilitychange (the
  // page was already hidden behind the window).
  const onPipClosed = () => {
    if (document.visibilityState === 'hidden') void onHidden();
  };
  // A lock with audio playing may never turn the page hidden (see hiddenNow),
  // and locked with audio is the main background case, so the screen itself
  // counts. PiP with the screen off is still PiP.
  const onScreenOff = () => {
    if (isInPip() !== true) void onHidden();
  };
  const onScreenOn = () => {
    if (document.visibilityState === 'visible') void onVisible();
  };
  hiddenNow = document.visibilityState === 'hidden';
  document.addEventListener('visibilitychange', handler);
  window.addEventListener('sn:pip-closed', onPipClosed);
  window.addEventListener('sn:screen-off', onScreenOff);
  window.addEventListener('sn:screen-on', onScreenOn);
  reportForeground(document.visibilityState === 'visible');
  return () => {
    document.removeEventListener('visibilitychange', handler);
    window.removeEventListener('sn:pip-closed', onPipClosed);
    window.removeEventListener('sn:screen-off', onScreenOff);
    window.removeEventListener('sn:screen-on', onScreenOn);
    installed = false;
  };
}
