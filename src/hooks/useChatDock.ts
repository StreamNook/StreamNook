import { useEffect, useMemo, useRef } from 'react';
import { useAppStore } from '../stores/AppStore';
import { usemultiNookStore } from '../stores/multiNookStore';
import { acquireChannel, releaseChannel } from '../stores/chatConnectionStore';
import { watchChannel, unwatchChannel } from '../stores/channelStateStore';
import { chatKey, dockView, startChatDock, useChatDockStore, type ChatChannel, type DockedChat } from '../stores/chatDockStore';
import { streamProvider } from '../utils/streamProvider';
import { Logger } from '../utils/logger';

/** The stream the main window is watching, as a chat. */
export function useLiveChatChannel(): ChatChannel | null {
  const login = useAppStore((s) => s.currentStream?.user_login ?? null);
  const provider = useAppStore((s) => (s.currentStream ? streamProvider(s.currentStream) : null));
  return useMemo(() => (login && provider ? { provider, login } : null), [login, provider]);
}

/** The dock, the chat on screen, and the watched stream as a chat. */
export function useDockView() {
  const chats = useChatDockStore((s) => s.chats);
  const active = useChatDockStore((s) => s.active);
  const live = useLiveChatChannel();
  const multiNook = usemultiNookStore((s) => s.isMultiNookActive);
  const view = useMemo(() => dockView(chats, active, live, multiNook), [chats, active, live, multiNook]);
  return { chats, active, live, multiNook, ...view };
}

/**
 * How narrow the capsule's name may get before the header compacts instead:
 * 3.5rem for a long name, and the name's own width for a short one. A fixed
 * 3.5rem padded "xQc" out to a long name's size.
 */
export function nameFloor(name: string): string {
  return `min(3.5rem, ${name.length + 0.5}ch)`;
}

/** The name the capsule shows: the docked chat on screen, or the stream's. */
export function useCapsuleName(): string {
  const { shown } = useDockView();
  const stream = useAppStore((s) => s.currentStream);
  return shown ? shown.display_name || shown.login : stream?.user_name || stream?.user_login || 'Stream chat';
}

/**
 * The capsule's floor, for the header row to hold it at: the name's floor plus
 * the picture, the count, the mark and the caret. Squeezed below it, the
 * capsule would spill over its neighbours instead of the row reporting
 * overflow, so the header would never compact.
 */
export function useDockSwitcherFloor(): string {
  return `calc(${nameFloor(useCapsuleName())} + 4.75rem)`;
}

/** A docked Twitch chat's row reads its live line from Rust's channel state. */
const watchesLive = (c: DockedChat) => c.provider === 'twitch' && !!c.channel_id;

/**
 * Keep every docked chat joined while the dock is in play, the way MultiChat
 * keeps its hidden tabs: only the chat on screen renders, every other one is
 * held back at the bridge (Rust keeps its rows and counts its mentions and new
 * messages), so a switch shows its backlog at once and never re-JOINs. Each
 * docked Twitch chat is also watched live-only in channel state for its row
 * in the list (one batched poll for all of them).
 *
 * While a docked chat is on screen, the watched stream's chat is held the same
 * way and stays fully watched, so its bonus-chest check keeps running.
 *
 * Mount ONCE, in the main window.
 */
export function useChatDockHold(): void {
  useEffect(() => startChatDock(), []);
  const { held, shown, others, live } = useDockView();

  // Diff the held set against what this hook holds, so adding the twentieth
  // chat touches one channel, not twenty.
  const holding = useRef(new Map<string, DockedChat>());
  const wanted = held ? others : [];
  const wantedSig = wanted.map((c) => `${chatKey(c)}|${c.channel_id}`).join('\n');
  useEffect(() => {
    const next = new Map(wanted.map((c) => [chatKey(c), c] as const));
    const current = holding.current;
    for (const [key, c] of current) {
      if (next.has(key)) continue;
      current.delete(key);
      void releaseChannel(c.login, c.provider, { background: true }).catch(() => {});
      if (watchesLive(c)) void unwatchChannel(c.login, true);
    }
    for (const [key, c] of next) {
      if (current.has(key)) continue;
      current.set(key, c);
      void acquireChannel(c.login, c.channel_id || null, c.provider, { background: true }).catch((e: unknown) =>
        Logger.warn('[ChatDock] could not hold a docked chat:', e),
      );
      if (watchesLive(c)) void watchChannel(c.login, c.channel_id, true);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the signature
  }, [wantedSig]);
  useEffect(() => {
    const current = holding.current;
    return () => {
      for (const c of current.values()) {
        void releaseChannel(c.login, c.provider, { background: true }).catch(() => {});
        if (watchesLive(c)) void unwatchChannel(c.login, true);
      }
      current.clear();
    };
  }, []);

  const liveId = useAppStore((s) => s.currentStream?.user_id ?? '');
  const recording = useAppStore(
    (s) => s.currentMediaType === 'video' || s.currentMediaType === 'clip' || s.currentMediaType === 'offline_chat',
  );
  // A VOD's chat is a replay, not a room to hold.
  const liveProvider = live?.provider ?? 'twitch';
  // The watched stream's chat is held for as long as the dock is in play, not
  // only while a docked chat covers it. React runs every effect cleanup before
  // any new effect, so a hold that followed the chat on screen dropped the
  // room's last reference for an instant on each switch: Twitch PARTs at once,
  // which wiped the room's emote table, rejoined, and re-read its history.
  const heldLogin = held && live && !recording ? live.login : null;
  useEffect(() => {
    if (!heldLogin) return;
    void acquireChannel(heldLogin, liveId || null, liveProvider, { background: true }).catch((e: unknown) =>
      Logger.warn('[ChatDock] could not hold the live chat:', e),
    );
    return () => {
      void releaseChannel(heldLogin, liveProvider, { background: true }).catch(() => {});
    };
  }, [heldLogin, liveId, liveProvider]);
  // Its full channel-state watch (the bonus-chest check) is this hook's job
  // only while a docked chat is on screen; otherwise the stream's own chat
  // watches it.
  const watchedLogin = shown && live && !recording && liveProvider === 'twitch' && liveId ? live.login : null;
  useEffect(() => {
    if (!watchedLogin) return;
    void watchChannel(watchedLogin, liveId);
    return () => {
      void unwatchChannel(watchedLogin);
    };
  }, [watchedLogin, liveId]);
}
