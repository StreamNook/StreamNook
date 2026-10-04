import { useEffect, useMemo } from 'react';
import { useAppStore } from '../stores/AppStore';
import { usemultiNookStore } from '../stores/multiNookStore';
import { acquireChannel, releaseChannel } from '../stores/chatConnectionStore';
import { watchChannel, unwatchChannel } from '../stores/channelStateStore';
import { pinPair, startChatPin, useChatPinStore, type ChatChannel } from '../stores/chatPinStore';
import { streamProvider } from '../utils/streamProvider';
import { Logger } from '../utils/logger';

/** The stream the main window is watching, as a chat. */
export function useLiveChatChannel(): ChatChannel | null {
  const login = useAppStore((s) => s.currentStream?.user_login ?? null);
  const provider = useAppStore((s) => (s.currentStream ? streamProvider(s.currentStream) : null));
  return useMemo(() => (login && provider ? { provider, login } : null), [login, provider]);
}

/** The pin, the side on screen, and whether pinned and live are two chats. */
export function usePinPair() {
  const pin = useChatPinStore((s) => s.pin);
  const view = useChatPinStore((s) => s.view);
  const live = useLiveChatChannel();
  const multiNook = usemultiNookStore((s) => s.isMultiNookActive);
  return { pin, view, live, ...pinPair(pin, view, live, multiNook) };
}

/**
 * Keep the pinned chat and the live chat both joined while a pin is in play,
 * the way MultiChat keeps its hidden tabs: only the chat on screen renders, the
 * other keeps filling its own capped buffer, so a flip shows its backlog at
 * once and never re-JOINs. The live channel stays watched in channel state
 * too, so its minute chest check keeps running while the pinned side shows.
 *
 * Mount ONCE, in the main window.
 */
export function useChatPinHold(): void {
  useEffect(() => startChatPin(), []);
  const { pin, held, split, live } = usePinPair();
  const liveId = useAppStore((s) => s.currentStream?.user_id ?? '');
  const recording = useAppStore(
    (s) => s.currentMediaType === 'video' || s.currentMediaType === 'clip' || s.currentMediaType === 'offline_chat',
  );

  const pinLogin = held && pin ? pin.login : null;
  const pinProvider = pin?.provider ?? 'twitch';
  const pinId = pin?.channel_id || null;
  useEffect(() => {
    if (!pinLogin) return;
    void acquireChannel(pinLogin, pinId, pinProvider, { background: true }).catch((e: unknown) =>
      Logger.warn('[ChatPin] could not hold the pinned chat:', e),
    );
    return () => {
      void releaseChannel(pinLogin, pinProvider, { background: true }).catch(() => {});
    };
  }, [pinLogin, pinId, pinProvider]);

  // A VOD's chat is a replay, not a room to hold.
  const liveLogin = split && live && !recording ? live.login : null;
  const liveProvider = live?.provider ?? 'twitch';
  useEffect(() => {
    if (!liveLogin) return;
    void acquireChannel(liveLogin, liveId || null, liveProvider, { background: true }).catch((e: unknown) =>
      Logger.warn('[ChatPin] could not hold the live chat:', e),
    );
    const watched = liveProvider === 'twitch' && !!liveId;
    if (watched) void watchChannel(liveLogin, liveId);
    return () => {
      void releaseChannel(liveLogin, liveProvider, { background: true }).catch(() => {});
      if (watched) void unwatchChannel(liveLogin);
    };
  }, [liveLogin, liveId, liveProvider]);
}
