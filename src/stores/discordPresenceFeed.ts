// The main window's feed to Discord presence: its MultiNook grid and its
// player, as raw facts (see discordPresenceFacts.ts for why this is TS).
// Installed once from App's boot, in the main window only; every other window
// loads these stores too and must never report them.

import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { useAppStore } from './AppStore';
import { activeChatSlot, usemultiNookStore } from './multiNookStore';
import {
  createSender,
  gridFacts,
  mediaFacts,
  type MediaFacts,
  type SenderDeps,
  type SlotFacts,
  type WriteResult,
} from './discordPresenceFacts';

/** To this window only: the page generation moved on (Rust cleared what this
 *  page sent before it); read it again and send both values. */
const PAGE_EVENT = 'discord-presence://page';

/** Start the feed. Resolves to its stop function; outside the main window it
 *  does nothing. */
export async function installDiscordPresenceFeed(): Promise<() => void> {
  const win = getCurrentWebviewWindow();
  if (win.label !== 'main') return () => {};

  let gen = 0;
  const refreshGen = async () => {
    gen = await invoke<number>('discord_presence_page');
  };
  const deps = (command: string, arg: 'slots' | 'media'): SenderDeps => ({
    send: (g, seq, value) => invoke<WriteResult>(command, { gen: g, seq, [arg]: value }),
    gen: () => gen,
    refreshGen,
    later: (fn, ms) => {
      const id = setTimeout(fn, ms);
      return () => clearTimeout(id);
    },
    now: () => Date.now(),
  });
  const grid = createSender<SlotFacts[]>(deps('discord_presence_set_grid', 'slots'));
  const media = createSender<MediaFacts>(deps('discord_presence_set_media', 'media'));

  // Only the fields the facts read: a volume drag changes the slots array
  // sixty times a second, and the facts it maps to stay equal, so the sender's
  // equality check stops it before anything is sent.
  const pushGrid = () => {
    const s = usemultiNookStore.getState();
    const focused = activeChatSlot(s.slots, s.activeChatChannelId);
    grid.push(gridFacts(s, focused?.id ?? null));
  };
  let lastMediaInput: unknown[] = [];
  const pushMedia = (force = false) => {
    const s = useAppStore.getState();
    const input = [s.currentMediaType, s.streamUrl, s.vodPlayback, s.originalMediaUrl, s.currentStream];
    if (!force && input.every((v, i) => v === lastMediaInput[i])) return;
    lastMediaInput = input;
    media.push(mediaFacts(s));
  };

  const unlistenPage = await win.listen(PAGE_EVENT, () => {
    void refreshGen()
      .then(() => {
        grid.reset();
        media.reset();
        pushGrid();
        pushMedia(true);
      })
      .catch(() => {});
  });
  // A failed read leaves generation 0: the first send then answers `stale`,
  // and the sender reads it again and resends. One hiccup never stops the feed.
  await refreshGen().catch(() => {});
  const unsubGrid = usemultiNookStore.subscribe(pushGrid);
  const unsubMedia = useAppStore.subscribe(() => pushMedia());
  pushGrid();
  pushMedia(true);
  return () => {
    unlistenPage();
    unsubGrid();
    unsubMedia();
  };
}
