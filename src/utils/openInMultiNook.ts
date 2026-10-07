import type React from 'react';
import type { TwitchStream } from '../types';
import { canGridProvider, gridRefusal } from '../types/providers';
import { useAppStore } from '../stores/AppStore';
import { usemultiNookStore } from '../stores/multiNookStore';
import { makeKey } from './providerKey';
import { streamProvider } from './streamProvider';

// The grid is owned by the frontend (multiNookStore; Rust keeps only the saved
// copy), so opening a stream into it runs against that store, the same way the
// player's own "Add to MultiNook" button does.

/**
 * Open a stream in MultiNook and bring the grid up.
 *
 * With the grid closed, a solo stream that is playing comes along as a tile
 * beside the new one, handing over the way the player's "Add to MultiNook" does:
 * the grid comes up first and the solo player closes keeping its backend. A solo
 * stream the grid cannot hold is closed outright, since the grid draws over the
 * player and would leave it playing unseen. With the grid open, the stream joins
 * it. Either way chat moves to the opened stream.
 */
export async function openStreamInMultiNook(stream: TwitchStream): Promise<void> {
  const provider = streamProvider(stream);
  const refusal = gridRefusal(provider);
  if (refusal) {
    useAppStore.getState().addToast(refusal, 'info');
    return;
  }

  const key = makeKey(provider, stream.user_login);
  const inGrid = (k: string) =>
    usemultiNookStore.getState().slots.some((s) => makeKey(s.provider ?? 'twitch', s.channelLogin) === k);

  const app = useAppStore.getState();
  const gridOpen = usemultiNookStore.getState().isMultiNookActive;
  const solo = !gridOpen && app.streamUrl ? app.currentStream : null;
  const soloProvider = solo ? streamProvider(solo) : null;
  const carrySolo = !!solo?.user_login && !!soloProvider && canGridProvider(soloProvider);

  if (solo && !carrySolo) await app.exitStream();

  // Adding before entering keeps entry from reloading the saved lineup over
  // these tiles (it only restores into an empty grid).
  if (carrySolo && solo && soloProvider) {
    const soloKey = makeKey(soloProvider, solo.user_login);
    if (!inGrid(soloKey)) await usemultiNookStore.getState().addSlot(solo.user_login, soloProvider);
  }
  if (!inGrid(key)) await usemultiNookStore.getState().addSlot(stream.user_login, provider);
  // addSlot reports its own refusal (a full grid); nothing to open then.
  if (!inGrid(key)) return;

  usemultiNookStore.getState().setActiveChatChannelId(key);
  if (!usemultiNookStore.getState().isMultiNookActive) {
    await usemultiNookStore.getState().toggleMultiNook();
  }
  if (carrySolo) await useAppStore.getState().exitStream({ preserveBackend: true });
}

function middleClickOpensMultiNook(): boolean {
  return useAppStore.getState().settings?.video_player?.middle_click_multinook ?? true;
}

/**
 * Mouse handlers for a stream card or row: a middle-click opens the stream in
 * MultiNook. The mousedown half stops the autoscroll ring before it starts;
 * preventing it there does not cancel the auxclick that follows.
 */
export function streamMiddleClickHandlers(stream: TwitchStream) {
  return {
    onMouseDown: (e: React.MouseEvent) => {
      if (e.button !== 1 || !middleClickOpensMultiNook()) return;
      e.preventDefault();
    },
    onAuxClick: (e: React.MouseEvent) => {
      if (e.button !== 1 || !middleClickOpensMultiNook()) return;
      e.preventDefault();
      e.stopPropagation();
      void openStreamInMultiNook(stream);
    },
  };
}
