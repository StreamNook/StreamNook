import { invoke } from '@tauri-apps/api/core';
import { refreshChannelState } from '../stores/channelStateStore';
import { useAppStore } from '../stores/AppStore';
import { Logger } from './logger';
import { IS_MOBILE } from './platform';

// Twitch posts your sub in chat only sometimes: a resub (Prime included)
// appears only once you press Share, so the chat notice that used to be the
// app's only sign of your own sub may never come. These helpers let the app
// learn it straight from Twitch instead: a short watch while the subscribe
// page is open, and a re-read when that page closes.

/** Window event: your sub to `channel` may have changed, so read it again
 *  (relationship, badge, the "share your resub" offer). */
export const OWN_SUB_SETTLED_EVENT = 'own-sub-settled';

export interface OwnSubSettledDetail {
  /** The Twitch login, lowercased. */
  channel: string;
  /** The desktop subscribe panel that just closed, when that is the cause. */
  closedLabel?: string;
}

/** Twitch can take a moment to mint the resub share offer after checkout, so
 *  a settle is announced twice: at once and after this delay. */
export const SETTLE_AGAIN_MS = 4_000;

/** How often, and how many times, the open subscribe page is checked. */
export const SUB_POLL_MS = 5_000;
export const SUB_POLL_TRIES = 36;

function announce(detail: OwnSubSettledDetail): void {
  window.dispatchEvent(new CustomEvent<OwnSubSettledDetail>(OWN_SUB_SETTLED_EVENT, { detail }));
}

/** Your sub to `channel` may have changed: Rust re-reads the watched chat's
 *  relationship, and every surface showing the channel reads again.
 *  `closedLabel` names the desktop subscribe panel whose closing caused it. */
export function settleOwnSub(channel: string, closedLabel?: string): void {
  const login = channel.trim().toLowerCase();
  if (!login) return;
  if (closedLabel && active.get(login)?.label === closedLabel) stopOwnSubWatch(login);
  void refreshChannelState(login, 'relationship');
  announce(closedLabel ? { channel: login, closedLabel } : { channel: login });
  setTimeout(() => {
    void refreshChannelState(login, 'relationship');
    announce({ channel: login });
  }, SETTLE_AGAIN_MS);
}

/** Calls `onSettled` whenever your sub to `channel` may have changed. */
export function onOwnSubSettled(
  channel: string | null | undefined,
  onSettled: (detail: OwnSubSettledDetail) => void,
): () => void {
  const login = channel?.trim().toLowerCase();
  if (!login) return () => {};
  const handler = (event: Event) => {
    const detail = (event as CustomEvent<OwnSubSettledDetail>).detail;
    if (detail?.channel === login) onSettled(detail);
  };
  window.addEventListener(OWN_SUB_SETTLED_EVENT, handler);
  return () => window.removeEventListener(OWN_SUB_SETTLED_EVENT, handler);
}

/** The desktop subscribe panel's label is `subscribe-<login>-<ms>`; the login
 *  it was opened for, or null for any other panel. */
export function subscribePanelLogin(label: string): string | null {
  const m = /^subscribe-(.+)-\d+$/.exec(label);
  return m ? m[1].toLowerCase() : null;
}

/** Asks `isSubscribed` every `pollMs` until it answers true (then calls
 *  `onSubscribed` once) or `tries` run out. A failed read counts as a try.
 *  Returns a stop function. */
export function pollUntilSubscribed(
  isSubscribed: () => Promise<boolean>,
  onSubscribed: () => void,
  pollMs: number = SUB_POLL_MS,
  tries: number = SUB_POLL_TRIES,
): () => void {
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let left = tries;
  const tick = async () => {
    if (stopped) return;
    left -= 1;
    let yes = false;
    try {
      yes = await isSubscribed();
    } catch {
      yes = false;
    }
    if (stopped) return;
    if (yes) {
      stopped = true;
      onSubscribed();
      return;
    }
    if (left > 0) timer = setTimeout(() => void tick(), pollMs);
  };
  timer = setTimeout(() => void tick(), pollMs);
  return () => {
    stopped = true;
    if (timer !== undefined) clearTimeout(timer);
  };
}

/** One watch per channel, kept here rather than in a component so it outlives
 *  the menu or sheet that opened the subscribe page (the phone's composer
 *  menu closes as the page opens). */
const active = new Map<string, { label: string; stop: () => void }>();

/** Stops the watch on `channel`, if any (the chat notice got there first, or
 *  the page closed). */
export function stopOwnSubWatch(channel: string): void {
  const login = channel.trim().toLowerCase();
  active.get(login)?.stop();
  active.delete(login);
}

/** The subscribe page for `channel` just opened from "not subscribed": ask
 *  Twitch until the sub shows, then thank the viewer, put the page away and
 *  let every surface read again. `label` is the page `open_subscribe_window`
 *  returned. */
export function watchSubscribePage(channel: string, channelId: string, label: string): void {
  const login = channel.trim().toLowerCase();
  if (!login || !channelId) return;
  stopOwnSubWatch(login);
  const stop = pollUntilSubscribed(
    () =>
      invoke<{ subscribed: boolean }>('channel_relationship', { channelId, channelLogin: login }).then(
        (rel) => !!rel?.subscribed,
      ),
    () => {
      active.delete(login);
      useAppStore.getState().addToast('Subscription successful! Thank you for subscribing!', 'success');
      settleOwnSub(login);
      // The phone has one login overlay rather than labelled panels.
      const close = IS_MOBILE ? invoke('close_mobile_login') : invoke('close_login_overlay', { label });
      close.catch((e: unknown) => Logger.warn('[ownSubWatch] Failed to close subscribe page:', e));
    },
  );
  active.set(login, { label, stop });
}
